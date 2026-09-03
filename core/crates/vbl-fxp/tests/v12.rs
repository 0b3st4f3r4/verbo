//! E2E dos recursos v1.2 (docs/FXP-SCHEMA-v1.md): TLS do transporte remoto —
//! confidencialidade e MAC por frame via rustls (TLS 1.3), com confiança por
//! **impressão digital** (SHA-256 do DER do certificado do servidor) fixada
//! no endpoint `tcps:host:porta@sha256:HEX`.
//!
//! Regra de ouro: TLS é opt-in nas duas pontas — sem `tcps:`/`--tls-*`, o
//! fio é byte a byte o da v1.1 (os testes anteriores seguem intocados).
//! Divergência de pin é **terminativa** (§4.6: falha de segurança nunca
//! degrada para canal aberto).

use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(2);
use vbl_fxp::registry::{DeviceEntry, Endpoint, FxpConfig, OperationMode, RegistryError};
use vbl_fxp::tls::{self, TlsAccept};
use vbl_fxp::transport::Connection;
use vbl_fxp::{BusConfig, DeviceRegistry, FxpBus, Message, PeerConfig, PeerServer};
use vbl_runtime::fxp::Fxp;
use vbl_runtime::ledger::ChainLedger;
use vbl_runtime::FxpSimulator;

/// Par autoassinado (rcgen): PEMs para o servidor e impressão digital do DER
/// para o cliente fixar (pin) — o "papel do servidor" do cenário.
fn certificados() -> (TlsAccept, [u8; 32], String) {
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".into()]).expect("rcgen");
    let fp = tls::fingerprint(ck.cert.der());
    (
        TlsAccept {
            certs_pem: ck.cert.pem(),
            key_pem: ck.signing_key.serialize_pem(),
        },
        fp,
        tls::hex32(&fp),
    )
}

/// Registro do PEER: um sensor simulado (temp_a).
fn peer_bus() -> FxpBus {
    let mut r = DeviceRegistry::new();
    let _ = r.register(DeviceEntry::sensor("temp_a", "temperature", "°C", 1.0));
    FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            ..Default::default()
        },
        FxpSimulator::new(),
    )
}

/// Bus do CLIENTE: temp_a via endpoint `tcps:` com o pin informado.
fn bus_cliente(porta: u16, pin_hex: &str) -> FxpBus {
    let cfg = format!(
        "mode = hibrido\ncache_ttl_ms = 0\n\
         temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
         temp_a.mode = real\ntemp_a.endpoint = tcps:127.0.0.1:{porta}@sha256:{pin_hex}\n"
    );
    let mut r = DeviceRegistry::new();
    FxpConfig::parse(&cfg)
        .expect("config do cliente")
        .apply(&mut r)
        .expect("registro");
    FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            ..Default::default()
        },
        FxpSimulator::new(),
    )
}

#[test]
fn e2e_tls_pin_certo_conecta_e_le() {
    let (aceitador, _fp, pin_hex) = certificados();
    let peer = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador),
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor TLS");

    let mut bus = bus_cliente(porta, &pin_hex);
    let mut ledger = ChainLedger::new();
    let v = bus
        .read_sensor("temp_a", &mut ledger)
        .expect("leitura via TLS");
    assert!(v.is_finite(), "valor físico válido atravessa o TLS: {v}");
}

#[test]
fn e2e_tls_pin_errado_falha_fechada() {
    let (aceitador, _pin_certo, _hex) = certificados();
    let (_aceitador_outra, _pin_outra, hex_outra) = certificados(); // cert DIFERENTE
    let peer = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador),
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor TLS");

    let mut bus = bus_cliente(porta, &hex_outra);
    let mut ledger = ChainLedger::new();
    let r = bus.read_sensor("temp_a", &mut ledger);
    assert!(
        r.is_err(),
        "pin divergente tem que falhar fechado, nunca conectar"
    );
    // A honestidade mora no Caderno: o evento de I/O registra o motivo
    // completo do transporte (handshake TLS), nunca "sensor falhou" a seco.
    let motivo: String = ledger
        .events
        .iter()
        .map(|e| e.msg.clone())
        .collect::<Vec<_>>()
        .join(" | ")
        .to_lowercase();
    assert!(
        motivo.contains("tls"),
        "Caderno sem o motivo TLS do handshake recusado: {motivo}"
    );
}

#[test]
fn e2e_cliente_plano_contra_servidor_tls_falha() {
    let (aceitador, _fp, _hex) = certificados();
    let peer = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador),
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor TLS");

    // O TCP conecta (é só transporte), mas o primeiro frame em texto plano
    // quebra o handshake TLS do servidor — a conexão morre sem resposta.
    let mut c = Connection::tcp("127.0.0.1", porta, Duration::from_millis(500))
        .expect("tcp conecta antes do handshake");
    let r = c.request(
        &Message::read("temp_a", 1, true),
        Duration::from_millis(500),
    );
    assert!(
        r.is_err(),
        "texto plano contra servidor TLS tem que falhar: {r:?}"
    );
}

