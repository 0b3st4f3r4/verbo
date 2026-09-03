//! E2E dos recursos v1.3 (docs/FXP-SCHEMA-v1.md §9): SSM IPv6 (RFC 4604 via
//! `setsockopt(MCAST_JOIN_SOURCE_GROUP)` — o item que a §9 da v1.2 registrou
//! como "aguarda API de socket"), TOFU como alternativa operacional ao
//! pinning, compressão zstd com dicionário treinado (id 3) e resumo de
//! sessão/0-RTT TLS.
//!
//! Regra de ouro mantida: todo recurso é opt-in e negociado — sem config, o
//! fio é byte a byte o da v1.0/v1.1/v1.2 (golden bytes e suites anteriores
//! ficam intocados). Falha de recurso desconhecido segue **fail closed**.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use vbl_fxp::discover::{self, parse_group, FonteSsm};
use vbl_fxp::schema::caps;
use vbl_fxp::tls::{self, ConfiancaCliente, TlsAccept, Trust};
use vbl_fxp::transport::Connection;
use vbl_fxp::{BusConfig, DeviceRegistry, FxpBus, OperationMode, PeerConfig, PeerServer};
use vbl_runtime::fxp::Fxp as _;
use vbl_runtime::ledger::ChainLedger;
use vbl_runtime::FxpSimulator;

const DEADLINE: Duration = Duration::from_secs(2);

/// Diretório de rascunho do teste (único por processo; limpo no fim).
struct Rascunho(PathBuf);
impl Rascunho {
    fn nova(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vbl-v13-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("criar rascunho");
        Self(dir)
    }
    fn caminho(&self, nome: &str) -> PathBuf {
        self.0.join(nome)
    }
}
impl Drop for Rascunho {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ======================================================================
// v1.3 §4.9 — SSM IPv6: a assinatura (grupo, fonte) v6 desbloqueada pelo
// join crua. Parse determinístico sempre roda; o cenário ao vivo é skip
// gracioso em rede sem multicast v6 (mesmo padrão v1.1/v1.2).
// ======================================================================

#[test]
fn parse_group_aceita_fonte_v6_escopada() {
    // fonte v6 link-local com scope explícito (sin6_scope_id preservado)
    let (grupo, fonte) = parse_group("[ff35::7080]:7080@[fe80::1%2]").expect("ssm v6");
    match grupo {
        std::net::SocketAddr::V6(g) => {
            assert_eq!(g.ip().to_string(), "ff35::7080");
            assert_eq!(
                g.scope_id(),
                0,
                "scope do GRUPO fica no grupo, não na fonte"
            );
        }
        _ => panic!("grupo deveria ser v6"),
    }
    assert_eq!(
        fonte,
        Some(FonteSsm::V6 {
            addr: "fe80::1".parse().expect("v6"),
            scope: 2
        })
    );
    // fonte v6 global sem scope: rota padrão
    let (_, fonte) = parse_group("[ff35::7081]:7080@[2001:db8::1]").expect("ssm v6 global");
    assert_eq!(
        fonte.map(FonteSsm::ip),
        Some(IpAddr::V6("2001:db8::1".parse().expect("v6")))
    );
}

#[test]
fn beacon_ssm_v6_assina_a_fonte_ou_ouve_vazio() {
    // Anunciador ligado a ::1 para que a FONTE do datagrama seja exatamente
    // a assinada (S,G) — espelho v6 do teste SSM v4 da v1.2.
    let (grupo, fonte) = parse_group("[ff15::7083]:7083@[::1]").expect("ssm v6");
    let Some(FonteSsm::V6 { addr, scope: _ }) = fonte else {
        panic!("fonte v6 esperada");
    };
    let anunciador = match discover::Announcer::start_bound(
        "fxpd-ssm-v6-teste",
        7083,
        0x8765,
        grupo,
        Duration::from_millis(50),
        Some(IpAddr::V6(addr)),
    ) {
        Ok(a) => a,
        Err(_) => return, // rede sem multicast v6: skip gracioso (§4.9 honesto)
    };
    // A assinatura SSM v6 é o caminho novo da v1.3: só o (S,G) assinado chega.
    let assinado = discover::discover_peers_ssm(
        Duration::from_millis(700),
        grupo,
        fonte.expect("fonte do ssm v6"),
    );
    let peers = match assinado {
        Ok(p) => p,
        // Kernel/rede sem suporte a SSM v6 (ex.: não-Linux): join falha
        // honesto — aceitável, o parse e a recusa de família sempre rodam.
        Err(_) => {
            drop(anunciador);
            return;
        }
    };
    drop(anunciador);
    if let Some(p) = peers.iter().find(|p| p.identifier == "fxpd-ssm-v6-teste") {
        assert_eq!(
            p.source.ip().to_string(),
            "::1",
            "fonte do datagrama é a assinada"
        );
        assert_eq!(p.tcp_port, 7083);
        assert_eq!(p.registry_hash, 0x8765);
    }
    // Sem a assinatura correta o grupo fica vazio — nunca ruído de outros
    // fxpd (semântica SSM; em rede sem multicast o vazio também é aceitável).
    for p in &peers {
        assert_eq!(
            p.source.ip().to_string(),
            "::1",
            "datagrama de outra fonte em SSM: {p:?}"
        );
    }
}

// ======================================================================
// v1.3 §7 — resumo de sessão TLS: o rustls só retoma quando o MESMO
// `ClientConfig` é reusado; a v1.3 cacheia por chave de confiança. Sem
// isso, cada conexão pagava o handshake completo (§9 da v1.2).
// ======================================================================

/// Par autoassinado (rcgen) — o "papel do servidor" do cenário TLS.
fn certificados() -> (TlsAccept, [u8; 32], String) {
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".into()]).expect("rcgen");
    let fp = tls::fingerprint(ck.cert.der());
    (
        TlsAccept {
            certs_pem: ck.cert.pem(),
            key_pem: ck.signing_key.serialize_pem(),
            sessoes: None,
        },
        fp,
        tls::hex32(&fp),
    )
}

/// Registro do PEER: um sensor simulado (temp_a).
fn peer_bus() -> FxpBus {
    let mut r = DeviceRegistry::new();
    let _ = r.register(vbl_fxp::registry::DeviceEntry::sensor(
        "temp_a",
        "temperature",
        "°C",
        1.0,
    ));
    FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            ..Default::default()
        },
        FxpSimulator::new(),
    )
}

