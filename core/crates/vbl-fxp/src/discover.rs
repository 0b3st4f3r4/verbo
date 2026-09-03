//! Descoberta multicast FXP v1.1/v1.2/v1.3 — beacon `FXPD` em UDP
//! (docs/FXP-SCHEMA-v1.md §4.9). A v1.2 adiciona grupos IPv6 (join
//! `join_multicast_v6`, hops = TTL §4.9, scope do link-local) e SSM IPv4
//! (assinatura por fonte — RFC 4607). A v1.3 completa a §9 da v1.2:
//! **SSM IPv6** (RFC 4604) via `setsockopt(IPPROTO_IPV6, MCAST_JOIN_SOURCE_GROUP)`
//! — nem `std` nem `socket2` 0.6 expõem a assinatura por fonte para v6; a
//! chamada crua (Unix, RFC 3678) desbloqueia o item. Fora do Unix o join
//! v6+fonte falha honesto (`MulticastIndisponivel`).
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

/// Fonte de assinatura SSM (v1.2/v1.3 §4.9, RFC 4607/4604): IPv4 simples ou
/// IPv6 com scope opcional (link-local exige o índice da interface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FonteSsm {
    /// Fonte IPv4 (v1.2).
    V4(Ipv4Addr),
    /// Fonte IPv6 (v1.3) — `scope` é o `sin6_scope_id` da fonte (0 = rota).
    V6 { addr: Ipv6Addr, scope: u32 },
}

impl FonteSsm {
    /// IP da fonte (para casar com a origem do datagrama em testes).
    pub fn ip(self) -> IpAddr {
        match self {
            FonteSsm::V4(a) => IpAddr::V4(a),
            FonteSsm::V6 { addr, .. } => IpAddr::V6(addr),
        }
    }
}

/// `[v6(%N)]` → `(addr, scope)` — usado no grupo e na fonte SSM v6.
fn parse_v6_com_scope(txt: &str, contexto: &str) -> Result<(Ipv6Addr, u32), DiscoveryError> {
    let (ip, scope) = match txt.split_once('%') {
        Some((ip, escopo)) => {
            let n: u32 = escopo.parse().map_err(|_| {
                DiscoveryError::MulticastIndisponivel(format!(
                    "scope numérico inválido: {contexto} (ex.: [fe80::1%3]:porta)"
                ))
            })?;
            (ip, n)
        }
        None => (txt, 0),
    };
    let ip: Ipv6Addr = ip.parse().map_err(|_| {
        DiscoveryError::MulticastIndisponivel(format!("endereço v6 inválido: {contexto}"))
    })?;
    Ok((ip, scope))
}

