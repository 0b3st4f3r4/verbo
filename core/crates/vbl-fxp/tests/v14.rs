//! E2E dos recursos v1.4 (docs/FXP-SCHEMA-v1.md §9 — a fila deixada pela
//! v1.3): TOFU estrito (exige entrada prévia no store), rotação de pins com
//! sobreposição (`@sha256:H1,H2`), zstd com dicionário VERIFICADO no fio
//! (id 4 + `DICT_SYNC`, para pontas com versões de zstd diferentes
//! degradarem honestamente em vez de falhar decompressão) e sessão
//! retomada entre renascimentos do PEER (cache de sessões em disco). O
//! bench de 0-RTT com RTT real mora em `benches/fxp.rs` (grupo
//! `v14_tls_0rtt_rtt`).
//!
//! Regra de ouro mantida: todo recurso é opt-in e negociado — sem config, o
//! fio é byte a byte o da v1.0/v1.1/v1.2/v1.3 (golden bytes e suites
//! anteriores ficam intocados). Falha de recurso desconhecido: fail closed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use vbl_fxp::schema::compress::DictConexao;
use vbl_fxp::schema::{caps, compress, flag, op, Body, Message};
use vbl_fxp::tls::{self, TofuFalha, TofuStore, TlsAccept, Trust};
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
        let dir = std::env::temp_dir().join(format!("vbl-v14-{}-{tag}", std::process::id()));
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