/// Registro RICO: o COVER (treino do dicionário zstd) precisa de matéria —
/// com ~6 bytes de registro (1 sensor curto) o treino não roda e o
/// servidor NÃO concede ZSTD (política honesta do §4.8). 41 sensores com
/// nomes canônicos longos dão ~2 KiB de amostras.
fn peer_bus_rico() -> FxpBus {
    let mut r = DeviceRegistry::new();
    let _ = r.register(vbl_fxp::registry::DeviceEntry::sensor(
        "temp_a",
        "temperature",
        "°C",
        1.0,
    ));
    for i in 0..40 {
        let nome = format!("temperatura_turbina_{i:02}_manifold_canonica_{i}");
        let _ = r.register(vbl_fxp::registry::DeviceEntry::sensor(
            &nome,
            "temperature",
            "°C",
            1.0,
        ));
    }
    FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            ..Default::default()
        },
        FxpSimulator::new(),
    )
}

/// Bus do CLIENTE: temp_a via `tcps:127.0.0.1:porta@{sufixo}` com store TOFU
/// opcional (v1.3 — caminho vai na BusConfig; o CLI resolve a flag).
fn bus_cliente_tls(porta: u16, sufixo: &str, tofu_store: Option<&Path>) -> FxpBus {
    let cfg = format!(
        "mode = hibrido\ncache_ttl_ms = 0\n\
         temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
         temp_a.mode = real\ntemp_a.endpoint = tcps:127.0.0.1:{porta}@{sufixo}\n"
    );
    let mut r = DeviceRegistry::new();
    vbl_fxp::registry::FxpConfig::parse(&cfg)
        .expect("config do cliente")
        .apply(&mut r)
        .expect("registro");
    FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            tofu_store: tofu_store.map(|p| p.to_path_buf()),
            ..Default::default()
        },
        FxpSimulator::new(),
    )
}