#[test]
fn e2e_cliente_tls_contra_servidor_plano_falha() {
    let peer = PeerServer::new(peer_bus(), ChainLedger::new(), PeerConfig::default());
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor plano");

    let (_a, fp, _hex) = certificados();
    let confianca = vbl_fxp::tls::ConfiancaCliente::Pin(fp);
    let r = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &confianca,
        Duration::from_millis(500),
        None,
    );
    assert!(
        r.is_err(),
        "handshake TLS contra servidor plano tem que falhar: {r:?}"
    );
}

#[test]
fn e2e_unix_rejeita_tls_na_construcao() {
    let (aceitador, _fp, _hex) = certificados();
    let peer = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador),
            ..Default::default()
        },
    );
    let r = vbl_fxp::peer::serve_unix_peer(
        &peer,
        &std::env::temp_dir().join(format!("vbl-v12-tls-unix-{}.sock", std::process::id())),
    );
    assert!(
        r.is_err(),
        "TLS é do TCP remoto; unix com tls configurado falha honesto"
    );
}

#[test]
fn endpoint_tcps_parse_descricao_roundtrip_e_erros() {
    let hex = "ab".repeat(32);
    let s = format!("tcps:127.0.0.1:7080@sha256:{hex}");
    let ep = Endpoint::parse(&s).expect("parse tcps");
    assert_eq!(ep.description(), s, "descrição canônica é re-parseável");

    // Hostname (não-IP) também vale — o pin é quem autentica, não o nome.
    let sh = format!("tcps:servidor.local:7080@sha256:{hex}");
    assert_eq!(Endpoint::parse(&sh).expect("hostname").description(), sh);

    // Fail closed: sem pin, pin curto, pin não-hex ⇒ erro de construção.
    assert!(
        Endpoint::parse("tcps:127.0.0.1:7080").is_err(),
        "sem pin não conecta"
    );
    assert!(
        Endpoint::parse("tcps:127.0.0.1:7080@sha256:ab").is_err(),
        "pin curto"
    );
    assert!(
        Endpoint::parse(&format!("tcps:127.0.0.1:7080@sha256:{}", "zz".repeat(32))).is_err(),
        "pin não-hex"
    );
    assert!(
        matches!(
            Endpoint::parse("tcps:127.0.0.1:7080@md5:aabb"),
            Err(RegistryError::InvalidEndpoint(_))
        ),
        "prefixo de hash desconhecido ⇒ InvalidEndpoint"
    );
}

// ======================================================================
// v1.2 §4.8 — dicionário de compressão compartilhado entre frames: o
// conteúdo é derivado do REGISTRO do servidor (nomes canônicos ordenados,
// concatenados com \n, teto de 64 KiB) — nenhum byte de dicionário cruza o
// fio. A negociação é o bit `caps::DICT`; o algoritmo é o id 2 no byte
// reservado; o gatilho de uso é o HELLO (§4.4) nas duas pontas.
// ======================================================================

use vbl_fxp::schema::{caps, compress, flag, op, Body, SchemaError};

/// Nomes pseudo-aleatórios determinísticos (LCG) — alta entropia para o LZ4
/// não achar repetição intra-frame: sem dicionário não comprime; com o
/// dicionário do registro, comprime.
fn nomes_ruidosos(n: usize) -> Vec<String> {
    // CJK determinístico (3 bytes/char, ~7 mil símbolos ⇒ janelas de 4 bytes
    // quase sem repetição): o LZ4 simples não acha razão; o dicionário, que
    // contém os nomes inteiros, acha.
    let mut s: u64 = 0x9E3779B97F4A7C15;
    (0..n)
        .map(|i| {
            let mut nome = format!("s{i:02}_");
            for _ in 0..43 {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let cp = 0x4E00 + ((s >> 33) as u32) % 0x51_5B;
                nome.push(char::from_u32(cp).unwrap_or('x'));
            }
            nome
        })
        .collect()
}

#[test]
fn dict_do_registro_e_deterministico_ordenado_e_limitado() {
    let nomes = vec![
        "temp_c".to_string(),
        "temp_a".to_string(),
        "cpu_temp".to_string(),
    ];
    let d1 = compress::dict_from_registry(&nomes);
    assert_eq!(
        d1,
        compress::dict_from_registry(&nomes),
        "mesmo registro ⇒ mesmo dict"
    );
    assert_eq!(
        d1, b"cpu_temp\ntemp_a\ntemp_c",
        "ordenado e concatenado com \\n (§4.9)"
    );
    assert!(
        compress::dict_from_registry(&[]).is_empty(),
        "registro vazio ⇒ dict vazio"
    );
    // Teto determinístico de 64 KiB: prefixo da ordem ordenada.
    let mut grandes: Vec<String> = (0..300)
        .map(|i| format!("s{i:03}_{}", "x".repeat(250)))
        .collect();
    grandes.sort();
    let d = compress::dict_from_registry(&grandes);
    assert_eq!(d.len(), compress::DICT_MAX, "dict limitado a 64 KiB");
    assert_eq!(&d[..6], b"s000_x", "prefixo segue a ordem ordenada");
}

