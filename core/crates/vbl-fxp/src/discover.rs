//! Descoberta multicast FXP v1.1 — beacon `FXPD` em UDP
//! (docs/FXP-SCHEMA-v1.md §4.9).
//!
//! Datagrama único, **sem ack** (UDP é lossy; liveness fica no heartbeat/TCP).
//! O anúncio **não carrega dado de sensor** — apenas identidade do servidor,
//! porta TCP e a impressão digital do registro. O canal de dados segue o
//! fluxo `AUTH → CAPS → HELLO` sobre Unix/TCP (§4.5/§4.6).
//!
//! Opt-in: só há tráfego quando um dispositivo declara `endpoint =
//! discover:<identificador>` (lado consumidor) ou o servidor anuncia
//! ([`Announcer`], lado `fxpd`). Rede sem multicast ⇒ erro honesto de
//! descoberta — o dispositivo fica registrado porém inacessível (§4.7 do
//! FORMAL), nunca erro de construção do barramento.

use sha2::{Digest, Sha256};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Magic do beacon: `"FXPD"`.
pub const MAGIC: [u8; 4] = *b"FXPD";
/// Versão do beacon.
pub const BEACON_VERSION: u8 = 1;
/// Grupo multicast padrão (IPv4 site-local) e porta.
pub const DEFAULT_GROUP: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::new(239, 255, 70, 80)), 7080);
/// Intervalo padrão de anúncio.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(2);
/// TTL multicast: 1 = não sai da sub-rede (site-local, §4.9).
pub const MULTICAST_TTL: u32 = 1;

/// Falha de descoberta — distinta de falha de transporte/schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    /// Socket multicast indisponível (rede/container sem suporte).
    MulticastIndisponivel(String),
    /// Datagrama malformado (magic/versão/tamanho) — ignorable no listener.
    BeaconInvalido,
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryError::MulticastIndisponivel(m) => {
                write!(f, "descoberta multicast indisponível: {m}")
            }
            DiscoveryError::BeaconInvalido => write!(f, "beacon FXPD malformado"),
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// Beacon decodificado (§4.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beacon {
    pub tcp_port: u16,
    pub identifier: String,
    pub registry_hash: u32,
}

/// Impressão digital do registro: primeiros 4 bytes (LE) de SHA-256 sobre os
/// nomes canônicos ordenados e concatenados com `\n` — informativa (probe).
pub fn registry_hash(names: &[String]) -> u32 {
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort();
    let mut h = Sha256::new();
    for (i, n) in sorted.iter().enumerate() {
        if i > 0 {
            h.update(b"\n");
        }
        h.update(n.as_bytes());
    }
    let out = h.finalize();
    u32::from_le_bytes([out[0], out[1], out[2], out[3]])
}

/// Serializa o beacon (datagrama único, sem length-prefix).
pub fn encode_beacon(identifier: &str, tcp_port: u16, registry_hash: u32) -> Vec<u8> {
    let id = identifier.as_bytes();
    let mut out = Vec::with_capacity(4 + 1 + 2 + 1 + id.len() + 4);
    out.extend_from_slice(&MAGIC);
    out.push(BEACON_VERSION);
    out.extend_from_slice(&tcp_port.to_le_bytes());
    out.push(id.len() as u8);
    out.extend_from_slice(id);
    out.extend_from_slice(&registry_hash.to_le_bytes());
    out
}

/// Decodifica um datagrama beacon; qualquer violação ⇒ `BeaconInvalido`
/// (o listener ignora e segue — UDP é lossy, nunca é erro fatal).
pub fn decode_beacon(buf: &[u8]) -> Result<Beacon, DiscoveryError> {
    if buf.len() < 4 + 1 + 2 + 1 || buf[0..4] != MAGIC || buf[4] != BEACON_VERSION {
        return Err(DiscoveryError::BeaconInvalido);
    }
    let tcp_port = u16::from_le_bytes([buf[5], buf[6]]);
    let id_len = buf[7] as usize;
    if buf.len() < 8 + id_len + 4 {
        return Err(DiscoveryError::BeaconInvalido);
    }
    let identifier =
        String::from_utf8(buf[8..8 + id_len].to_vec()).map_err(|_| DiscoveryError::BeaconInvalido)?;
    let off = 8 + id_len;
    let registry_hash =
        u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
    Ok(Beacon { tcp_port, identifier, registry_hash })
}