/// Parse da config de descoberta (v1.2/v1.3 §4.9): `ip:porta` (IPv4),
/// `[v6]:porta` (IPv6; scope numérico opcional `%N` dentro do colchete) e
/// `@fonte` para SSM — fonte v4 solta (`239.255.70.81:7080@192.168.1.10`,
/// v1.2) ou v6 escopada (`[ff35::7080]:7080@[fe80::1%2]`, v1.3). Fonte com
/// família diferente do grupo ⇒ erro honesto (SSM é mesmo família).
pub fn parse_group(s: &str) -> Result<(SocketAddr, Option<FonteSsm>), DiscoveryError> {
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
        let (ip, scope) = parse_v6_com_scope(addr, s)?;
        let porta: u16 = porta
            .parse()
            .map_err(|_| DiscoveryError::MulticastIndisponivel(format!("porta inválida: {s}")))?;
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
        let porta: u16 = porta
            .parse()
            .map_err(|_| DiscoveryError::MulticastIndisponivel(format!("porta inválida: {s}")))?;
        SocketAddr::new(IpAddr::V4(ip), porta)
    };
    let fonte = match fonte {
        None => None,
        Some(f) => Some(match grupo {
            // SSM v6 (v1.3 §4.9): fonte em colchetes, scope opcional.
            SocketAddr::V6(_) => {
                let Some(interno) = f.strip_prefix('[').and_then(|r| r.strip_suffix(']')) else {
                    return Err(DiscoveryError::MulticastIndisponivel(format!(
                        "fonte SSM v6 inválida: {s} (esperado @[v6%N])"
                    )));
                };
                let (addr, scope) = parse_v6_com_scope(interno, s)?;
                FonteSsm::V6 { addr, scope }
            }
            SocketAddr::V4(_) => {
                let Ok(v4) = f.parse() else {
                    return Err(DiscoveryError::MulticastIndisponivel(format!(
                        "fonte SSM inválida: {s}"
                    )));
                };
                FonteSsm::V4(v4)
            }
        }),
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
    let identifier = String::from_utf8(buf[8..8 + id_len].to_vec())
        .map_err(|_| DiscoveryError::BeaconInvalido)?;
    let off = 8 + id_len;
    let registry_hash = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
    Ok(Beacon {
        tcp_port,
        identifier,
        registry_hash,
    })
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
        Ok(Self {
            identifier: identifier.into(),
            stop,
            handle: Some(handle),
        })
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

/// SSM (v1.2/v1.3 §4.9): assina (fonte, grupo) — só datagramas DA FONTE chegam
/// (RFC 4607 para v4, RFC 4604 para v6). O servidor deve anunciar com bind na
/// mesma fonte ([`Announcer::start_bound`]).
pub fn discover_peers_ssm(
    window: Duration,
    group: SocketAddr,
    fonte: FonteSsm,
) -> Result<Vec<DiscoveredPeer>, DiscoveryError> {
    let (socket, _) = listener_multicast(group, Some(fonte))?;
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

/// Listener multicast v1.2/v1.3: v4 (com SSM quando `fonte` = [`FonteSsm::V4`])
/// ou v6 (join com scope; SSM v6 via [`FonteSsm::V6`] e o join crua da v1.3).
/// Hops irrelevantes no listener. Devolve o socket pronto para recv. Falha de
/// rede ⇒ honesto (`MulticastIndisponivel`).
fn listener_multicast(
    group: SocketAddr,
    fonte: Option<FonteSsm>,
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
                Some(FonteSsm::V4(f)) => sock
                    .join_ssm_v4(&f, &grupo, &Ipv4Addr::UNSPECIFIED)
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?,
                // Fonte v6 em grupo v4: família divergente (parse v1.3 já
                // recusa; guard honesto para quem chama a API diretamente).
                Some(FonteSsm::V6 { .. }) => {
                    return Err(DiscoveryError::MulticastIndisponivel(
                        "fonte SSM v6 em grupo IPv4 — SSM é mesmo família (§4.9)".into(),
                    ))
                }
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
            match fonte {
                // SSM IPv6 (v1.3 §4.9, RFC 4604): join (grupo, fonte) crua —
                // a assinatura por fonte v6 não existe no std/socket2 0.6.
                Some(FonteSsm::V6 {
                    addr: fonte_v6,
                    scope: escopo_fonte,
                }) => join_ssm_v6(&sock, &grupo, v6.scope_id(), &fonte_v6, escopo_fonte)?,
                Some(FonteSsm::V4(_)) => {
                    return Err(DiscoveryError::MulticastIndisponivel(
                        "fonte SSM v4 em grupo IPv6 — SSM é mesmo família (§4.9)".into(),
                    ))
                }
                None => sock
                    .join_multicast_v6(&grupo, v6.scope_id())
                    .map_err(|e| DiscoveryError::MulticastIndisponivel(e.to_string()))?,
            }
            Ok((sock.into(), None))
        }
    }
}