/// Par autoassinado (rcgen) — o "papel do servidor" do cenário TLS.
fn certificado(tag: &str) -> (TlsAccept, [u8; 32]) {
    let ck = rcgen::generate_simple_self_signed(vec![tag.into()]).expect("rcgen");
    let fp = tls::fingerprint(ck.cert.der());
    (
        TlsAccept {
            certs_pem: ck.cert.pem(),
            key_pem: ck.signing_key.serialize_pem(),
            sessoes: None,
        },
        fp,
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

/// Registro RICO: matéria suficiente para o COVER treinar (mesma política da
/// v1.3 — registro pequeno demais ⇒ o servidor NÃO concede zstd nenhum).
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

// ======================================================================
// v1.4 §4.8 — id 4 (zstd + dicionário VERIFICADO no fio): codec tipado.
// O id 4 decodifica SÓ com `DictConexao::ZstdV` — id 3 com Zstd, id 2 com
// Lz4; qualquer cruzamento é `UnknownCompression` (fail closed por
// construção, mesmo padrão da v1.3).
// ======================================================================

/// Nomes pseudo-aleatórios determinísticos (mesma técnica da v1.2): alta
/// entropia para o codec não achar razão sem o dicionário.
fn nomes_ruidosos(n: usize) -> Vec<String> {
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

fn lote(nomes: &[String], seq: u32) -> Message {
    let resultados: Vec<vbl_fxp::BatchResult> = nomes
        .iter()
        .enumerate()
        .map(|(i, n)| vbl_fxp::BatchResult::Ok {
            value: i as f64 + 0.5,
            canonical: n.clone(),
        })
        .collect();
    Message::read_batch_ok(resultados, seq)
}

#[test]
fn schema_id4_roundtrip_e_fail_closed_tipado() {
    let nomes = nomes_ruidosos(20);
    let dict = compress::dict_from_registry(&nomes);
    let msg = lote(&nomes, 1);

    let mut f4 = Vec::new();
    vbl_fxp::schema::encode_with_zstd_dict_v(&msg, &dict, &mut f4).expect("encode id 4");
    assert!(f4[4 + 5] & flag::COMPRESSED != 0, "frame id 4 vem comprimido");
    assert_eq!(f4[4 + 6], compress::ALGO_ZSTD_DICT_V, "byte reservado = id 4");

    // Com o dicionário VERIFICADO decodifica; com Zstd (id 3) ou Lz4 (id 2)
    // é UnknownCompression{4} — o tipo do dicionário é a autoridade.
    let (msg2, _) =
        vbl_fxp::schema::decode_with_conexao(&f4, Some(&DictConexao::ZstdV(dict.clone())))
            .expect("decode id 4 com ZstdV");
    assert_eq!(msg2, msg);
    for (variante, d) in [
        ("Zstd", DictConexao::Zstd(dict.clone())),
        ("Lz4", DictConexao::Lz4(dict.clone())),
    ] {
        let e = vbl_fxp::schema::decode_with_conexao(&f4, Some(&d))
            .expect_err(&format!("id 4 com dict {variante} falha fechado"));
        assert_eq!(
            e,
            vbl_fxp::schema::SchemaError::UnknownCompression {
                received: compress::ALGO_ZSTD_DICT_V
            }
        );
    }
    // E o inverso: id 3 não decodifica com ZstdV (promoção segura por
    // construção — um frame v1.3 nunca vira id 4 silencioso).
    let mut f3 = Vec::new();
    vbl_fxp::schema::encode_with_zstd_dict(&msg, &dict, &mut f3).expect("encode id 3");
    let e = vbl_fxp::schema::decode_with_conexao(&f3, Some(&DictConexao::ZstdV(dict.clone())))
        .expect_err("id 3 com ZstdV falha fechado");
    assert_eq!(
        e,
        vbl_fxp::schema::SchemaError::UnknownCompression {
            received: compress::ALGO_ZSTD_DICT
        }
    );
}

#[test]
fn hash_dict_e_deterministica_e_distingue_materias() {
    let a = compress::hash_dict(b"cpu_temp\ntemp_a");
    let b = compress::hash_dict(b"cpu_temp\ntemp_b");
    assert_eq!(a, compress::hash_dict(b"cpu_temp\ntemp_a"), "determinística");
    assert_ne!(a, b, "matéria diferente ⇒ hash diferente");
}

// ======================================================================
// v1.4 §4.8 — DICT_SYNC/DICT_SYNC_OK: o par troca (versão do zstd, hash do
// dicionário treinado) após o HELLO; hash igual ⇒ id 4 habilitado nos dois
// sentidos; hash diferente (ex.: versões de zstd distintas) ⇒ degradação
// honesta para o id 2, SEM tentativa de frame que falharia.
// ======================================================================

#[test]
fn dict_sync_roundtrip_no_fio() {
    let msg = Message::dict_sync(105_640_001, [7u8; 32], 3);
    let f = vbl_fxp::schema::encode_to_vec(&msg).expect("encode");
    let (msg2, _) = vbl_fxp::schema::decode(&f).expect("decode");
    assert_eq!(msg2.opcode, op::DICT_SYNC);
    let Body::DictSync {
        zstd_version,
        dict_hash,
    } = msg2.body
    else {
        panic!("corpo DictSync esperado");
    };
    assert_eq!(zstd_version, 105_640_001);
    assert_eq!(dict_hash, [7u8; 32]);

    let ok = Message::dict_sync_ok(105_640_002, [9u8; 32], 4);
    let f = vbl_fxp::schema::encode_to_vec(&ok).expect("encode");
    let (ok2, _) = vbl_fxp::schema::decode(&f).expect("decode");
    assert_eq!(ok2.opcode, op::DICT_SYNC_OK);
    let Body::DictSync { .. } = ok2.body else {
        panic!("corpo DictSync esperado no OK");
    };
}

#[test]
fn e2e_id4_hash_casado_verifica_e_responde_com_id_4() {
    let peer = PeerServer::new(
        peer_bus_rico(),
        ChainLedger::new(),
        PeerConfig {
            // READ_BATCH exige o bit BATCH negociado (§4.7 fail closed).
            caps: caps::DICT | caps::ZSTD | caps::ZSTD_V | caps::BATCH,
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor");

    let mut c = Connection::tcp("127.0.0.1", porta, DEADLINE).expect("conexão");
    let concedidas = c
        .negotiate(caps::DICT | caps::ZSTD | caps::ZSTD_V | caps::BATCH, DEADLINE)
        .expect("negociação");
    assert_eq!(
        concedidas,
        caps::DICT | caps::ZSTD | caps::ZSTD_V | caps::BATCH
    );

    let remoto = c.exchange_hello(&[], DEADLINE).expect("hello");
    let nomes: Vec<String> = remoto.iter().map(|d| d.name().to_string()).collect();
    let treinado = compress::zstd_dict_from_registry(&nomes).expect("treina");
    let hash = compress::hash_dict(&treinado);

    let (versao_peer, hash_peer) = c
        .dict_sync(compress::zstd_version(), hash, DEADLINE)
        .expect("sync");
    assert_eq!(hash_peer, hash, "mesma matéria + mesmo zstd ⇒ mesmo dict");
    assert!(versao_peer > 0, "versão do zstd do peer vai no fio");
    // O hash casado libera o id 4 — quem chama instala (o bus faz isso no
    // fluxo real; aqui espelhamos a decisão).
    assert_eq!(hash_peer, hash);
    c.set_zstd_dict_v(treinado);
    assert!(c.dict_verificado(), "hash casado ⇒ id 4 habilitado");

    // Resposta grande (acima do threshold) DEVE vir com id 4: o cliente só
    // decodifica id 4 depois de verificado — se o servidor mandasse id 2/3,
    // a leitura abaixo falharia com UnknownCompression.
    let nomes_lote: Vec<String> = (0..40)
        .map(|i| format!("temperatura_turbina_{i:02}_manifold_canonica_{i}"))
        .collect();
    let resp = c
        .request(&Message::read_batch(nomes_lote.clone(), 9), DEADLINE)
        .expect("lote grande sobre id 4");
    let Body::ReadBatchOk { results } = resp.body else {
        panic!("lote esperado");
    };
    assert_eq!(results.len(), nomes_lote.len());
    for (r, n) in results.iter().zip(&nomes_lote) {
        match r {
            vbl_fxp::BatchResult::Ok { canonical, .. } => assert_eq!(canonical, n),
            outro => panic!("item com falha no lote id 4: {outro:?}"),
        }
    }
}

#[test]
fn e2e_id4_hash_divergente_degrada_sem_usar_id_4() {
    // Servidor artesanal: concede DICT|ZSTD|ZSTD_V, responde o HELLO com um
    // registro — mas o hash devolvido no DICT_SYNC_OK vem de OUTRA matéria,
    // simulando a ponta com versão de zstd diferente (mesma entrada, dict
    // treinado diferente). O cliente NÃO instala id 4 e a conexão segue
    // honesta — sem DecompressionFailed, sem silêncio.
    let nomes_hello = nomes_ruidosos(24);
    let outra_materia = nomes_ruidosos(30); // matéria do "zstd diferente"

    use std::sync::{Arc, Mutex as Mx};
    let hash_peer = Arc::new(Mx::new(compress::hash_dict(
        &compress::zstd_dict_from_registry(&outra_materia).expect("treina a outra matéria"),
    )));
    let handler_peer = hash_peer.clone();
    let handler = move |msg: Message| -> Option<Message> {
        match msg.opcode {
            op::CAPS => {
                let Body::Caps { capabilities } = msg.body else {
                    return None;
                };
                Some(Message::caps_ok(
                    capabilities & (caps::DICT | caps::ZSTD | caps::ZSTD_V),
                    msg.seq,
                ))
            }
            op::HELLO => {
                let dev: Vec<vbl_fxp::schema::DeviceDesc> = nomes_hello
                    .iter()
                    .map(|n| vbl_fxp::schema::DeviceDesc::Sensor {
                        name: n.clone(),
                        min: None,
                        max: None,
                        quantity: "temperature".into(),
                        unit: "°C".into(),
                        precision_pct: 0.0,
                    })
                    .collect();
                Some(Message::hello(dev, msg.seq))
            }
            op::DICT_SYNC => {
                let meu_hash = *handler_peer.lock().ok()?;
                Some(Message::dict_sync_ok(compress::zstd_version(), meu_hash, msg.seq))
            }
            op::HEARTBEAT => Some(Message::heartbeat_ack(true, msg.seq)),
            _ => None,
        }
    };
    let (_srv, porta) = vbl_fxp::transport::serve_tcp(handler).expect("servidor artesanal");

    let mut c = Connection::tcp("127.0.0.1", porta, DEADLINE).expect("conexão");
    let concedidas = c
        .negotiate(caps::DICT | caps::ZSTD | caps::ZSTD_V, DEADLINE)
        .expect("negociação");
    assert_eq!(concedidas, caps::DICT | caps::ZSTD | caps::ZSTD_V);
    let remoto = c.exchange_hello(&[], DEADLINE).expect("hello");
    let nomes: Vec<String> = remoto.iter().map(|d| d.name().to_string()).collect();
    let treinado_cli = compress::zstd_dict_from_registry(&nomes).expect("treina");
    let hash_cli = compress::hash_dict(&treinado_cli);

    let (_versao_peer, hash_peer) = c
        .dict_sync(compress::zstd_version(), hash_cli, DEADLINE)
        .expect("sync respondido");
    assert_ne!(
        hash_peer, hash_cli,
        "cenário: a ponta deriva um dict treinado diferente"
    );
    assert!(
        !c.dict_verificado(),
        "hash divergente ⇒ id 4 NUNCA é usado"
    );

    // A conexão segue honesta: um HEARTBEAT ainda tem resposta (o caminho
    // daqui em diante é id 2/plano — o id 4 jamais entra no fio).
    let hb = c
        .request(&Message::heartbeat("temp_a", 13), DEADLINE)
        .expect("conexão segue viva após divergência");
    assert_eq!(hb.opcode, op::HEARTBEAT_ACK);
}

// ======================================================================
// v1.4 §4.8 — bit 5 (`ZSTD_V`): promoção de bit reservado com a mesma
// mecânica das versões anteriores — quem não pede não recebe; peers v1.3
// o tratam como reservado (ignorado no decode ⇒ interseção sem ele).
// ======================================================================

#[test]
fn caps_bit5_reservados_viram_6_a_15() {
    assert_eq!(caps::ZSTD_V, 1 << 5);
    // Encode aceita bit 5 limpo e recusa bit 6+ (reservados v1.4).
    let mut ok = Vec::new();
    vbl_fxp::schema::encode(&Message::caps(caps::ZSTD_V, 1), &mut ok).expect("bit 5 encoda");
    let mut ruim = Vec::new();
    let e = vbl_fxp::schema::encode(&Message::caps(1 << 6, 1), &mut ruim)
        .expect_err("bit 6 é reservado na v1.4");
    assert_eq!(e, vbl_fxp::schema::SchemaError::ReservedCaps);
}

#[test]
fn e2e_bit5_nao_concedido_por_peer_v13_e_caminho_id3_permanece() {
    // Servidor SEM ZSTD_V anunciado (v1.3 exato): a interseção não traz o
    // bit; o cliente usa o caminho id 3 da v1.3 (set_zstd_dict) e a
    // conexão funciona — nada da v1.4 liga sem o peer anunciar.
    let peer = PeerServer::new(
        peer_bus_rico(),
        ChainLedger::new(),
        PeerConfig {
            caps: caps::DICT | caps::ZSTD | caps::BATCH, // v1.3 exato
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor");
    let mut c = Connection::tcp("127.0.0.1", porta, DEADLINE).expect("conexão");
    let concedidas = c
        .negotiate(caps::DICT | caps::ZSTD | caps::ZSTD_V | caps::BATCH, DEADLINE)
        .expect("negociação");
    assert_eq!(
        concedidas,
        caps::DICT | caps::ZSTD | caps::BATCH,
        "peer v1.3 não concede o bit 5"
    );
    let remoto = c.exchange_hello(&[], DEADLINE).expect("hello");
    let nomes: Vec<String> = remoto.iter().map(|d| d.name().to_string()).collect();
    let treinado = compress::zstd_dict_from_registry(&nomes).expect("treina");
    c.set_zstd_dict(treinado);
    assert!(!c.dict_verificado(), "caminho v1.3 não verifica dict");
    assert!(c.dict_ready(), "dict id 3 pronto como na v1.3");

    // Lote grande: o servidor responde com id 3 (v1.3 intocado) e o
    // cliente com dict Zstd decodifica.
    let nomes_lote: Vec<String> = (0..40)
        .map(|i| format!("temperatura_turbina_{i:02}_manifold_canonica_{i}"))
        .collect();
    let resp = c
        .request(&Message::read_batch(nomes_lote.clone(), 9), DEADLINE)
        .expect("lote sobre id 3");
    let Body::ReadBatchOk { results } = resp.body else {
        panic!("lote esperado");
    };
    assert_eq!(results.len(), nomes_lote.len());
}

// ======================================================================
// v1.4 §7 — rotação de pins com sobreposição: `@sha256:H1,H2` aceita
// qualquer um dos pins declarados (janela de rotação: novo pin entra
// ANTES do certificado trocar; pin velho sai DEPOIS).
// ======================================================================

#[test]
fn endpoint_tcps_aceita_multiplos_pins_e_descreve_reparseavel() {
    let a = tls::hex32(&[1u8; 32]);
    let b = tls::hex32(&[2u8; 32]);
    let ep =
        vbl_fxp::registry::Endpoint::parse(&format!("tcps:h:1@sha256:{a},{b}")).expect("dois pins");
    let vbl_fxp::registry::Endpoint::Remote {
        addr: vbl_fxp::registry::RemoteAddr::TcpTls { trust, .. },
    } = &ep
    else {
        panic!("tcps esperado");
    };
    match trust {
        Trust::Pin(pins) => assert_eq!(pins, &vec![[1u8; 32], [2u8; 32]]),
        outro => panic!("Trust::Pin esperado, veio {outro:?}"),
    }
    let ep1 = vbl_fxp::registry::Endpoint::parse(&format!("tcps:h:1@sha256:{a}")).expect("1 pin");
    assert_eq!(ep.description(), format!("tcps:h:1@sha256:{a},{b}"));
    assert_eq!(
        ep1.description(),
        format!("tcps:h:1@sha256:{a}"),
        "pin único descreve como na v1.2"
    );
}

#[test]
fn endpoint_tcps_recusa_pins_malformados_honesto() {
    let a = tls::hex32(&[1u8; 32]);
    let casos = [
        format!("tcps:h:1@sha256:{a},"),        // vírgula final ⇒ item vazio
        format!("tcps:h:1@sha256:{a},xyz"),     // segundo pin não-hex
        format!("tcps:h:1@sha256:{a},sha256:zz"), // hex inválido
        "tcps:h:1@sha256:".to_string(),          // vazio
        format!("tcps:h:1@sha256:{a},sha256:{}", "ab".repeat(31)), // 62 dígitos
    ];
    for ruim in casos {
        assert!(
            vbl_fxp::registry::Endpoint::parse(&ruim).is_err(),
            "deveria recusar {ruim}"
        );
    }
}

#[test]
fn endpoint_tcps_aceita_tofu_estrito_e_descreve() {
    let ep = vbl_fxp::registry::Endpoint::parse("tcps:h:1@tofu-estrito").expect("estrito");
    let vbl_fxp::registry::Endpoint::Remote {
        addr: vbl_fxp::registry::RemoteAddr::TcpTls { trust, .. },
    } = &ep
    else {
        panic!("tcps esperado");
    };
    assert_eq!(*trust, Trust::TofuEstrito);
    assert_eq!(ep.description(), "tcps:h:1@tofu-estrito");
}

// ======================================================================
// v1.4 §7 — TOFU estrito: exige entrada PRÉVIA no store (o store vira
// allow-list operacional); desconhecido ⇒ falha fechada com motivo novo
// (`Desconhecida`). A entrada aceita qualquer um dos pins (rotação).
// ======================================================================

#[test]
fn store_tofu_formato_novo_legado_e_misto_carregam() {
    let dir = Rascunho::nova("store-formatos");
    let caminho = dir.caminho("store.json");
    let a = tls::hex32(&[1u8; 32]);
    let b = tls::hex32(&[2u8; 32]);
    // Formato v1.3 (string única) + v1.4 (objeto multi-pin) no MESMO arquivo.
    std::fs::write(
        &caminho,
        format!(
            r#"{{"legado:1":"sha256:{a}","rotativo:1":{{"pins":["sha256:{a}","sha256:{b}"]}}}}"#
        ),
    )
    .expect("escrever store");
    let store = TofuStore::open(&caminho).expect("formatos mistos carregam");
    assert_eq!(store.pins_de("legado:1"), Some(&[[1u8; 32]][..]));
    assert_eq!(
        store.pins_de("rotativo:1"),
        Some(&[[1u8; 32], [2u8; 32]][..])
    );
}

#[test]
fn store_estrito_sem_entrada_falha_fechada_com_motivo() {
    let dir = Rascunho::nova("store-estrito");
    let mut store = TofuStore::open(&dir.caminho("store.json")).expect("store vazio");
    let fp = [3u8; 32];
    match store.verificar_estrito("srv:1", fp) {
        Err(TofuFalha::Desconhecida { alvo }) => assert_eq!(alvo, "srv:1"),
        outro => panic!("estrito sem entrada deve falhar fechado: {outro:?}"),
    }
    // Estrito NÃO grava a primeira use (diferença central do modo).
    assert!(
        store.pins_de("srv:1").is_none(),
        "estrito nunca aprende sozinho"
    );
}

#[test]
fn store_estrito_aceita_qualquer_pin_da_entrada_e_recusa_terceiro() {
    let dir = Rascunho::nova("store-estrito-multi");
    let caminho = dir.caminho("store.json");
    let mut store = TofuStore::open(&caminho).expect("store");
    let a = [1u8; 32];
    let b = [2u8; 32];
    let c = [3u8; 32];
    store.adicionar_pin("srv:1", a).expect("adiciona a");
    store.adicionar_pin("srv:1", b).expect("adiciona b");
    assert!(!store.verificar_estrito("srv:1", a).expect("a ok"));
    assert!(!store.verificar_estrito("srv:1", b).expect("b ok (sobreposição)"));
    match store.verificar_estrito("srv:1", c) {
        Err(TofuFalha::Divergencia { armazenada, vista }) => {
            assert_eq!(vista, c);
            assert!(armazenada == a || armazenada == b, "armazenada é uma das pins");
        }
        outro => panic!("pin fora da entrada falha fechado: {outro:?}"),
    }
    // Rotação completa: remove o pin velho, o novo segue aceito.
    assert!(store.remover_pin("srv:1", a).expect("remove a"));
    assert!(!store.verificar_estrito("srv:1", b).expect("b segue ok"));
    assert!(store.verificar_estrito("srv:1", a).is_err(), "a saiu");
    // Persistência: reabrir vê a rotação.
    let recarregado = TofuStore::open(&caminho).expect("reabre");
    assert_eq!(recarregado.pins_de("srv:1"), Some(&[b][..]));
}

#[test]
fn store_aprendizagem_v13_segue_intacta_com_multiplas_pins() {
    // Modo aprendiz (@tofu v1.3): primeira use grava; a entrada pode ganhar
    // pin novo via rotação operacional (adicionar_pin) e o modo segue
    // verificando qualquer uma — divergência de pin desconhecido falha.
    let dir = Rascunho::nova("store-aprendiz");
    let mut store = TofuStore::open(&dir.caminho("store.json")).expect("store");
    let a = [1u8; 32];
    let b = [2u8; 32];
    assert!(store.verificar("srv:1", a).expect("primeira use"), "grava");
    assert!(!store.verificar("srv:1", a).expect("segunda"), "conhece");
    assert!(store.verificar("srv:1", b).is_err(), "ainda não conhece b");
    store.adicionar_pin("srv:1", b).expect("rotação: b entra");
    assert!(!store.verificar("srv:1", b).expect("b aceito"), "sobreposição");
    assert!(store.verificar("srv:1", [9u8; 32]).is_err(), "fora falha");
}

// ======================================================================
// v1.4 §7 — E2E de rotação de certificado do servidor com sobreposição:
// o cliente com pins {velho, novo} atravessa a troca de certificado sem
// mudar config; o cliente com só o pin velho falha fechado.
// ======================================================================

#[test]
fn e2e_rotacao_de_certificado_com_sobreposicao_de_pins() {
    let (aceitador_a, fp_a) = certificado("localhost");
    let (aceitador_b, fp_b) = certificado("localhost");
    assert_ne!(fp_a, fp_b);

    // Fase 1: servidor com cert A na porta FIXA.
    let peer_a = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador_a),
            ..Default::default()
        },
    );
    let (srv_a, porta) = vbl_fxp::peer::serve_tcp_peer(&peer_a).expect("servidor A");

    // Cliente com pin SOLO do A: conecta na fase 1.
    let mut c1 = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &tls::ConfiancaCliente::Pin(vec![fp_a]),
        DEADLINE,
        None,
    )
    .expect("pin solo A na fase 1");
    c1.negotiate(caps::LZ4, DEADLINE).expect("negocia");
    drop(c1);

    // Cliente com sobreposição {A, B} (o operador preparou a rotação):
    // também conecta.
    let mut c2 = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &tls::ConfiancaCliente::Pin(vec![fp_a, fp_b]),
        DEADLINE,
        None,
    )
    .expect("pins A,B na fase 1");
    c2.negotiate(caps::LZ4, DEADLINE).expect("negocia");
    drop(c2);
    srv_a.parar();

    // Fase 2: MESMA porta, cert B (a rotação aconteceu).
    let peer_b = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador_b),
            ..Default::default()
        },
    );
    let (srv_b, porta_b) =
        vbl_fxp::peer::serve_tcp_peer_port(&peer_b, porta).expect("servidor B");
    assert_eq!(porta_b, porta, "a rotação preserva o endpoint");

    // Sobreposição {A, B}: conecta (é o pin B no fio).
    let mut c3 = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &tls::ConfiancaCliente::Pin(vec![fp_a, fp_b]),
        DEADLINE,
        None,
    )
    .expect("pins A,B atravessam a rotação");
    c3.negotiate(caps::LZ4, DEADLINE).expect("negocia");
    drop(c3);

    // Pin solo velho: falha fechada (nunca texto plano, nunca tolerância).
    let e = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &tls::ConfiancaCliente::Pin(vec![fp_a]),
        DEADLINE,
        None,
    )
    .expect_err("pin velho solo deve falhar contra cert novo");
    assert!(matches!(
        e,
        vbl_fxp::transport::TransportError::ConnectionFailed(_)
    ));

    // Pin solo NOVO: conecta (fim da rotação — operador removeu o velho).
    let mut c4 = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &tls::ConfiancaCliente::Pin(vec![fp_b]),
        DEADLINE,
        None,
    )
    .expect("pin novo solo na fase 2");
    c4.negotiate(caps::LZ4, DEADLINE).expect("negocia");
    drop(c4);
    srv_b.parar();
}