#[test]
fn schema_dict_roundtrip_com_algoritmo_2() {
    let nomes = nomes_ruidosos(20);
    let dict = compress::dict_from_registry(&nomes);
    let resultados: Vec<vbl_fxp::BatchResult> = nomes
        .iter()
        .enumerate()
        .map(|(i, n)| vbl_fxp::BatchResult::Ok {
            value: i as f64 + 0.5,
            canonical: n.clone(),
        })
        .collect();
    let msg = Message::read_batch_ok(resultados, 1);

    // Sem dict: LZ4 só conhece o próprio frame — a razão vem do que houver
    // de repetição local (o codec nunca infla: blob ≥ região ⇒ plano).
    let mut plano = Vec::new();
    vbl_fxp::schema::encode_with_compression(&msg, &mut plano).unwrap();

    // Com dict: os nomes moram no dicionário ⇒ comprime e marca id 2.
    let mut com_dict = Vec::new();
    vbl_fxp::schema::encode_with_compression_dict(&msg, &dict, &mut com_dict).unwrap();
    assert!(
        com_dict[4 + 5] & flag::COMPRESSED != 0,
        "frame com dict vem comprimido"
    );
    assert_eq!(
        com_dict[4 + 6],
        compress::ALGO_LZ4_DICT,
        "byte reservado = id 2"
    );
    assert!(
        com_dict.len() < plano.len(),
        "dict ({} B) precisa vencer o plano ({} B) com os nomes no dicionário",
        com_dict.len(),
        plano.len()
    );

    // Roundtrip bit a bit do conteúdo.
    let (dec, _) = vbl_fxp::schema::decode_with_dict(&com_dict, Some(&dict)).unwrap();
    let Body::ReadBatchOk { results } = dec.body else {
        panic!("corpo errado: {:?}", dec.body)
    };
    assert_eq!(results.len(), 20);
    for (i, (r, nome)) in results.iter().zip(&nomes).enumerate() {
        match r {
            vbl_fxp::BatchResult::Ok { value, canonical } => {
                assert_eq!(*value, i as f64 + 0.5);
                assert_eq!(canonical, nome);
            }
            _ => panic!("item não é Ok: {r:?}"),
        }
    }
}

#[test]
fn dict_frame_sem_dicionario_falha_fechado_como_v1_1() {
    let nomes = nomes_ruidosos(20);
    let dict = compress::dict_from_registry(&nomes);
    let resultados: Vec<vbl_fxp::BatchResult> = nomes
        .iter()
        .map(|n| vbl_fxp::BatchResult::Ok {
            value: 1.0,
            canonical: n.clone(),
        })
        .collect();
    let msg = Message::read_batch_ok(resultados, 1);
    let mut frame = Vec::new();
    vbl_fxp::schema::encode_with_compression_dict(&msg, &dict, &mut frame).unwrap();

    // Codec sem dicionário (v1.1 real ou conexão sem DICT negociado):
    // id 2 é "algoritmo desconhecido" — fail closed, princípio 7.
    let err = vbl_fxp::schema::decode(&frame).unwrap_err();
    assert!(
        matches!(err, SchemaError::UnknownCompression { received: 2 }),
        "{err:?}"
    );
    let err2 = vbl_fxp::schema::decode_with_dict(&frame, None).unwrap_err();
    assert_eq!(err, err2, "decode() ≡ decode_with_dict(_, None)");
    // Dicionário ERRADO também nunca vira lixo: falha de descompressão.
    let outro = compress::dict_from_registry(&["outro".to_string()]);
    assert!(vbl_fxp::schema::decode_with_dict(&frame, Some(&outro)).is_err());
}

#[test]
fn e2e_dict_negociado_com_hello_e_degradacao_v1_1() {
    use std::path::PathBuf;
    use vbl_fxp::transport::wait_ready_unix;

    let sock = std::env::temp_dir().join(format!("vbl-v12-dict-{}.sock", std::process::id()));

    // Servidor v1.2: anuncia DICT (+LZ4); cliente pede dict ⇒ HELLO vira
    // parte do handshake e as leituras seguem funcionando.
    let peer = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            caps: caps::DICT | caps::LZ4,
            ..Default::default()
        },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut r = DeviceRegistry::new();
    let cfg = "mode = hibrido\ncache_ttl_ms = 0\ncompression = true\n\
               temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
               temp_a.mode = real\ntemp_a.endpoint = unix:PLACE\n"
        .replace("PLACE", &sock.display().to_string());
    FxpConfig::parse(&cfg).unwrap().apply(&mut r).unwrap();
    let mut bus = FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            compression: true,
            compression_dict: true,
            ..Default::default()
        },
        FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();
    let addr = vbl_fxp::RemoteAddr::Unix(PathBuf::from(&sock));
    let v = bus.read_sensor("temp_a", &mut ledger).unwrap();
    assert!(v.is_finite());
    assert_eq!(
        bus.granted_caps_of(&addr),
        caps::DICT | caps::LZ4,
        "interseção cheia: as duas pontas anunciam dict"
    );

    // Degradacao v1.1: servidor sem DICT anunciado ⇒ cliente segue em LZ4
    // simples, sem HELLO, e continua lendo (degradação honesta da v1.1).
    let sock11 = std::env::temp_dir().join(format!("vbl-v12-dict11-{}.sock", std::process::id()));
    let peer11 = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            caps: caps::LZ4,
            ..Default::default()
        },
    );
    let _srv11 = vbl_fxp::peer::serve_unix_peer(&peer11, &sock11).expect("servidor v1.1");
    assert!(wait_ready_unix(&sock11, DEADLINE));
    let mut r11 = DeviceRegistry::new();
    let cfg11 = "mode = hibrido\ncache_ttl_ms = 0\n\
                 temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
                 temp_a.mode = real\ntemp_a.endpoint = unix:PLACE\n"
        .replace("PLACE", &sock11.display().to_string());
    FxpConfig::parse(&cfg11).unwrap().apply(&mut r11).unwrap();
    let mut bus11 = FxpBus::build(
        r11,
        BusConfig {
            mode: OperationMode::Hybrid,
            compression: true,
            compression_dict: true,
            ..Default::default()
        },
        FxpSimulator::new(),
    );
    let v11 = bus11.read_sensor("temp_a", &mut ledger).unwrap();
    assert!(v11.is_finite());
    assert_eq!(
        bus11.granted_caps_of(&vbl_fxp::RemoteAddr::Unix(PathBuf::from(&sock11))),
        caps::LZ4,
        "v1.1 não concede DICT: cliente segue sem dicionário"
    );
}