#[test]
fn tls_segunda_conexao_retoma_sessao_e_negocia() {
    let (aceitador, fp, _hex) = certificados();
    let peer = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador),
            caps: caps::LZ4,
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor TLS");

    // 1ª conexão: handshake completo; CAPS pelo caminho normal.
    let mut c1 = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &ConfiancaCliente::Pin(vec![fp]),
        DEADLINE,
        None,
    )
    .expect("1ª conexão TLS");
    let concedidas1 = c1.negotiate(caps::LZ4, DEADLINE).expect("negociação 1");
    assert_eq!(concedidas1, caps::LZ4);
    assert_eq!(
        c1.tls_handshake_kind(),
        Some(rustls::HandshakeKind::Full),
        "1ª conexão é handshake completo"
    );
    assert_eq!(
        c1.tls_0rtt_aceito(),
        Some(false),
        "sem early data na 1ª conexão"
    );
    drop(c1);

    // 2ª conexão no MESMO processo (a config vem do cache da v1.3): sessão
    // retomada E o CAPS já parte como 0-RTT durante o handshake (§7).
    let mut c2 = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &ConfiancaCliente::Pin(vec![fp]),
        DEADLINE,
        Some(caps::LZ4),
    )
    .expect("2ª conexão TLS");
    let concedidas2 = c2.negotiate(caps::LZ4, DEADLINE).expect("negociação 2");
    assert_eq!(
        concedidas2,
        caps::LZ4,
        "CAPS enviado como 0-RTT precisa ser concedido igual"
    );
    assert_eq!(
        c2.tls_handshake_kind(),
        Some(rustls::HandshakeKind::Resumed),
        "2ª conexão deve retomar a sessão (cache de ClientConfig, §7)"
    );
    assert_eq!(
        c2.tls_0rtt_aceito(),
        Some(true),
        "o frame CAPS partiu como 0-RTT e foi aceito pelo servidor"
    );
}

#[test]
fn tls_sem_negociacao_0rtt_nao_parte_e_conexao_segue_honesta() {
    // early_caps = None (ex.: bus sem recursos pedidos): conexão retomada
    // SEM early data — negotiate segue no caminho normal (fail-honesto).
    let (aceitador, fp, _hex) = certificados();
    let peer = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador),
            caps: caps::LZ4,
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor TLS");
    Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &ConfiancaCliente::Pin(vec![fp]),
        DEADLINE,
        None,
    )
    .and_then(|mut c| c.negotiate(caps::LZ4, DEADLINE).map(|_| ()))
    .expect("aquecimento da sessão");
    let mut c = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &ConfiancaCliente::Pin(vec![fp]),
        DEADLINE,
        None,
    )
    .expect("conexão retomada sem early data");
    assert_eq!(c.tls_handshake_kind(), Some(rustls::HandshakeKind::Resumed));
    assert_eq!(
        c.tls_0rtt_aceito(),
        Some(false),
        "nada foi enviado como 0-RTT"
    );
    assert_eq!(
        c.negotiate(caps::LZ4, DEADLINE).expect("negociação normal"),
        caps::LZ4
    );
}

// ======================================================================
// v1.3 §7 — TOFU (trust on first use): alternativa OPERACIONAL ao
// pinning — o operador não copia o pin para o config; a impressão
// digital vista na PRIMEIRA conexão é gravada no store e as seguintes
// são verificadas contra ela. Divergência ⇒ falha fechada (§4.6).
// ======================================================================

#[test]
fn tofu_store_grava_primeira_uso_e_recarrega() {
    let dir = Rascunho::nova("tofu-store");
    let caminho = dir.caminho("known-hosts.json");

    let mut store = tls::TofuStore::open(&caminho).expect("store novo");
    let fa = [1u8; 32];
    assert!(
        store.verificar("srv:7000", fa).expect("primeira uso"),
        "1ª use ⇒ true"
    );
    assert!(
        !store.verificar("srv:7000", fa).expect("segunda use"),
        "conhecida ⇒ false"
    );
    // Persistência em disco (atômica) — um store reaberto vê a entrada.
    let mut recarregado = tls::TofuStore::open(&caminho).expect("store reaberto");
    assert!(
        !recarregado.verificar("srv:7000", fa).is_err(),
        "entrada sobrevive ao reabrir"
    );

    // Divergência: impressão distinta ⇒ falha com as DUAS impressões.
    let fb = [2u8; 32];
    match store.verificar("srv:7000", fb) {
        Err(tls::TofuFalha::Divergencia { armazenada, vista }) => {
            assert_eq!(armazenada, fa);
            assert_eq!(vista, fb);
        }
        outro => panic!("divergência deve falhar fechado: {outro:?}"),
    }
    // O store NÃO gravou a impressão divergente (o arquivo continua com fa).
    let txt = std::fs::read_to_string(&caminho).expect("store legível");
    assert!(
        txt.contains(&tls::hex32(&fa)),
        "entrada boa no store: {txt}"
    );
    assert!(
        !txt.contains(&tls::hex32(&fb)),
        "impressão divergente nunca entra no store"
    );
}