// ======================================================================
// v1.4 §7 — E2E de TOFU estrito contra servidor vivo via BUS: store pré-
// semeado ⇒ conecta; store sem a entrada ⇒ falha fechada (a leitura nunca
// vira valor).
// ======================================================================

fn bus_cliente(porta: u16, sufixo: &str, tofu_store: Option<&Path>) -> FxpBus {
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

/// Bus cliente v1.4: pede ZSTD_V (id 4) além do par v1.3.
fn bus_cliente_zstd_v(porta: u16, tls: bool) -> FxpBus {
    let endpoint = if tls {
        format!("tcps:127.0.0.1:{porta}@tofu-estrito")
    } else {
        format!("tcp:127.0.0.1:{porta}")
    };
    let cfg = format!(
        "mode = hibrido\ncache_ttl_ms = 0\n\
         temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
         temp_a.mode = real\ntemp_a.endpoint = {endpoint}\n"
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
            compression_zstd_v: true,
            batch_prefetch: true,
            ..Default::default()
        },
        FxpSimulator::new(),
    )
}

#[test]
fn e2e_bus_zstd_v_completa_dict_sync_e_conexao_fica_de_pe() {
    // Fluxo real do `vbl run --zstd-v`: o bus pede DICT|ZSTD|ZSTD_V, faz o
    // DICT_SYNC (hash casado — mesmo zstd no processo) e a conexão segue
    // viva com o id 4 liberado (quem observa os frames id 4 em si é o
    // e2e_id4_hash_casado, no nível do transporte).
    let peer = PeerServer::new(
        peer_bus_rico(),
        ChainLedger::new(),
        PeerConfig {
            caps: caps::DICT | caps::ZSTD | caps::ZSTD_V | caps::BATCH,
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor");

    let mut bus = bus_cliente_zstd_v(porta, false);
    let _ = bus
        .read_sensor("temp_a", &mut ChainLedger::new())
        .expect("primeira leitura (handshake completo com DICT_SYNC)");
    let _ = bus
        .read_sensor("temp_a", &mut ChainLedger::new())
        .expect("segunda leitura (conexão retomada)");
}

#[test]
fn e2e_tofu_estrito_sem_entrada_falha_fechada_e_com_entrada_conecta() {
    let (aceitador, fp) = certificado("localhost");
    let peer = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(aceitador),
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer(&peer).expect("servidor");

    let dir = Rascunho::nova("tofu-estrito-e2e");

    // Sem entrada no store: o bus falha a conexão (falha fechada honesta —
    // o erro sobe como rota inacessível com motivo; a leitura nunca vira 0).
    let store_vazio = dir.caminho("store-vazio.json");
    let mut bus = bus_cliente(porta, "tofu-estrito", Some(&store_vazio));
    let leitura = bus.read_sensor("temp_a", &mut ChainLedger::new());
    assert!(leitura.is_err(), "estrito sem entrada não conecta");

    // Pré-semeadura OPERACIONAL (o dono do endpoint registra o pin) — a
    // confiança nasce declarada, nunca aprendida. Store de chave DISTINTA:
    // a config cliente é cacheada por (host:porta, caminho do store) dentro
    // do processo (§7) — a semente em disco vale para o novo store, exatamente
    // como valeria para o próximo processo do operador.
    let store_semeado = dir.caminho("store-semeado.json");
    {
        let mut store = TofuStore::open(&store_semeado).expect("abre store");
        store
            .adicionar_pin(&format!("127.0.0.1:{porta}"), fp)
            .expect("semeia");
    }

    // Com entrada: conecta e lê.
    let mut bus2 = bus_cliente(porta, "tofu-estrito", Some(&store_semeado));
    let _valor = bus2
        .read_sensor("temp_a", &mut ChainLedger::new())
        .expect("estrito com entrada conecta");
}

// ======================================================================
// v1.4 §7 — cache de sessões TLS em DISCO no servidor: a sessão sobrevive
// ao renascimento do PEER (daemon reinicia, cliente vivo retoma com o
// ticket que já tinha). 0-RTT segue funcionando (storage stateful é o que
// habilita early data no rustls — ticketer stateless desligaria o 0-RTT).
// ======================================================================

#[test]
fn e2e_sessao_retomada_entre_renascimentos_do_peer() {
    let dir = Rascunho::nova("sessoes-disco");
    let store_path = dir.caminho("sessoes.json");

    let (aceitador, fp) = certificado("localhost");

    // --- Vida 1: handshake completo; o cliente guarda o ticket (config
    // cacheada — simula o processo cliente VIVO). ----------------------
    let peer1 = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(TlsAccept {
                sessoes: Some(store_path.clone()),
                ..aceitador.clone()
            }),
            ..Default::default()
        },
    );
    let (srv1, porta) = vbl_fxp::peer::serve_tcp_peer(&peer1).expect("servidor 1");
    let mut c1 = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &tls::ConfiancaCliente::Pin(vec![fp]),
        DEADLINE,
        None,
    )
    .expect("1ª conexão");
    assert_eq!(
        c1.tls_handshake_kind(),
        Some(rustls::HandshakeKind::Full),
        "1ª vida ⇒ handshake completo"
    );
    c1.negotiate(caps::LZ4, DEADLINE).expect("negocia");
    drop(c1);
    srv1.parar();

    // --- Vida 2: NOVO ServerConfig/PeerServer (o "processo" renasceu), o
    // MESMO arquivo de sessões; o cliente (config viva) retoma. --------
    let peer2 = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(TlsAccept {
                sessoes: Some(store_path.clone()),
                ..aceitador
            }),
            ..Default::default()
        },
    );
    let (srv2, porta2) =
        vbl_fxp::peer::serve_tcp_peer_port(&peer2, porta).expect("servidor 2");
    assert_eq!(porta2, porta);
    let mut c2 = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &tls::ConfiancaCliente::Pin(vec![fp]),
        DEADLINE,
        Some(caps::LZ4),
    )
    .expect("conexão contra peer renascido");
    assert_eq!(
        c2.tls_handshake_kind(),
        Some(rustls::HandshakeKind::Resumed),
        "ticket sobreviveu ao renascimento do peer (cache em disco)"
    );
    assert_eq!(
        c2.tls_0rtt_aceito(),
        Some(true),
        "0-RTT segue ativo com storage stateful em disco"
    );
    c2.negotiate(caps::LZ4, DEADLINE).expect("negocia");
    drop(c2);
    srv2.parar();
}