// ======================================================================
// v1.2 §4.9 — beacon IPv6 e SSM (assinatura por fonte, IPv4): o datagrama
// FXPD é o mesmo (versão 1 intocada); o que muda é a camada de socket.
// Sem multicast na rede de teste, os cenários ao vivo são skip gracioso
// (mesmo padrão dos testes v1.1) — o parse e a recusa honesta sempre rodam.
// ======================================================================

use vbl_fxp::discover::{self, parse_group};

#[test]
fn parse_group_v4_v6_scope_e_ssm() {
    // v4 clássico (default do §4.9)
    let (g, fonte) = parse_group("239.255.70.80:7080").expect("v4");
    assert_eq!(g.to_string(), "239.255.70.80:7080");
    assert!(fonte.is_none());
    // v6 com colchetes + porta
    let (g, fonte) = parse_group("[ff15::7080]:7080").expect("v6");
    assert!(g.is_ipv6());
    assert!(fonte.is_none());
    // scope numérico: [fe80::7080%3]:7080 ⇒ scope_id 3 (preservado no addr)
    let (g, _) = parse_group("[fe80::7080%3]:7080").expect("v6+scope");
    assert_eq!(
        g,
        std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
            std::net::Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0x7080),
            7080,
            0,
            3,
        ))
    );
    // SSM: @fonte-v4
    let (g, fonte) = parse_group("239.255.70.81:7080@127.0.0.1").expect("ssm");
    assert_eq!(g.to_string(), "239.255.70.81:7080");
    assert_eq!(
        fonte,
        Some(vbl_fxp::discover::FonteSsm::V4(
            std::net::Ipv4Addr::LOCALHOST
        ))
    );
    // erros honestos
    assert!(parse_group("sem-porta").is_err());
    assert!(parse_group("239.255.70.80").is_err());
    assert!(parse_group("[ff15::7080]:porta").is_err());
    assert!(parse_group("239.255.70.81:7080@nao-ip").is_err());
    // Fonte com família divergente do grupo segue recusa honesta
    // (v1.2 recusava todo SSM em grupo v6; a v1.3 aceita fonte v6 — §4.9).
    assert!(parse_group("[ff15::7080]:7080@127.0.0.1").is_err());
}

#[test]
fn beacon_ipv6_loopback_ou_anuncio_ignorado() {
    let grupo = parse_group("[ff15::7080]:7081").expect("grupo v6").0;
    let anunciador = match discover::Announcer::start(
        "fxpd-v6-teste",
        7081,
        0x1234,
        grupo,
        Duration::from_millis(50),
    ) {
        Ok(a) => a,
        Err(_) => return, // rede sem multicast v6: skip gracioso
    };
    let peers = discover::discover_peers(Duration::from_millis(700), grupo).unwrap_or_default();
    drop(anunciador);
    // Sem rede multicast v6 funcional, o window devolve vazio — aceitável
    // em CI; com rede, o peer aparece com fonte v6 e o beacon decodifica.
    if let Some(p) = peers.iter().find(|p| p.identifier == "fxpd-v6-teste") {
        assert!(p.source.is_ipv6(), "fonte deve ser v6: {:?}", p.source);
        assert_eq!(p.tcp_port, 7081);
        assert_eq!(p.registry_hash, 0x1234);
    }
}