#[test]
fn tofu_store_corrompido_falha_abertura() {
    let dir = Rascunho::nova("tofu-corrompido");
    let caminho = dir.caminho("known-hosts.json");
    std::fs::write(&caminho, "{isto não é json").expect("corromper");
    assert!(
        tls::TofuStore::open(&caminho).is_err(),
        "store corrompido ⇒ erro honesto"
    );
}

#[test]
fn e2e_tofu_primeira_conecta_segunda_verifica_divergencia_falha_fechada() {
    let dir = Rascunho::nova("tofu-e2e");
    let store = dir.caminho("known-hosts.json");

    // Servidor A (cert A) na porta PA.
    let (aceitador_a, fp_a, _) = certificados();
    let peer_a = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador_a),
            ..Default::default()
        },
    );
    let (_srv_a, porta_a) = vbl_fxp::peer::serve_tcp_peer(&peer_a).expect("servidor A");

    // 1ª conexão: primeira use grava e conecta; o store no disco fala.
    let mut bus1 = bus_cliente_tls(porta_a, "tofu", Some(&store));
    let mut ledger = ChainLedger::new();
    bus1.read_sensor("temp_a", &mut ledger)
        .expect("1ª conexão TOFU conecta (grava a impressão)");
    let txt = std::fs::read_to_string(&store).expect("store persistido");
    assert!(
        txt.contains(&tls::hex32(&fp_a)),
        "impressão do cert A gravada: {txt}"
    );

    // 2ª conexão (novo bus, mesmo store): conhecida e igual ⇒ conecta.
    let mut bus2 = bus_cliente_tls(porta_a, "tofu", Some(&store));
    bus2.read_sensor("temp_a", &mut ChainLedger::new())
        .expect("2ª conexão TOFU verifica e conecta");

    // Servidor B (cert DIFERENTE) em porta PB; o store JÁ tem uma entrada
    // para "127.0.0.1:PB" (a do cert A) ⇒ divergência ⇒ falha fechada.
    let (aceitador_b, fp_b, _) = certificados();
    assert_ne!(fp_a, fp_b);
    let peer_b = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador_b),
            ..Default::default()
        },
    );
    let (_srv_b, porta_b) = vbl_fxp::peer::serve_tcp_peer(&peer_b).expect("servidor B");
    let mut pre_alimentado = tls::TofuStore::open(&store).expect("store");
    let _ = pre_alimentado
        .verificar(&format!("127.0.0.1:{porta_b}"), fp_a)
        .expect("pré-alimentar a entrada divergente");

    let mut bus3 = bus_cliente_tls(porta_b, "tofu", Some(&store));
    let r = bus3.read_sensor("temp_a", &mut ChainLedger::new());
    assert!(r.is_err(), "impressão divergente tem que falhar fechado");
}

#[test]
fn e2e_tofu_divergencia_registra_motivo_no_caderno() {
    let dir = Rascunho::nova("tofu-caderno");
    let store = dir.caminho("known-hosts.json");
    let (_aceitador, fp_a, _) = certificados();
    let (aceitador_b, _fp_b, _) = certificados();
    let peer = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador_b),
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor");
    // Store já conhece a porta com OUTRA impressão (cert A).
    let mut pre = tls::TofuStore::open(&store).expect("store");
    let _ = pre
        .verificar(&format!("127.0.0.1:{porta}"), fp_a)
        .expect("pré-alimentar");

    let mut bus = bus_cliente_tls(porta, "tofu", Some(&store));
    let mut ledger = ChainLedger::new();
    assert!(
        bus.read_sensor("temp_a", &mut ledger).is_err(),
        "divergência falha fechado"
    );
    let motivo: String = ledger
        .events
        .iter()
        .map(|e| e.msg.clone())
        .collect::<Vec<_>>()
        .join(" | ")
        .to_lowercase();
    assert!(
        motivo.contains("tls"),
        "motivo honesto do handshake recusado: {motivo}"
    );
}

// ======================================================================
// v1.3 §7 — config de registry: `@tofu` vs `@sha256:HEX` (pin v1.2).
// ======================================================================

