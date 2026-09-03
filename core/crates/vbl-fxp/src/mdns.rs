//! mDNS/DNS-SD (v1.2 §4.10) — descoberta por DNS-SD via `mdns-sd` (puro
//! Rust, thread própria, sem runtime async). Opt-in por cargo feature
//! `mdns`: o FIO não muda (os mesmos frames §2); o que muda é só o
//! transporte do ANÚNCIO — serviço `_fxp._tcp.local.` com TXT `id`,
//! `hash` e, para peers TLS (§7), `tls=1` + `pin` (hex SHA-256).
//! Nenhuma descoberta é confiável: mDNS é lossy como o beacon (§4.9) —
//! ausência de resposta não é recusa; quem anuncia pode sumir a qualquer
//! instante (nenhum cache além da janela de escuta).

use crate::tls::{unhex32, Fingerprint};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::time::{Duration, Instant};

/// Tipo de serviço DNS-SD do FXP (v1.2 §4.10).
pub const SERVICE_TYPE: &str = "_fxp._tcp.local.";

/// Peer descoberto via mDNS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsPeer {
    /// Identificador do `fxpd` (TXT `id` — canônico para match).
    pub identifier: String,
    /// IP de controle (A/AAAA resolvido).
    pub host: std::net::IpAddr,
    /// Porta TCP anunciada para o canal de dados.
    pub port: u16,
    /// Hash do registro (u32, TXT `hash` — mesmo beacon §4.9).
    pub registry_hash: u32,
    /// Pin SHA-256 do certificado quando o peer anuncia TLS (`tls=1`).
    pub tls: Option<Fingerprint>,
}

/// Erros honestos de mDNS (§4.10): daemon indisponível ou janela sem
/// resolução — nada silencioso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdnsError {
    /// Daemon DNS-SD não pôde ser criado/consultado.
    Indisponivel(String),
}

impl std::fmt::Display for MdnsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MdnsError::Indisponivel(d) => write!(f, "mDNS indisponível: {d}"),
        }
    }
}

impl std::error::Error for MdnsError {}

/// Anunciante mDNS do `fxpd` — `Drop`/`stop()` desregistra o serviço.
pub struct MdnsAnnouncer {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsAnnouncer {
    /// Publica o serviço `_fxp._tcp.<identifier>` com TXT `id`/`hash` e,
    /// quando `tls` informado, `tls=1` + `pin` (hex do SHA-256 do DER do
    /// certificado). Endereço automático (A/AAAA reais da máquina).
    pub fn start(
        identifier: &str,
        tcp_port: u16,
        registry_hash: u32,
        tls: Option<Fingerprint>,
    ) -> Result<Self, MdnsError> {
        let daemon = ServiceDaemon::new()
            .map_err(|e| MdnsError::Indisponivel(format!("daemon: {e}")))?;
        // host_name único e estável: derivado do identificador (sem pontos,
        // para não colidir com sub-tipos do DNS-SD).
        let host = format!("fxp-{}.local.", identificador_seguro(identifier));
        let hash_hex = format!("{registry_hash:08x}");
        let mut props: Vec<(&str, &str)> =
            vec![("id", identifier), ("hash", &hash_hex)];
        let pin_hex;
        if let Some(pin) = tls {
            pin_hex = hex32(&pin);
            props.push(("tls", "1"));
            props.push(("pin", &pin_hex));
        }
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            identifier,
            &host,
            "",
            tcp_port,
            &props[..],
        )
        .map_err(|e| MdnsError::Indisponivel(format!("service info: {e}")))?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon
            .register(info)
            .map_err(|e| MdnsError::Indisponivel(format!("registro: {e}")))?;
        Ok(Self { daemon, fullname })
    }

    /// Desregistra (best effort) e encerra o daemon.
    pub fn stop(self) {
        let _ = self.daemon.unregister(&self.fullname);
    }
}

impl Drop for MdnsAnnouncer {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
    }
}

/// Escuta o serviço por `window` e devolve os peers distintos resolvidos
/// (dedupe por `id` + IP; mDNS repete anúncios — como o beacon §4.9).
pub fn discover_mdns(window: Duration) -> Result<Vec<MdnsPeer>, MdnsError> {
    let daemon = ServiceDaemon::new()
        .map_err(|e| MdnsError::Indisponivel(format!("daemon: {e}")))?;
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|e| MdnsError::Indisponivel(format!("browse: {e}")))?;
    let deadline = Instant::now() + window;
    let mut peers: Vec<MdnsPeer> = Vec::new();
    while Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                if let Some(peer) = peer_de(&info) {
                    if !peers.iter().any(|p| {
                        p.identifier == peer.identifier && p.host == peer.host
                    }) {
                        peers.push(peer);
                    }
                }
            }
            Ok(_) => continue,
            Err(_) => continue, // timeout/desconexo: mDNS é lossy (§4.9/§4.10)
        }
    }
    let _ = daemon.stop_browse(SERVICE_TYPE);
    Ok(peers)
}

/// TXT como &str — mdns-sd 0.21: `TxtProperties` devolve
/// `Option<Option<&[u8]>>` (chave ausente × vazio são distintos no DNS-SD).
fn txt<'a>(props: &'a mdns_sd::TxtProperties, chave: &str) -> Option<&'a str> {
    std::str::from_utf8(props.get_property_val(chave)??).ok()
}

/// Extrai o peer de um serviço resolvido (TXT obrigatório: `id` e `hash`;
/// sem eles o anúncio não é um fxpd legível ⇒ ignorado).
fn peer_de(info: &mdns_sd::ResolvedService) -> Option<MdnsPeer> {
    let identifier = txt(&info.txt_properties, "id")?.to_string();
    let hash = unhex32(txt(&info.txt_properties, "hash")?)?;
    let tls = match txt(&info.txt_properties, "tls") {
        Some("1") => Some(unhex32(txt(&info.txt_properties, "pin")?)?),
        _ => None,
    };
    // mDNS é lossy: um IP de controle basta (o primeiro resolvedor).
    let host: std::net::IpAddr = match info.addresses.iter().next()? {
        mdns_sd::ScopedIp::V4(v) => std::net::IpAddr::V4(*v.addr()),
        mdns_sd::ScopedIp::V6(v) => std::net::IpAddr::V6(*v.addr()),
        // Non-exhaustive: endereçamento desconhecido ⇒ sem peer honesto.
        _ => return None,
    };
    let port = info.port;
    // Mesma truncatura LE do beacon (§4.9, primeiros 4 bytes do SHA-256).
    let hash = u32::from_le_bytes([hash[0], hash[1], hash[2], hash[3]]);
    Some(MdnsPeer { identifier, host, port, registry_hash: hash, tls })
}

/// Instance/host seguro para DNS-SD: troca separadores por '-'.
fn identificador_seguro(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Hex minúsculo de 32 bytes.
fn hex32(bytes: &Fingerprint) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