#[test]
fn cache_sessoes_disco_put_take_persistencia_e_limite() {
    use rustls::server::StoresServerSessions as _;
    use vbl_fxp::sessoes::CacheSessoesDisco;

    let dir = Rascunho::nova("cache-sessoes");
    let caminho = dir.caminho("sessoes.json");

    let cache = CacheSessoesDisco::open(&caminho, 4).expect("cache novo");
    assert!(cache.can_cache(), "cache anuncia que sabe cachear");
    assert!(cache.put(b"k1".to_vec(), b"blob-1".to_vec()), "put grava");
    assert_eq!(cache.get(b"k1"), Some(b"blob-1".to_vec()));
    // take consome (semântica rustls — sessão retomada é single-use).
    assert_eq!(cache.take(b"k1"), Some(b"blob-1".to_vec()));
    assert_eq!(cache.get(b"k1"), None, "take remove");

    // Persistência: o put abaixo sobrevive ao reabrir.
    assert!(cache.put(b"k2".to_vec(), b"blob-2".to_vec()));
    assert_eq!(cache.get(b"k2"), Some(b"blob-2".to_vec()));
    drop(cache);
    let recarregado = CacheSessoesDisco::open(&caminho, 4).expect("reabre");
    assert_eq!(
        recarregado.get(b"k2"),
        Some(b"blob-2".to_vec()),
        "sessão sobrevive ao processo"
    );

    // Limite: além do teto, o mais VELHO sai (o mais novo fica).
    for i in 0..6u8 {
        assert!(recarregado.put(format!("k{i}").into_bytes(), vec![i; 16]));
    }
    assert_eq!(recarregado.get(b"k0"), None, "mais velho evictado");
    assert_eq!(
        recarregado.get(b"k5"),
        Some(vec![5u8; 16]),
        "mais novo fica"
    );
    drop(recarregado);

    // Corrompido ⇒ erro honesto (nunca lixo parcial — padrão do TofuStore).
    std::fs::write(&caminho, "{corrompido").expect("corromper");
    assert!(
        CacheSessoesDisco::open(&caminho, 4).is_err(),
        "store corrompido falha abertura"
    );
}