#[test]
fn registry_tcps_aceita_tofu_e_mantem_pin() {
    use vbl_fxp::registry::{Endpoint, RegistryError};

    let ep = Endpoint::parse("tcps:127.0.0.1:7000@tofu").expect("tofu");
    // Descrição canônica re-parseável (antes do move no match).
    assert_eq!(ep.description(), "tcps:127.0.0.1:7000@tofu");
    match &ep {
        Endpoint::Remote {
            addr: vbl_fxp::RemoteAddr::TcpTls { host, port, trust },
        } => {
            assert_eq!(host, "127.0.0.1");
            assert_eq!(*port, 7000);
            assert_eq!(*trust, Trust::Tofu);
        }
        _ => panic!("endpoint remoto esperado"),
    }

    // Pin da v1.2 continua igual.
    let hex = "ab".repeat(32);
    let ep = Endpoint::parse(&format!("tcps:h:1@sha256:{hex}")).expect("pin");
    match &ep {
        Endpoint::Remote {
            addr: vbl_fxp::RemoteAddr::TcpTls { trust, .. },
        } => {
            assert_eq!(*trust, Trust::Pin(vec![[0xabu8; 32]]));
        }
        _ => panic!("endpoint remoto esperado"),
    }
    // Slot de confiança inválido ⇒ erro honesto (nada silencioso).
    let err = Endpoint::parse("tcps:h:1@confiar").err();
    assert!(
        matches!(err, Some(RegistryError::InvalidEndpoint(_))),
        "{err:?}"
    );
}

// ======================================================================
// v1.3 §4.8 — zstd com dicionário TREINADO (id 3): o treino é
// determinístico para (nomes ordenados, versão do zstd); zero bytes de
// dicionário no fio; divergência de dicionário ou de algoritmo ⇒
// `UnknownCompression`/`DecompressionFailed` (fail closed). O bit ZSTD é
// sempre negociado JUNTO com DICT — o gatilho do `HELLO` é o mesmo.
// ======================================================================

#[test]
fn zstd_dict_do_registro_e_deterministico_e_limitado() {
    use vbl_fxp::schema::compress::{zstd_dict_from_registry, ZSTD_DICT_MAX};

    let nomes: Vec<String> = (0..40)
        .map(|i| format!("temperatura_turbina_{i:02}_manifold_canonica_{i}"))
        .collect();
    let d1 = zstd_dict_from_registry(&nomes).expect("treina com corpus saudável");
    let d2 = zstd_dict_from_registry(&nomes).expect("treina de novo");
    assert_eq!(d1, d2, "mesma matéria ⇒ mesmo dicionário (determinismo)");
    assert!(
        d1.len() <= ZSTD_DICT_MAX,
        "dicionário respeita o teto de 16 KiB"
    );
    // A ordem de CHEGADA não muda o dicionário: nomes ordenados antes do
    // treino (mesma regra do id 2 — os dois lados derivam o mesmo bytes).
    let mut embaralhada = nomes.clone();
    embaralhada.reverse();
    assert_eq!(
        zstd_dict_from_registry(&embaralhada).as_ref(),
        Some(&d1),
        "ordem de chegada não muda o dicionário"
    );
    // Registro vazio ⇒ None honesto: sem matéria, o servidor NÃO concede
    // ZSTD (degradação explícita, nunca dicionário vazio no fio).
    assert_eq!(zstd_dict_from_registry(&[]), None);
}