/// Peer descoberto na rede.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPeer {
    pub identifier: String,
    /// Endereço de ORIGEM do datagrama (IP do peer).
    pub source: SocketAddr,
    /// Porta TCP anunciada para o canal de dados.
    pub tcp_port: u16,
    pub registry_hash: u32,
}

/// Anunciante periódico do `fxpd` — `Drop`/`stop()` encerra a thread.
#[derive(Debug)]
pub struct Announcer {
    identifier: String,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Announcer {
    /// Anuncia `identifier` na porta TCP `tcp_port` a cada `interval`
    /// (multicast com loop ligado: o próprio host pode se ouvir; quem ouve
    /// filtra pelo identificador). A ORIGEM do beacon é o endereço da
    /// interface de saída multicast (padrão da rota) — por isso o servidor
    /// anunciado deve escutar em 0.0.0.0 (`serve_tcp_peer`), tornando o IP
    /// anunciado conectável por qualquer consumidor da sub-rede.
    pub fn start(
        identifier: &str,
        tcp_port: u16,
        registry_hash: u32,
        group: SocketAddr,
        interval: Duration,
    ) -> Result<Self, DiscoveryError> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
        if matches!(group.ip(), std::net::IpAddr::V6(_)) {
            return Err(DiscoveryError::MulticastIndisponivel(
                "grupo IPv6 fora do escopo v1.1 (§4.9)".into(),
            ));
        }
        socket
            .set_multicast_loop_v4(true)
            .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
        socket
            .set_multicast_ttl_v4(MULTICAST_TTL)
            .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
        let beacon = encode_beacon(identifier, tcp_port, registry_hash);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                let _ = socket.send_to(&beacon, group);
                // Sono em fatias curtas: parada responsiva sem polling de rede.
                let fim = Instant::now() + interval;
                while Instant::now() < fim && !flag.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        });
        Ok(Self { identifier: identifier.into(), stop, handle: Some(handle) })
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub fn stop(mut self) {
        self.desligar();
    }

    fn desligar(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Announcer {
    fn drop(&mut self) {
        self.desligar();
    }
}

/// Ouve o grupo por `window` e devolve os peers distintos anunciados
/// (dedupe por identificador + IP de origem; o próprio beacon repetido
/// não duplica). Falha de rede ⇒ `MulticastIndisponivel` (caminho honesto).
pub fn discover_peers(
    window: Duration,
    group: SocketAddr,
) -> Result<Vec<DiscoveredPeer>, DiscoveryError> {
    let group_ip = match group.ip() {
        std::net::IpAddr::V4(v4) => v4,
        std::net::IpAddr::V6(_) => {
            return Err(DiscoveryError::MulticastIndisponivel(
                "grupo IPv6 fora do escopo v1.1 (§4.9)".into(),
            ))
        }
    };
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, group.port()))
        .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
    socket
        .join_multicast_v4(&group_ip, &Ipv4Addr::UNSPECIFIED)
        .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;

    let mut peers: Vec<DiscoveredPeer> = Vec::new();
    let deadline = Instant::now() + window;
    let mut buf = [0u8; 512];
    while Instant::now() < deadline {
        match socket.recv_from(&mut buf) {
            Ok((n, source)) => {
                if let Ok(b) = decode_beacon(&buf[..n]) {
                    let peer = DiscoveredPeer {
                        identifier: b.identifier,
                        source,
                        tcp_port: b.tcp_port,
                        registry_hash: b.registry_hash,
                    };
                    if !peers.iter().any(|p| {
                        p.identifier == peer.identifier && p.source.ip() == peer.source.ip()
                    }) {
                        peers.push(peer);
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(_) => continue, // datagrama perdido: UDP é lossy (§4.9)
        }
    }
    Ok(peers)
}