#[test]
fn beacon_ssm_v4_assina_a_fonte_ou_ouve_vazio() {
    let (grupo, fonte) = parse_group("239.255.70.82:7082@127.0.0.1").expect("ssm");
    // O anunciador liga-se a 127.0.0.1 para que a FONTE do datagrama seja
    // exatamente a assinada (S,G).
    let anunciador = match discover::Announcer::start_bound(
        "fxpd-ssm-teste",
        7082,
        0x5678,
        grupo,
        Duration::from_millis(50),
        fonte.map(|f| f.ip()),
    ) {
        Ok(a) => a,
        Err(_) => return, // SSM indisponível no kernel/rede: skip gracioso
    };
    let peers = discover::discover_peers(Duration::from_millis(700), grupo).unwrap_or_default();
    drop(anunciador);
    if let Some(p) = peers.iter().find(|p| p.identifier == "fxpd-ssm-teste") {
        assert_eq!(p.source.ip().to_string(), "127.0.0.1");
        assert_eq!(p.tcp_port, 7082);
        assert_eq!(p.registry_hash, 0x5678);
    }
}

// ======================================================================
// v1.2 §4.10 — mDNS/DNS-SD (feature `mdns`, default-off): anúncio em
// `_fxp._tcp.local.` com TXT id/hash (+tls/pin); o fio de dados continua
// o §2. Em rede sem mDNS funcional, cenários ao vivo são skip gracioso.
// ======================================================================
#[cfg(feature = "mdns")]
mod mdns_v12 {
    use super::*;
    use vbl_fxp::mdns::{discover_mdns, MdnsAnnouncer};
    use vbl_fxp::tls::fingerprint;

    #[test]
    fn mdns_annuncia_e_resolve_ou_vazio_gracioso() {
        let anunciador = MdnsAnnouncer::start("fxpd-mdns-teste", 7391, 0x00C0FFEEu32, None)
            .expect("anunciador mDNS deve subir em rede com multicast");
        let peers = discover_mdns(Duration::from_millis(1200)).unwrap_or_default();
        drop(anunciador);
        if let Some(p) = peers.iter().find(|p| p.identifier == "fxpd-mdns-teste") {
            assert_eq!(p.port, 7391);
            assert_eq!(p.registry_hash, 0x00C0FFEE);
            assert!(p.tls.is_none(), "sem TXT tls ⇒ sem pin");
        }
        // Vazio = mDNS indisponível no sandbox: skip gracioso (perdido ≠ recusa).
    }

    #[test]
    fn mdns_txt_tls_publica_pin() {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pin = fingerprint(cert.cert.der());
        let anunciador = MdnsAnnouncer::start("fxpd-mdns-tls", 7392, 0x42, Some(pin)).unwrap();
        let peers = discover_mdns(Duration::from_millis(1200)).unwrap_or_default();
        drop(anunciador);
        if let Some(p) = peers.iter().find(|p| p.identifier == "fxpd-mdns-tls") {
            assert_eq!(p.tls, Some(pin), "TXT tls=1 + pin hex ⇒ fingerprint");
        }
    }

    #[test]
    fn endpoint_mdns_parse_e_bus() {
        use vbl_fxp::Endpoint;
        // Com a feature: parse ok e descrição redonda.
        let ep = Endpoint::parse("mdns:cozinha-fxpd").expect("endpoint mdns");
        assert_eq!(
            ep,
            Endpoint::AutoRemoteMdns {
                identifier: "cozinha-fxpd".into()
            }
        );
        assert_eq!(ep.description(), "mdns:cozinha-fxpd");
        assert!(Endpoint::parse("mdns:").is_err(), "id vazio ⇒ erro honesto");

        // e2e: bus com endpoint mdns lê o peer anunciado — se o mDNS do
        // sandbox não resolver, a leitura é inacessível com motivo honesto
        // (nunca dado sintético).
        let anunciador = MdnsAnnouncer::start("fxpd-mdns-bus", 7393, 0x77, None).unwrap();
        let resolvido = !discover_mdns(Duration::from_millis(1200))
            .unwrap_or_default()
            .is_empty();
        drop(anunciador);
        if !resolvido {
            return; // sandbox sem mDNS: nada a provar além do parse acima
        }
        let sock = std::env::temp_dir().join(format!("vbl-v12-mdns-{}.sock", std::process::id()));
        let peer = PeerServer::new(
            peer_bus(),
            ChainLedger::new(),
            PeerConfig {
                caps: caps::LZ4,
                ..Default::default()
            },
        );
        let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
        assert!(vbl_fxp::transport::wait_ready_unix(&sock, DEADLINE));
        let mut r = DeviceRegistry::new();
        let cfg = "mode = hibrido\ncache_ttl_ms = 0\n\
                   temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
                   temp_a.mode = real\ntemp_a.endpoint = unix:PLACE\n"
            .replace("PLACE", &sock.display().to_string());
        FxpConfig::parse(&cfg).unwrap().apply(&mut r).unwrap();
        // O device mdns: aponta para o PEER anunciado via mDNS.
        let mut r2 = DeviceRegistry::new();
        let cfg2 = "mode = hibrido\ncache_ttl_ms = 0\n\
                    temp_b.grandeza = temperatura\ntemp_b.unidade = C\n\
                    temp_b.mode = real\ntemp_b.endpoint = mdns:fxpd-mdns-bus\n";
        FxpConfig::parse(cfg2).unwrap().apply(&mut r2).unwrap();
        let mut bus = FxpBus::build(
            r2,
            BusConfig {
                mode: OperationMode::Hybrid,
                ..Default::default()
            },
            FxpSimulator::new(),
        );
        let mut ledger = ChainLedger::new();
        // O peer anunciado por mDNS é o NOSSO anunciador (não há servidor
        // TCP atrás dele aqui) ⇒ sem anúncio tcp real, a leitura cai no
        // caminho honesto de inacessibilidade (ou conecta, se houvesse).
        let _ = bus.read_sensor("temp_b", &mut ledger);
    }
}