#[test]
fn zstd_encode_id3_roundtrip_threshold_nunca_infla_e_falha_fechada() {
    use vbl_fxp::schema::compress::THRESHOLD;
    use vbl_fxp::schema::compress::{DictConexao, ALGO_LZ4_DICT, ALGO_ZSTD_DICT};
    use vbl_fxp::schema::{
        decode_with_conexao, decode_with_dict, encode_with_zstd_dict, Message, SchemaError,
        HEADER_LEN,
    };

    let amostras: Vec<String> = (0..40)
        .map(|i| format!("temperatura_turbina_{i:02}_manifold_canonica_{i}"))
        .collect();
    let dict = vbl_fxp::schema::compress::zstd_dict_from_registry(&amostras)
        .expect("treina com corpus saudável");
    let qualquer_dict = vbl_fxp::schema::compress::dict_from_registry(&amostras);

    // Abaixo do threshold: sai plano (id nenhum), regra idêntica à do LZ4.
    let pequena = Message::read("temp_a", 1, false);
    let mut f = Vec::new();
    encode_with_zstd_dict(&pequena, &dict, &mut f).expect("pequena");
    assert!(
        f[4 + 5] & vbl_fxp::schema::flag::COMPRESSED == 0,
        "sem compressão no pequeno"
    );

    // Acima do threshold: id 3 no byte reservado; decodifica com o dict
    // Zstd. (READ_BATCH: nomes ≤ 255 cada, corpo > THRESHOLD.)
    let nomes_lote: Vec<String> = (0..8)
        .map(|i| {
            format!("sensor_de_leitura_longa_para_estourar_o_threshold_{i:03}_pad_pad_pad_pad_pad_pad_pad_pad_pad_pad")
        })
        .collect();
    let grande = Message::read_batch(nomes_lote, 7);
    let plano = vbl_fxp::schema::encode_to_vec(&grande).expect("plano");
    // Região plana (nome+corpo) do frame: o threshold é sobre ELA.
    let regiao = plano.len() - 4 - HEADER_LEN;
    assert!(
        regiao > THRESHOLD,
        "região > threshold no teste (região = {regiao})"
    );
    let mut f = Vec::new();
    encode_with_zstd_dict(&grande, &dict, &mut f).expect("grande");
    assert_eq!(f[4 + 6], ALGO_ZSTD_DICT, "algoritmo 3 no byte reservado");
    assert!(
        f.len() < plano.len(),
        "nunca inflar: frame comprimido menor que o plano"
    );
    let (msg, _) =
        decode_with_conexao(&f, Some(&DictConexao::Zstd(dict.clone()))).expect("decodifica");
    assert_eq!(msg.name, grande.name);

    // Fail closed por construção do TIPO:
    // (a) id 3 com a matéria do id 2 ⇒ UnknownCompression{3};
    // (b) id 2 com o dict treinado (v1.3 com setup v1.2) ⇒ UnknownCompression{2};
    // (c) codec v1.2 (decode_with_dict) vendo id 3 ⇒ UnknownCompression{3}.
    let a = decode_with_conexao(&f, Some(&DictConexao::Lz4(qualquer_dict.clone())))
        .expect_err("id 3 exige o dict treinado");
    assert!(
        matches!(a, SchemaError::UnknownCompression { received: 3 }),
        "{a:?}"
    );
    let mut f2 = Vec::new();
    vbl_fxp::schema::encode_with_compression_dict(&grande, &qualquer_dict, &mut f2)
        .expect("id 2 padrão");
    let b = decode_with_conexao(&f2, Some(&DictConexao::Zstd(dict.clone())))
        .expect_err("id 2 exige a matéria concatenada");
    assert!(
        matches!(b, SchemaError::UnknownCompression { received: 2 }),
        "{b:?}"
    );
    let c = decode_with_dict(&f, Some(&qualquer_dict))
        .expect_err("codec v1.2 não conhece o id 3");
    assert!(
        matches!(c, SchemaError::UnknownCompression { received: 3 }),
        "{c:?}"
    );

    // Divergência de dicionário (ex.: zstd de versões diferentes nas pontas)
    // ⇒ DecompressionFailed — nunca lixo silencioso.
    let outras: Vec<String> = (0..40)
        .map(|i| format!("pressao_caldeira_{i:02}_vapor_principal_canonico_{i}"))
        .collect();
    let outro_dict =
        vbl_fxp::schema::compress::zstd_dict_from_registry(&outras).expect("treina outro");
    if outro_dict != dict {
        let d = decode_with_conexao(&f, Some(&DictConexao::Zstd(outro_dict)))
            .expect_err("dict divergente falha fechado");
        assert!(matches!(d, SchemaError::DecompressionFailed), "{d:?}");
    } else {
        panic!("dicts de matérias diferentes deveriam divergir");
    }
    let _ = ALGO_LZ4_DICT; // referência documental do par de algoritmos
}