#[test]
fn servidor_com_store_de_sessoes_corrompido_falha_arranque() {
    let dir = Rascunho::nova("sessoes-corrompido");
    let caminho = dir.caminho("sessoes.json");
    std::fs::write(&caminho, "{não sou json").expect("corromper");
    let (aceitador, _fp) = certificado("localhost");
    let peer = PeerServer::new(
        peer_bus(),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(TlsAccept {
                sessoes: Some(caminho),
                ..aceitador
            }),
            ..Default::default()
        },
    );
    let e = vbl_fxp::peer::serve_tcp_peer(&peer)
        .err()
        .expect("arranque com store corrompido deve falhar honesto");
    assert!(matches!(
        e,
        vbl_fxp::transport::TransportError::ConnectionFailed(_)
    ));
}

// ── v1.4 — lacunas honestas de cobertura: erros de estado do transporte,
// cache em disco (Debug), store padrão e cláusulas do registro. ──────────

/// Servidor TCP artesanal que responde QUALQUER pedido com a mensagem
/// escolhida — exercita os braços de erro da máquina de estados do
/// cliente (§4.4/§4.5/§4.8 fail-closed).
fn servidor_maluco(resposta: Message) -> (vbl_fxp::transport::Server, u16) {
    vbl_fxp::transport::serve_tcp(move |_pedido| Some(resposta.clone()))
        .expect("servidor artesanal")
}

