//! Descoberta multicast FXP v1.1/v1.2 — beacon `FXPD` em UDP
//! (docs/FXP-SCHEMA-v1.md §4.9). A v1.2 adiciona grupos IPv6 (join
//! `join_multicast_v6`, hops = TTL §4.9, scope do link-local) e SSM IPv4
//! (assinatura por fonte — RFC 4607; SSM IPv6 fica fora do escopo: nem std
//! nem socket2 expõem `MCAST_JOIN_SOURCE_GROUP` para v6 — §9 honesto).
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
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6, UdpSocket};
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

/// Parse da config de descoberta (v1.2 §4.9): `ip:porta` (IPv4),
/// `[v6]:porta` (IPv6; scope numérico opcional `%N` dentro do colchete) e
/// `@fonte-v4` para SSM — ex.: `239.255.70.81:7080@192.168.1.10`.
/// SSM com grupo IPv6 ⇒ erro honesto (fora do escopo, §9).
pub fn parse_group(s: &str) -> Result<(SocketAddr, Option<Ipv4Addr>), DiscoveryError> {
    let (alvo, fonte) = match s.split_once('@') {
        Some((a, f)) => (a, Some(f)),
        None => (s, None),
    };
    let grupo = if let Some(resto) = alvo.strip_prefix('[') {
        // IPv6: [addr(%N)]:porta
        let Some((addr, porta)) = resto.rsplit_once("]:") else {
            return Err(DiscoveryError::MulticastIndisponivel(format!(
                "grupo inválido: {s} (esperado [v6]:porta)"
            )));
        };
        let (ip, scope) = match addr.split_once('%') {
            Some((ip, escopo)) => {
                let n: u32 = escopo.parse().map_err(|_| {
                    DiscoveryError::MulticastIndisponivel(format!(
                        "scope numérico inválido: {s} (ex.: [fe80::1%3]:porta)"
                    ))
                })?;
                (ip, n)
            }
            None => (addr, 0),
        };
        let ip: Ipv6Addr = ip.parse().map_err(|_| {
            DiscoveryError::MulticastIndisponivel(format!("endereço v6 inválido: {s}"))
        })?;
        let porta: u16 = porta.parse().map_err(|_| {
            DiscoveryError::MulticastIndisponivel(format!("porta inválida: {s}"))
        })?;
        SocketAddr::V6(std::net::SocketAddrV6::new(ip, porta, 0, scope))
    } else {
        let Some((ip, porta)) = alvo.rsplit_once(':') else {
            return Err(DiscoveryError::MulticastIndisponivel(format!(
                "grupo inválido: {s} (esperado ip:porta)"
            )));
        };
        let ip: Ipv4Addr = ip.parse().map_err(|_| {
            DiscoveryError::MulticastIndisponivel(format!("endereço v4 inválido: {s}"))
        })?;
        let porta: u16 = porta.parse().map_err(|_| {
            DiscoveryError::MulticastIndisponivel(format!("porta inválida: {s}"))
        })?;
        SocketAddr::new(IpAddr::V4(ip), porta)
    };
    let fonte = match fonte {
        None => None,
        Some(f) => {
            if grupo.is_ipv6() {
                return Err(DiscoveryError::MulticastIndisponivel(
                    "SSM IPv6 fora do escopo v1.2 (§9): fonte só com grupo IPv4".into(),
                ));
            }
            Some(f.parse().map_err(|_| {
                DiscoveryError::MulticastIndisponivel(format!("fonte SSM inválida: {s}"))
            })?)
        }
    };
    Ok((grupo, fonte))
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
        Self::start_bound(identifier, tcp_port, registry_hash, group, interval, None)
    }

    /// Variante com bind local explícito (v1.2): para SSM, a FONTE do
    /// datagrama tem que ser o IP assinado pelo consumidor — anuncie com
    /// `local = Some(ip_do_servidor)`.
    pub fn start_bound(
        identifier: &str,
        tcp_port: u16,
        registry_hash: u32,
        group: SocketAddr,
        interval: Duration,
        local: Option<IpAddr>,
    ) -> Result<Self, DiscoveryError> {
        // socket2: hops v6 (equivalente do TTL v4 do §4.9) não existe no std.
        use socket2::{Domain, Protocol, Socket, Type};
        let socket = match (local, group.is_ipv4()) {
            (None, true) | (Some(IpAddr::V4(_)), true) => {
                let ip = match local {
                    Some(IpAddr::V4(v)) => v,
                    _ => Ipv4Addr::UNSPECIFIED,
                };
                let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
                s.bind(&SocketAddr::from((ip, 0)).into())
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
                s.set_multicast_loop_v4(true)
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
                s.set_multicast_ttl_v4(MULTICAST_TTL)
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
                s
            }
            (None, false) | (Some(IpAddr::V6(_)), false) => {
                let ip = match local {
                    Some(IpAddr::V6(v)) => v,
                    _ => Ipv6Addr::UNSPECIFIED,
                };
                let s = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
                s.bind(&SocketAddrV6::new(ip, 0, 0, 0).into())
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
                s.set_multicast_loop_v6(true)
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
                s.set_multicast_hops_v6(MULTICAST_TTL)
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
                s
            }
            (Some(_), _) => {
                return Err(DiscoveryError::MulticastIndisponivel(
                    "bind local com família diferente do grupo".into(),
                ))
            }
        };
        let socket: UdpSocket = socket.into();
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
    let (socket, _) = listener_multicast(group, None)?;
    coletar_peers(socket, window)
}