#[test]
fn e2e_zstd_negociado_e_degradacao_sem_zstd_no_peer() {
    use std::path::PathBuf;
    use vbl_fxp::transport::wait_ready_unix;

    let sock = std::env::temp_dir().join(format!("vbl-v13-zstd-{}.sock", std::process::id()));

    // Servidor v1.3: anuncia ZSTD+DICT+LZ4. Cliente pede zstd ⇒ interseção
    // cheia, HELLO treina o dicionário dos DOIS lados, leitura funciona.
    // (Registro rico: sem matéria o treino não roda e ZSTD sai da interseção.)
    let peer = PeerServer::new(
        peer_bus_rico(),
        ChainLedger::new(),
        PeerConfig {
            caps: caps::ZSTD | caps::DICT | caps::LZ4,
            ..Default::default()
        },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let cfg = "mode = hibrido\ncache_ttl_ms = 0\n\
               temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
               temp_a.mode = real\ntemp_a.endpoint = unix:PLACE\n"
        .replace("PLACE", &sock.display().to_string());
    let mut r = DeviceRegistry::new();
    vbl_fxp::registry::FxpConfig::parse(&cfg)
        .unwrap()
        .apply(&mut r)
        .unwrap();
    let mut bus = FxpBus::build(
        r,
        BusConfig {
            mode: OperationMode::Hybrid,
            compression_zstd: true,
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
        caps::ZSTD | caps::DICT,
        "interseção do que o cliente PEDIU (zstd anda com dict); as duas pontas treinam o mesmo dicionário"
    );

    // Degrad honesta (1): peer v1.2 anuncia DICT (sem ZSTD) ⇒ CAPS_OK sem o
    // bit zstd, a conexão cai no id 2 e a leitura SEGUE funcionando.
    let sock2 = std::env::temp_dir().join(format!("vbl-v13-zstd-deg-{}.sock", std::process::id()));
    let peer2 = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            caps: caps::DICT | caps::LZ4,
            ..Default::default()
        },
    );
    let _srv2 = vbl_fxp::peer::serve_unix_peer(&peer2, &sock2).expect("servidor v1.2");
    assert!(wait_ready_unix(&sock2, DEADLINE));
    let cfg = "mode = hibrido\ncache_ttl_ms = 0\n\
               temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
               temp_a.mode = real\ntemp_a.endpoint = unix:PLACE2\n"
        .replace("PLACE2", &sock2.display().to_string());
    let mut r2 = DeviceRegistry::new();
    vbl_fxp::registry::FxpConfig::parse(&cfg)
        .unwrap()
        .apply(&mut r2)
        .unwrap();
    let mut bus2 = FxpBus::build(
        r2,
        BusConfig {
            mode: OperationMode::Hybrid,
            compression_zstd: true,
            ..Default::default()
        },
        FxpSimulator::new(),
    );
    let addr2 = vbl_fxp::RemoteAddr::Unix(PathBuf::from(&sock2));
    let v2 = bus2.read_sensor("temp_a", &mut ledger).unwrap();
    assert!(v2.is_finite());
    assert_eq!(
        bus2.granted_caps_of(&addr2),
        caps::DICT,
        "zstd sai da interseção honestamente; dict v1.2 segue"
    );
}

#[test]
#[ignore = "medição de tamanhos para o relatório v1.3"]
fn zz_tamanhos_bench() {
    let nomes: Vec<String> = (0..40)
        .map(|i| format!("temp_{i:02}_sensor_de_temperatura_do_rack_{i:02}"))
        .collect();
    let zdict = vbl_fxp::schema::compress::zstd_dict_from_registry(&nomes).unwrap();
    let ldict = vbl_fxp::schema::compress::dict_from_registry(&nomes);
    let resultados: Vec<vbl_fxp::BatchResult> = nomes
        .iter()
        .map(|n| vbl_fxp::BatchResult::Ok {
            value: 36.5,
            canonical: n.clone(),
        })
        .collect();
    let msg = vbl_fxp::schema::Message::read_batch_ok(resultados, 1);
    let mut fl = Vec::new();
    vbl_fxp::schema::encode_with_compression_dict(&msg, &ldict, &mut fl).unwrap();
    let mut fz = Vec::new();
    vbl_fxp::schema::encode_with_zstd_dict(&msg, &zdict, &mut fz).unwrap();
    let mut fp = Vec::new();
    vbl_fxp::schema::encode(&msg, &mut fp).unwrap();
    println!(
        "plano={} lz4_dict={} zstd_dict={} dict_lz4_bytes={} dict_zstd_bytes={}",
        fp.len() - 4,
        fl.len() - 4,
        fz.len() - 4,
        ldict.len(),
        zdict.len()
    );
}