#[test]
fn negotiate_com_seq_errado_falha_fechada() {
    // §4.5: resposta à negociação com seq trocado ⇒ conexão quebrada
    // (nunca seguir com estado dessincronizado).
    let (_srv, porta) = servidor_maluco(Message::caps_ok(caps::LZ4, 99));
    let mut c = Connection::tcp("127.0.0.1", porta, DEADLINE).expect("conexão");
    let e = c
        .negotiate(caps::LZ4, DEADLINE)
        .expect_err("seq errado deve falhar");
    assert!(
        matches!(&e, vbl_fxp::transport::TransportError::Broken(m) if m.contains("seq")),
        "{e:?}"
    );
}

#[test]
fn negotiate_com_resposta_errada_falha_fechada() {
    // §4.5: resposta que não é CAPS_OK ⇒ erro honesto.
    let (_srv, porta) = servidor_maluco(Message::heartbeat_ack(true, 1));
    let mut c = Connection::tcp("127.0.0.1", porta, DEADLINE).expect("conexão");
    let e = c
        .negotiate(caps::LZ4, DEADLINE)
        .expect_err("resposta errada deve falhar");
    assert!(
        matches!(&e, vbl_fxp::transport::TransportError::Broken(m) if m.contains("CAPS_OK")),
        "{e:?}"
    );
}