/// SSM (v1.2 §4.9): assina (fonte, grupo) — só datagramas DA FONTE chegam
/// (RFC 4607; IPv4). O servidor deve anunciar com bind na mesma fonte
/// ([`Announcer::start_bound`]).
pub fn discover_peers_ssm(
    window: Duration,
    group: SocketAddr,
    source: Ipv4Addr,
) -> Result<Vec<DiscoveredPeer>, DiscoveryError> {
    let (socket, _) = listener_multicast(group, Some(source))?;
    coletar_peers(socket, window)
}

/// Laço de escuta comum: dedupe por (identificador, IP de origem).
fn coletar_peers(
    socket: UdpSocket,
    window: Duration,
) -> Result<Vec<DiscoveredPeer>, DiscoveryError> {
    let socket = &socket;
    // Fatias curtas de escuta: a janela é um TETO real (§4.9) — sem isso um
    // grupo sem tráfego bloquearia o recv para sempre.
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

/// Listener multicast v1.2: v4 (com SSM quando `fonte` informado) ou v6
/// (join com scope; hops irrelevantes no listener). Devolve o socket pronto
/// para recv. Falha de rede ⇒ honesto (`MulticastIndisponivel`).
fn listener_multicast(
    group: SocketAddr,
    fonte: Option<Ipv4Addr>,
) -> Result<(UdpSocket, Option<Ipv4Addr>), DiscoveryError> {
    use socket2::{Domain, Protocol, Socket, Type};
    match group {
        SocketAddr::V4(v4) => {
            let grupo = *v4.ip();
            let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
                .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
            sock.set_reuse_address(true)
                .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
            sock.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, v4.port())).into())
                .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
            match fonte {
                // SSM (v1.2): assina (fonte, grupo) — sem ruído de outros fxpd.
                Some(f) => sock
                    .join_ssm_v4(&f, &grupo, &Ipv4Addr::UNSPECIFIED)
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?,
                None => sock
                    .join_multicast_v4(&grupo, &Ipv4Addr::UNSPECIFIED)
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?,
            }
            Ok((sock.into(), Some(grupo)))
        }
        SocketAddr::V6(v6) => {
            let grupo = *v6.ip();
            let sock = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
                .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
            sock.set_reuse_address(true)
                .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
            sock.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, v6.port())).into())
                .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
            // Interface = scope do grupo (0 = padrão da rota; link-local
            // exige o índice da interface para ter semântica).
            sock.join_multicast_v6(&grupo, v6.scope_id())
                .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?;
            Ok((sock.into(), None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_group_aceita_v4_v6_scope_ssm_e_recusa_honesto() {
        // v4/v6 válidos
        let (g, f) = parse_group("239.255.70.80:7080").expect("v4");
        assert!(g.is_ipv4() && f.is_none());
        let (g, f) = parse_group("[ff15::7080]:7080").expect("v6");
        assert!(g.is_ipv6() && f.is_none());
        let (g, f) = parse_group("[fe80::7080%3]:7080").expect("v6 scope");
        assert!(g.is_ipv6() && f.is_none());
        let (g, f) = parse_group("239.255.70.81:7080@127.0.0.1").expect("ssm");
        assert_eq!(f, Some(Ipv4Addr::LOCALHOST));
        let _ = g;
        // Recusas honestas: cada braço de erro do parse.
        assert!(parse_group("sem-separador").is_err());
        assert!(parse_group("[ff15::7080]:porta").is_err());
        assert!(parse_group("]:7080").is_err());
        assert!(parse_group("[ff15::7080%escopo]:7080").is_err());
        assert!(parse_group("[endereco]:7080").is_err());
        assert!(parse_group("endereco:7080").is_err());
        assert!(parse_group("239.255.70.80:99999").is_err());
        assert!(parse_group("[ff15::7080]:7080@10.0.0.1").is_err());
    }

    #[test]
    fn discovery_error_display_honesto() {
        let e = DiscoveryError::MulticastIndisponivel("motivo x".into());
        assert_eq!(e.to_string(), "descoberta multicast indisponível: motivo x");
        assert_eq!(DiscoveryError::BeaconInvalido.to_string(), "beacon FXPD malformado");
    }

    #[test]
    fn announcer_rejeita_bind_local_de_familia_divergente() {
        let v4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 7083);
        let err = Announcer::start_bound(
            "x",
            1,
            2,
            v4,
            Duration::from_secs(1),
            Some(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
        )
        .unwrap_err();
        assert!(matches!(err, DiscoveryError::MulticastIndisponivel(m) if m.contains("família")));
    }
}