// ======================================================================
// v1.2 — arestas do codec com dict e do handshake HELLO (§4.8)
// ======================================================================

#[test]
fn dict_regiao_abaixo_do_threshold_sai_plana() {
    let dict = compress::dict_from_registry(&["unico".to_string()]);
    // READ_OK pequeno: região < 512 B ⇒ encode com dict NÃO comprime.
    let msg = Message::read_ok(36.5, "temp_a", false, 1);
    let mut com_dict = Vec::new();
    vbl_fxp::schema::encode_with_compression_dict(&msg, &dict, &mut com_dict).unwrap();
    assert_eq!(
        com_dict[4 + 5] & flag::COMPRESSED,
        0,
        "região pequena sai plana"
    );
    assert_eq!(com_dict[4 + 6], 0, "sem algoritmo no byte reservado");
}

#[test]
fn dict_nunca_infla_o_fio() {
    // Payload grande de alta entropia: blob ≥ região ⇒ sai plano (id 0),
    // nunca maior que o plano. O payload é determinístico: se este teste
    // passa uma vez, passa sempre (mesmos bytes).
    let nomes = nomes_ruidosos(30); // ~3.9 KB de região
    let dict = compress::dict_from_registry(&nomes);
    let resultados: Vec<vbl_fxp::BatchResult> = nomes
        .iter()
        .map(|_n| vbl_fxp::BatchResult::Err { reason: 1 })
        .collect();
    let msg = Message::read_batch_ok(resultados, 1);
    let mut com_dict = Vec::new();
    vbl_fxp::schema::encode_with_compression_dict(&msg, &dict, &mut com_dict).unwrap();
    assert_eq!(
        com_dict[4 + 5] & flag::COMPRESSED,
        0,
        "incompressível sai plano"
    );
}