#[test]
fn dict_sync_com_resposta_errada_falha_fechada() {
    // v1.4 §4.8: resposta ao DICT_SYNC que não é DICT_SYNC_OK ⇒ quebrada.
    let (_srv, porta) = servidor_maluco(Message::heartbeat_ack(true, 1));
    let mut c = Connection::tcp("127.0.0.1", porta, DEADLINE).expect("conexão");
    let e = c
        .dict_sync(compress::zstd_version(), [0u8; 32], DEADLINE)
        .expect_err("resposta errada ao DICT_SYNC deve falhar");
    assert!(
        matches!(&e, vbl_fxp::transport::TransportError::Broken(m) if m.contains("DICT_SYNC_OK")),
        "{e:?}"
    );
}

#[test]
fn dict_sync_com_corpo_invalido_falha_fechada() {
    // v1.4 §4.8: opcode DICT_SYNC_OK com corpo de outra mensagem — o codec
    // tipado já recusa no DECODE (opcode×corpo são um par canônico no fio);
    // a conexão falha fechada sem nunca instalar dicionário. O braço de
    // corpo inválido da máquina de estados é defesa em profundidade.
    let corpo_errado = Message {
        opcode: op::DICT_SYNC_OK,
        flags: 0,
        seq: 1,
        name: String::new(),
        timestamp_us: None,
        body: Body::HeartbeatAck { ok: true },
    };
    let (_srv, porta) = servidor_maluco(corpo_errado);
    let mut c = Connection::tcp("127.0.0.1", porta, DEADLINE).expect("conexão");
    let e = c
        .dict_sync(compress::zstd_version(), [0u8; 32], DEADLINE)
        .expect_err("corpo inválido deve falhar");
    assert!(
        matches!(
            &e,
            vbl_fxp::transport::TransportError::Schema(
                vbl_fxp::schema::SchemaError::MissingField
            )
        ),
        "{e:?}"
    );
}

