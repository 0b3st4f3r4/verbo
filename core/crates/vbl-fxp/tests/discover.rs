//! Descoberta multicast v1.1 (docs/FXP-SCHEMA-v1.md §4.9): beacon roundtrip,
//! anúncio→escuta em loopback e caminho honesto quando multicast não existe.
//!
//! Os testes de REDE degradam graciosamente (skip) quando o ambiente não
//! suporta multicast — lição do commit b7537d2 (cobertura não depende de host).

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vbl_fxp::discover::{
    decode_beacon, discover_peers, encode_beacon, registry_hash, Announcer, DiscoveryError,
};

/// Porta ÚNICA por teste (contador global): testes paralelos não brigam pelo
/// mesmo grupo/porta dentro do processo.
fn grupo_de_teste() -> SocketAddr {
    static N: AtomicUsize = AtomicUsize::new(0);
    SocketAddr::new(
        std::net::IpAddr::V4(Ipv4Addr::new(239, 255, 70, 88)),
        7200 + ((std::process::id() % 400) as u16) * 8 + N.fetch_add(1, Ordering::Relaxed) as u16,
    )
}

fn multicast_disponivel() -> bool {
    // Fumaça em porta EFÊMERA: valida suporte a multicast sem ocupar a porta
    // que o teste vai usar.
    let g = grupo_de_teste();
    let ip = match g.ip() { std::net::IpAddr::V4(v) => v, _ => return false };
    std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|s| s.join_multicast_v4(&ip, &Ipv4Addr::UNSPECIFIED))
        .is_ok()
}

#[test]
fn beacon_roundtrip_e_hash_do_registro() {
    let hash = registry_hash(&["cpu_temp".into(), "cpu_power".into(), "attention".into()]);
    let bytes = encode_beacon("fxpd-lab-1", 7000, hash);
    let b = decode_beacon(&bytes).expect("beacon válido");
    assert_eq!(b.identifier, "fxpd-lab-1");
    assert_eq!(b.tcp_port, 7000);
    assert_eq!(b.registry_hash, hash);
    // hash é determinístico e sensível à ordem canônica (ordenado antes).
    let de_novo = registry_hash(&["attention".into(), "cpu_power".into(), "cpu_temp".into()]);
    assert_eq!(hash, de_novo, "ordem de entrada não muda a impressão digital");
    // Datagramas truncados/corrompidos ⇒ BeaconInvalido (listener ignora).
    assert!(matches!(decode_beacon(&bytes[..6]), Err(DiscoveryError::BeaconInvalido)));
    assert!(matches!(decode_beacon(b"LIXO"), Err(DiscoveryError::BeaconInvalido)));
    let mut versao_estrangeira = encode_beacon("x", 1, 2);
    versao_estrangeira[4] = 9;
    assert!(matches!(
        decode_beacon(&versao_estrangeira),
        Err(DiscoveryError::BeaconInvalido)
    ));
}

#[test]
fn anuncio_e_escuta_no_loopback() {
    if !multicast_disponivel() {
        println!("skip: multicast indisponível neste ambiente (caminho honesto §4.9)");
        return;
    }
    let grupo = grupo_de_teste();
    let hash = registry_hash(&["cpu_temp".into()]);
    let _ann = Announcer::start("fxpd-bench-loop", 7123, hash, grupo, Duration::from_millis(50))
        .expect("anunciante");
    let peers = discover_peers(Duration::from_millis(400), grupo).expect("escuta");
    assert!(
        peers.iter().any(|p| p.identifier == "fxpd-bench-loop"
            && p.tcp_port == 7123
            && p.registry_hash == hash),
        "peer anunciado deve aparecer: {peers:?}"
    );
}