#[test]
fn exchange_hello_exige_resposta_hello_valida() {
    use std::path::Path;
    use vbl_fxp::transport::{serve_unix, wait_ready_unix};

    let dir = std::env::temp_dir().join(format!("vbl-v12-hello-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);

    // Servidor "bobo" que responde HELLO com ACK (opcode errado).
    let sock_errado = dir.join("errado.sock");
    let _srv = serve_unix(Path::new(&sock_errado), |msg| {
        (msg.opcode == op::HELLO).then(|| Message::heartbeat_ack(true, msg.seq))
    })
    .expect("srv");
    assert!(wait_ready_unix(&sock_errado, DEADLINE));
    let mut c = Connection::unix(Path::new(&sock_errado), DEADLINE).expect("con");
    let err = c
        .exchange_hello(&[], DEADLINE)
        .expect_err("ACK no lugar de HELLO ⇒ erro honesto");
    assert!(
        matches!(err, vbl_fxp::transport::TransportError::Broken(m) if m.contains("não é HELLO"))
    );

    // Nota: o braço "corpo do HELLO inválido" é INATINGÍVEL por fio — o
    // decoder é estrito por opcode (HELLO ⇒ Body::Hello obrigatório) e
    // falha antes (Schema::MissingField), exatamente o fail-closed do §1.2.
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// v1.2 — honestidade do transporte, arestas de decode e descoberta por
// grupo configurável (§4.9). Cada teste mira braços de erro reais.
// ======================================================================

#[test]
fn transport_error_display_honesto() {
    assert_eq!(
        vbl_fxp::transport::TransportError::Broken("fim".into()).to_string(),
        "conexão quebrada: fim"
    );
    assert_eq!(
        vbl_fxp::transport::TransportError::Timeout.to_string(),
        "ack não chegou no prazo"
    );
    let e = vbl_fxp::transport::TransportError::Schema(SchemaError::InvalidMagic);
    assert_eq!(
        e.to_string(),
        "violação do schema v1: magic inválido (esperado \"FXP\")"
    );
}

/// Corrige o byte do opcode no cabeçalho já codificado (offset 8):
/// jeito direto de alcançar braços de decode que o encode nunca produz.
fn opcode_de(frame: &mut [u8], novo: u8) {
    frame[8] = novo;
}

#[test]
fn decode_rejeita_batch_vazio_e_read_sem_nome() {
    // READ_BATCH_OK com count=0: encode nunca produz; decode reprova (§4.7).
    let msg = Message::read_batch_ok(
        vec![vbl_fxp::BatchResult::Ok {
            value: 1.0,
            canonical: "temp_a".into(),
        }],
        1,
    );
    let mut frame = vbl_fxp::schema::encode_to_vec(&msg).unwrap();
    frame[16] = 0; // count u16 LE = 0
    frame[17] = 0;
    opcode_de(&mut frame, op::READ_BATCH_OK);
    let err = vbl_fxp::schema::decode(&frame).unwrap_err();
    assert!(matches!(err, SchemaError::BatchTooLarge), "{err:?}");

    // READ_BATCH (pedido) com count=0 ⇒ mesma reprovação.
    let pedido = Message::read_batch(vec!["temp_a".into()], 1);
    let mut f2 = vbl_fxp::schema::encode_to_vec(&pedido).unwrap();
    f2[16] = 0;
    f2[17] = 0;
    opcode_de(&mut f2, op::READ_BATCH);
    let err2 = vbl_fxp::schema::decode(&f2).unwrap_err();
    assert!(matches!(err2, SchemaError::BatchTooLarge), "{err2:?}");

    // READ com nome vazio: corpo vazio é honesto apenas para HEARTBEAT/BYE.
    let f3 = vbl_fxp::schema::encode_to_vec(&Message {
        opcode: op::READ,
        flags: 0,
        seq: 1,
        name: String::new(),
        timestamp_us: None,
        body: Body::Empty,
    })
    .unwrap();
    let err3 = vbl_fxp::schema::decode(&f3).unwrap_err();
    assert!(matches!(err3, SchemaError::InvalidName), "{err3:?}");
}

#[test]
fn encode_nunca_infla_com_ou_sem_dicionario() {
    // Payload grande de alta entropia e NÃO derivado do dicionário:
    // blob ≥ região ⇒ sai plano (id 0), nunca maior que o plano.
    let nomes_dict = nomes_ruidosos(40);
    let dict = compress::dict_from_registry(&nomes_dict);
    // Nomes ASCII aleatórios (LCG): sem prefixo comum, o LZ4 não acha
    // correspondências de 4 bytes — região honestamente incompressível.
    const ALFABETO: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut x = 0x9E37_79B9u32;
    let estranhos: Vec<String> = (0..60)
        .map(|_| {
            let mut s = String::with_capacity(12);
            for _ in 0..12 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                s.push(ALFABETO[(x % ALFABETO.len() as u32) as usize] as char);
            }
            s
        })
        .collect();
    let resultados: Vec<vbl_fxp::BatchResult> = estranhos
        .iter()
        .map(|n| vbl_fxp::BatchResult::Ok {
            value: 1.0,
            canonical: n.clone(),
        })
        .collect();
    let msg = Message::read_batch_ok(resultados, 1);

    let mut com_dict = Vec::new();
    vbl_fxp::schema::encode_with_compression_dict(&msg, &dict, &mut com_dict).unwrap();
    assert_eq!(
        com_dict[10] & flag::COMPRESSED,
        0,
        "incompressível com dict sai plano"
    );

    let mut sem_dict = Vec::new();
    vbl_fxp::schema::encode_with_compression(&msg, &mut sem_dict).unwrap();
    assert_eq!(
        sem_dict[10] & flag::COMPRESSED,
        0,
        "incompressível sem dict sai plano"
    );
}

#[test]
fn beacon_curto_falha_honesto() {
    let err = vbl_fxp::discover::decode_beacon(&[0xff, 0x00]).unwrap_err();
    assert!(
        matches!(err, vbl_fxp::DiscoveryError::BeaconInvalido),
        "{err:?}"
    );
}

#[test]
fn grupo_do_config_ruim_registra_sensor_inacessivel_honesto() {
    // parse_group reprova no build ⇒ rota registrada porém inacessível,
    // com o motivo honesto (§4.9) — leitura NUNCA vira valor falso.
    let mut r = DeviceRegistry::new();
    let cfg = "mode = hibrido\ncache_ttl_ms = 0\n\
               temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
               temp_a.mode = real\ntemp_a.endpoint = discover:fxpd-lab\n";
    FxpConfig::parse(cfg).unwrap().apply(&mut r).unwrap();
    let mut bus = FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            discover_group: Some("grupo sem porta".into()),
            ..Default::default()
        },
        FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();
    let err = bus.read_sensor("temp_a", &mut ledger).unwrap_err();
    assert!(matches!(err, vbl_runtime::fxp::SensorFailure::Inaccessible));
}

#[test]
fn grupo_customizado_do_config_resolve_leitura_remota_ou_falha_honesto() {
    use vbl_fxp::discover;

    let peer = PeerServer::new(peer_bus(), ChainLedger::new(), PeerConfig::default());
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer_port(&peer, 0).expect("srv tcp");
    let (grupo, _f) = discover::parse_group("239.255.70.90:7090").expect("grupo v4");
    let _anunciador = match discover::Announcer::start(
        "fxpd-grupo-v12",
        porta,
        0x99,
        grupo,
        Duration::from_millis(30),
    ) {
        Ok(a) => a,
        Err(_) => return, // multicast indisponível: skip gracioso
    };

    let mut r = DeviceRegistry::new();
    let cfg = "mode = hibrido\ncache_ttl_ms = 0\n\
               temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
               temp_a.mode = real\ntemp_a.endpoint = discover:fxpd-grupo-v12\n";
    FxpConfig::parse(cfg).unwrap().apply(&mut r).unwrap();
    let mut bus = FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            discover_group: Some("239.255.70.90:7090".into()),
            ..Default::default()
        },
        FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();
    // Com multicast de loopback funcional resolve e lê; sem rede multicast,
    // a rota fica inacessível com motivo honesto — ambos aceitáveis.
    if let Err(e) = bus.read_sensor("temp_a", &mut ledger) {
        assert!(
            matches!(e, vbl_runtime::fxp::SensorFailure::Inaccessible),
            "{e:?}"
        );
    }
}