#[test]
fn hello_com_resposta_errada_falha_fechada() {
    // §4.4: resposta ao HELLO com corpo de outra mensagem ⇒ erro honesto.
    let (_srv, porta) = servidor_maluco(Message::heartbeat_ack(true, 1));
    let mut c = Connection::tcp("127.0.0.1", porta, DEADLINE).expect("conexão");
    let e = c
        .exchange_hello(&[], DEADLINE)
        .expect_err("resposta errada ao HELLO deve falhar");
    assert!(
        matches!(&e, vbl_fxp::transport::TransportError::Broken(m) if m.contains("HELLO")),
        "{e:?}"
    );
}

#[test]
fn cache_sessoes_disco_debug_nao_vaza_blob() {
    use std::fmt::Write as _;
    use vbl_fxp::sessoes::CacheSessoesDisco;
    let dir = Rascunho::nova("cache-sessoes-debug");
    let caminho = dir.caminho("sessoes.json");
    let cache = CacheSessoesDisco::open(&caminho, 7).expect("cache");
    let mut saida = String::new();
    write!(&mut saida, "{cache:?}").expect("debug");
    assert!(saida.contains("sessoes.json"), "{saida}");
    assert!(saida.contains('7'), "{saida}");
    assert!(
        !saida.contains("blob"),
        "Debug não deve renderizar o conteúdo das entradas: {saida}"
    );
}

#[test]
fn caminho_padrao_do_store_segue_xdg_e_home() {
    // §7: sem --tofu-store, o store padrão nasce sob
    // XDG_STATE_HOME (ou HOME)/verbo/fxp-known-hosts.json.
    let dir = Rascunho::nova("tofu-padrao");
    let xdg = dir.caminho("state");
    std::env::set_var("XDG_STATE_HOME", &xdg);
    let esperado = xdg.join("verbo").join("fxp-known-hosts.json");
    assert_eq!(TofuStore::caminho_padrao(), Some(esperado));

    // Sem XDG, cai no HOME (restaura o ambiente no fim — os testes
    // compartilham o processo).
    std::env::remove_var("XDG_STATE_HOME");
    let home = dir.caminho("home");
    let home_antigo = std::env::var("HOME").ok();
    std::env::set_var("HOME", &home);
    let esperado_home = home.join(".local").join("state").join("verbo").join("fxp-known-hosts.json");
    assert_eq!(TofuStore::caminho_padrao(), Some(esperado_home));
    std::env::remove_var("HOME");
    if let Some(h) = home_antigo {
        std::env::set_var("HOME", h);
    }
}

#[test]
fn registro_recusa_excesso_de_pins_e_slot_mdns_vazio() {
    use vbl_fxp::registry::Endpoint;
    // v1.4 §7: teto de pins por endpoint (rotação, não lista de confiança).
    let pins: Vec<String> = (0..9).map(|i| format!("{:064x}", i + 1)).collect();
    let e = Endpoint::parse(&format!("tcps:h:1@sha256:{}", pins.join(",")))
        .expect_err("9 pins devem ser recusados");
    assert!(e.to_string().contains("máximo"), "{e}");

    // §4.10: slot mdns sem identificador ⇒ erro honesto.
    assert!(Endpoint::parse("mdns:").is_err());
    // mdns com identificador SEM a feature ⇒ recusado honestamente
    // (com a feature, resolve — por isso o braço é condicional aqui).
    #[cfg(not(feature = "mdns"))]
    assert!(Endpoint::parse("mdns:fxpd-lab").is_err());
}

#[test]
fn config_recusa_compress_threshold_que_nao_cabe_em_usize() {
    // §4.8 (config): `compress_threshold` fora do usize ⇒ erro honesto,
    // nunca truncamento silencioso.
    let cfg = "mode = simulado\ncompress_threshold = 99999999999999999999999\n";
    let e = vbl_fxp::registry::FxpConfig::parse(cfg)
        .and_then(|c| {
            let mut r = DeviceRegistry::new();
            c.apply(&mut r)
        })
        .expect_err("threshold gigantesco deve falhar");
    assert!(e.to_string().contains("compress_threshold"), "{e}");
}

#[test]
fn wait_ready_unix_reporta_servidor_vivo_e_morto() {
    use vbl_fxp::transport::{serve_unix, wait_ready_unix};
    let dir = Rascunho::nova("wait-ready");
    let sock = dir.caminho("pronto.sock");
    let _srv = serve_unix(&sock, |m: Message| Some(m)).expect("servidor unix");
    assert!(
        wait_ready_unix(&sock, Duration::from_secs(2)),
        "servidor vivo deve responder ao probe"
    );
    let morto = dir.caminho("ninguem.sock");
    assert!(
        !wait_ready_unix(&morto, Duration::from_millis(60)),
        "caminho sem servidor deve estourar o prazo"
    );
}