#[test]
fn beacon_repetido_nao_duplica_peer() {
    if !multicast_disponivel() {
        println!("skip: multicast indisponível neste ambiente (caminho honesto §4.9)");
        return;
    }
    let grupo = grupo_de_teste();
    let _ann = Announcer::start("fxpd-dup", 7155, 0xABCD, grupo, Duration::from_millis(30))
        .expect("anunciante");
    let peers = discover_peers(Duration::from_millis(350), grupo).expect("escuta");
    let do_peer: Vec<_> = peers.iter().filter(|p| p.identifier == "fxpd-dup").collect();
    assert_eq!(do_peer.len(), 1, "dedupe por identificador+IP: {peers:?}");
}

#[test]
fn sem_anuncio_nao_encontra_peer() {
    if !multicast_disponivel() {
        println!("skip: multicast indisponível neste ambiente (caminho honesto §4.9)");
        return;
    }
    // Janela curta sem anunciante na porta do PID: ninguém é inventado.
    let peers = discover_peers(Duration::from_millis(150), grupo_de_teste()).expect("escuta");
    assert!(peers.iter().all(|p| p.identifier != "nunca-anunciado"));
}

// ══════════════════════════════════════════════════════════════════════════
// §4.9 — caminhos de erro e ciclo de vida do Announcer.
// ══════════════════════════════════════════════════════════════════════════

#[test]
fn grupo_ipv6_suportado_e_ssm_v6_fora_do_escopo_v1_2() {
    use std::net::SocketAddrV6;
    // v1.2 §4.9: grupos IPv6 são aceitos (join com scope; hops = TTL §4.9).
    let v6 = SocketAddr::from(SocketAddrV6::new(
        "ff15::7080".parse().unwrap(),
        7080,
        0,
        0,
    ));
    // Sem multicast na rede o start pode falhar honesto — mas NUNCA com
    // "grupo IPv6 fora do escopo" (o que seria a recusa v1.1).
    if let Err(e) = Announcer::start("x", 1, 2, v6, Duration::from_secs(2)) {
        assert!(!matches!(&e, DiscoveryError::MulticastIndisponivel(m) if m.contains("fora do escopo")),
            "v1.2 aceita grupo IPv6: {e:?}");
    }
    if let Err(e) = discover_peers(Duration::from_millis(10), v6) {
        assert!(!matches!(&e, DiscoveryError::MulticastIndisponivel(m) if m.contains("fora do escopo")),
            "v1.2 aceita grupo IPv6: {e:?}");
    }
    // Fora do escopo REAL da v1.2: SSM com fonte em grupo IPv6 (§9) — o
    // parse rejeita honesto (SSM IPv6 aguarda API de socket).
    let err = vbl_fxp::discover::parse_group("[ff15::7080]:7080@127.0.0.1").unwrap_err();
    assert!(matches!(err, DiscoveryError::MulticastIndisponivel(m) if m.contains("SSM IPv6")));
}

#[test]
fn porta_de_grupo_ocupada_e_multicast_indisponivel() {
    if !multicast_disponivel() {
        println!("skip: multicast indisponível");
        return;
    }
    // Ocupar a porta ANTES: o listener do discover_peers falha honesto.
    let g = grupo_de_teste();
    let _ocupante = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, g.port())).expect("ocupar");
    let err = discover_peers(Duration::from_millis(20), g).unwrap_err();
    assert!(matches!(err, DiscoveryError::MulticastIndisponivel(_)));
}

#[test]
fn announcer_para_e_reporta_identificador() {
    if !multicast_disponivel() {
        println!("skip: multicast indisponível");
        return;
    }
    let g = grupo_de_teste();
    let ann = Announcer::start("fxpd-ciclo", 7000, 1, g, Duration::from_millis(50))
        .expect("anunciante");
    assert_eq!(ann.identifier(), "fxpd-ciclo");
    ann.stop(); // encerra a thread e junta — sem pânico, sem vazamento
}

#[test]
fn beacon_com_identificador_nao_utf8_e_ignorado() {
    let mut bytes = encode_beacon("ok", 1, 2);
    // Corrompe o identificador para bytes UTF-8 inválidos.
    let off = 8;
    bytes[off] = 0xFF;
    bytes[off + 1] = 0xFE;
    assert!(matches!(decode_beacon(&bytes), Err(DiscoveryError::BeaconInvalido)));
}