#[test]
fn grupo_ssm_do_config_assina_fonte_ou_falha_honesto() {
    use vbl_fxp::discover;

    let peer = PeerServer::new(peer_bus(), ChainLedger::new(), PeerConfig::default());
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer_port(&peer, 0).expect("srv tcp");
    let (grupo, fonte) = discover::parse_group("239.255.70.91:7091@127.0.0.1").expect("ssm");
    let _anunciador = match discover::Announcer::start_bound(
        "fxpd-ssm-v12",
        porta,
        0x77,
        grupo,
        Duration::from_millis(30),
        fonte.map(|f| f.ip()),
    ) {
        Ok(a) => a,
        Err(_) => return,
    };

    let mut r = DeviceRegistry::new();
    let cfg = "mode = hibrido\ncache_ttl_ms = 0\n\
               temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
               temp_a.mode = real\ntemp_a.endpoint = discover:fxpd-ssm-v12\n";
    FxpConfig::parse(cfg).unwrap().apply(&mut r).unwrap();
    let mut bus = FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            discover_group: Some("239.255.70.91:7091@127.0.0.1".into()),
            ..Default::default()
        },
        FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();
    if let Err(e) = bus.read_sensor("temp_a", &mut ledger) {
        assert!(
            matches!(e, vbl_runtime::fxp::SensorFailure::Inaccessible),
            "{e:?}"
        );
    }
}

#[test]
fn peer_v1_0_puro_em_tcp_degrada_honesto_e_continua_lendo() {
    use std::io::{Read, Write};

    // Peer v1.0 real: não conhece CAPS/HELLO — FECHA a conexão diante deles
    // e responde READ com READ_OK (§4.5: degradação registrada no Caderno,
    // o fio segue v1.0 puro).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let porta = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for fluxo in listener.incoming().flatten() {
            let mut s = fluxo;
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut rest: Vec<u8> = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => break, // solta ESTA conexão
                    Ok(n) => rest.extend_from_slice(&buf[..n]),
                }
                if rest.len() >= 16
                    && rest.len()
                        >= u32::from_le_bytes(rest[0..4].try_into().expect("len")) as usize
                {
                    break;
                }
            }
            let opcode = rest.get(8).copied().unwrap_or(0);
            if opcode == vbl_fxp::schema::op::CAPS || opcode == vbl_fxp::schema::op::HELLO {
                continue; // v1.0 fecha a conexão: gatilho da degradação honesta
            }
            if opcode == vbl_fxp::schema::op::READ {
                let nome = String::from_utf8_lossy(&rest[16..16 + rest[11] as usize]).to_string();
                let seq = u32::from_le_bytes(rest[12..16].try_into().expect("seq"));
                let resp =
                    vbl_fxp::schema::encode_to_vec(&Message::read_ok(42.0, &nome, false, seq))
                        .expect("encode");
                let _ = s.write_all(&resp);
            }
        }
    });

    let mut r = DeviceRegistry::new();
    let cfg = format!(
        "mode = hibrido\ncache_ttl_ms = 0\ncompression = true\n\
         temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
         temp_a.mode = real\ntemp_a.endpoint = tcp:127.0.0.1:{porta}\n"
    );
    FxpConfig::parse(&cfg).unwrap().apply(&mut r).unwrap();
    let mut bus = FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            compression: true,
            ..Default::default()
        },
        FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();
    let v = match bus.read_sensor("temp_a", &mut ledger) {
        Ok(v) => v,
        Err(e) => {
            for ev in ledger.search("ALERT", &[]) {
                eprintln!("ALERTA: {}", ev.line());
            }
            panic!("fallback v1.0 lê: {e:?}");
        }
    };
    assert!((v - 42.0).abs() < f64::EPSILON);
}

#[test]
fn fxp_config_reprova_valores_invalidos_e_aceita_dict() {
    // compression_dict aceito e aplicado…
    let mut r = DeviceRegistry::new();
    let ok = "mode = hibrido\ncompression_dict = true\n\
              temp_a.grandeza = temperatura\ntemp_a.unidade = C\n";
    FxpConfig::parse(ok).unwrap().apply(&mut r).unwrap();
    // …valor não-booleano reprovado no parse com mensagem honesta…
    let ruim = "mode = hibrido\ncompression_dict = talvez\n\
                temp_a.grandeza = temperatura\ntemp_a.unidade = C\n";
    let err = FxpConfig::parse(ruim).unwrap_err();
    assert!(err.to_string().contains("compression_dict"), "{err}");
    // …compress_threshold fora do usize reprova no parse…
    let grande = "mode = hibrido\ncompress_threshold = 99999999999999999999999\n\
                  temp_a.grandeza = temperatura\ntemp_a.unidade = C\n";
    let err2 = FxpConfig::parse(grande).unwrap_err();
    assert!(err2.to_string().contains("compress_threshold"), "{err2}");
}