/// Join SSM IPv6 (v1.3 §4.9 — RFC 3678/4604): `setsockopt(IPPROTO_IPV6,
/// MCAST_JOIN_SOURCE_GROUP, group_source_req)`. A opção MCAST_* (42–48) é
/// compartilhada entre os níveis IPPROTO_IP/IPPROTO_IPV6 no Linux; nem std
/// nem socket2 0.6 a expõem para v6 e a crate `libc` liga a constante sem o
/// struct — o POD `repr(C)` local (sobre `sockaddr_storage` da libc) completa
/// o que a glibc define em `netinet/in.h`, desbloqueando o item registrado
/// na §9 da v1.2. Escopo honesto: **Linux** (o número da opção é definido
/// pelo SO — em BSD/macOS o valor difere e não se adivinha). Falha do OS ⇒
/// honesto (`MulticastIndisponivel`).
#[cfg(target_os = "linux")]
fn join_ssm_v6(
    sock: &socket2::Socket,
    grupo: &Ipv6Addr,
    escopo_grupo: u32,
    fonte: &Ipv6Addr,
    escopo_fonte: u32,
) -> Result<(), DiscoveryError> {
    use std::os::fd::AsRawFd;

    /// `group_source_req` da RFC 3678 (glibc `netinet/in.h`) — a crate libc
    /// (0.2.189) não o liga; POD plano, sem padding sensível.
    #[repr(C)]
    struct GroupSourceReq {
        gsr_interface: u32,
        gsr_group: libc::sockaddr_storage,
        gsr_source: libc::sockaddr_storage,
    }

    let req = GroupSourceReq {
        gsr_interface: escopo_grupo,
        gsr_group: sockaddr_in6_em_storage(grupo, escopo_grupo),
        gsr_source: sockaddr_in6_em_storage(fonte, escopo_fonte),
    };
    // SAFETY: `req` é um POD plano (sockaddr_storage embutidos), vivo pela
    // duração da chamada; setsockopt só lê.
    let r = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::IPPROTO_IPV6,
            libc::MCAST_JOIN_SOURCE_GROUP,
            &req as *const GroupSourceReq as *const libc::c_void,
            std::mem::size_of::<GroupSourceReq>() as libc::socklen_t,
        )
    };
    if r != 0 {
        return Err(DiscoveryError::MulticastIndisponivel(format!(
            "join SSM IPv6 ({grupo}@{fonte}): {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

/// `sockaddr_in6` (libc) copiado byte a byte para `sockaddr_storage` — o
/// padrão RFC 3678 (`memcpy(&gsr->gsr_group, &sin6, sizeof(sin6))`).
#[cfg(target_os = "linux")]
fn sockaddr_in6_em_storage(addr: &Ipv6Addr, scope: u32) -> libc::sockaddr_storage {
    let mut sa: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
    sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
    sa.sin6_port = 0; // porta é ignorada no join multicast
    sa.sin6_addr = libc::in6_addr {
        s6_addr: addr.octets(),
    };
    sa.sin6_scope_id = scope;
    let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    // SAFETY: sockaddr_storage é grande/alinhado para qualquer sockaddr;
    // copiamos só sizeof(sockaddr_in6) bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(
            &sa as *const libc::sockaddr_in6 as *const u8,
            &mut ss as *mut libc::sockaddr_storage as *mut u8,
            std::mem::size_of::<libc::sockaddr_in6>(),
        );
    }
    ss
}

/// Fora do Linux: o número da opção `MCAST_JOIN_SOURCE_GROUP` é definido pelo
/// SO e não se adivinha (BSD/macOS diferem do Linux) ⇒ falha honesta (§4.9).
#[cfg(not(target_os = "linux"))]
fn join_ssm_v6(
    _sock: &socket2::Socket,
    grupo: &Ipv6Addr,
    _escopo_grupo: u32,
    fonte: &Ipv6Addr,
    _escopo_fonte: u32,
) -> Result<(), DiscoveryError> {
    Err(DiscoveryError::MulticastIndisponivel(format!(
        "SSM IPv6 implementado via setsockopt Linux (RFC 3678/4604): {grupo}@{fonte}"
    )))
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
        let (g, f) = parse_group("239.255.70.81:7080@127.0.0.1").expect("ssm v4");
        assert_eq!(f, Some(FonteSsm::V4(Ipv4Addr::LOCALHOST)));
        let _ = g;
        // v1.3 §4.9 — SSM IPv6: fonte v6 escopada e global
        let (g, f) = parse_group("[ff35::7080]:7080@[fe80::1%2]").expect("ssm v6 link-local");
        assert!(g.is_ipv6());
        assert_eq!(
            f,
            Some(FonteSsm::V6 {
                addr: Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
                scope: 2
            })
        );
        let (_, f) = parse_group("[ff35::7080]:7080@[2001:db8::1]").expect("ssm v6 global");
        assert_eq!(
            f.map(FonteSsm::ip),
            Some(IpAddr::V6("2001:db8::1".parse().expect("v6")))
        );
        // Recusas honestas: cada braço de erro do parse.
        assert!(parse_group("sem-separador").is_err());
        assert!(parse_group("[ff15::7080]:porta").is_err());
        assert!(parse_group("]:7080").is_err());
        assert!(parse_group("[ff15::7080%escopo]:7080").is_err());
        assert!(parse_group("[endereco]:7080").is_err());
        assert!(parse_group("endereco:7080").is_err());
        assert!(parse_group("239.255.70.80:99999").is_err());
        // Família da fonte divergente do grupo (SSM é mesmo família):
        assert!(
            parse_group("[ff15::7080]:7080@10.0.0.1").is_err(),
            "fonte v4 solta em grupo v6"
        );
        assert!(
            parse_group("[ff15::7080]:7080@[10.0.0.1]").is_err(),
            "v4 colchetado não é fonte v6"
        );
        assert!(
            parse_group("239.255.70.80:7080@[fe80::1]").is_err(),
            "fonte v6 em grupo v4"
        );
        assert!(
            parse_group("[ff35::7080]:7080@[fe80::1%x]").is_err(),
            "scope da fonte não numérico"
        );
        assert!(
            parse_group("[ff35::7080]:7080@[fe80::1").is_err(),
            "colchete da fonte aberto"
        );
    }

    #[test]
    fn discovery_error_display_honesto() {
        let e = DiscoveryError::MulticastIndisponivel("motivo x".into());
        assert_eq!(e.to_string(), "descoberta multicast indisponível: motivo x");
        assert_eq!(
            DiscoveryError::BeaconInvalido.to_string(),
            "beacon FXPD malformado"
        );
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
