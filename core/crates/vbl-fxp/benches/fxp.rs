//! Orçamentos de latência do FXP (AGENTS §1.2/§1.3, PLAN Etapa 3 + v1.1):
//! - schema v1: encode+decode sem perda;
//! - leitura local simulada e real (fixture sysfs): meta ≤ 1 ms p95;
//! - leitura remota Unix (schema v1 sobre socket): meta ≤ 10 ms p95;
//! - atuação local (ack do driver): meta ≤ 10 µs (overhead de integração);
//! - v1.1 (PLAN §8): lote×individual, timestamp no fio, compressão LZ4 —
//!   números para docs/reports/FXP-V1.1-REPORT.md.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vbl_fxp::bus::{BusConfig, FxpBus};
use vbl_fxp::registry::{DeviceEntry, DeviceRegistry, FxpConfig, OperationMode};
use vbl_fxp::schema::{caps, decode, encode_to_vec, AckAct, BatchResult, Message};
use vbl_fxp::transport::{serve_unix, wait_ready_unix};
use vbl_fxp::{PeerConfig, PeerServer, TlsAccept};
use vbl_runtime::fxp::{Fxp, Value};
use vbl_runtime::ledger::ChainLedger;

fn tmpdir(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vbl-bench-{}-{}-{}",
        name,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("criar tmpdir");
    dir
}

fn bus_simulated() -> FxpBus {
    FxpBus::build(
        DeviceRegistry::minimum(),
        BusConfig {
            mode: OperationMode::Simulated,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    )
}

/// Bus híbrido com cpu_temp real (fixture thermal_zone) e CpuPowerCap real
/// (fixture rapl_constraint), cache desligado para medir I/O cru.
fn bus_real_fixture() -> (FxpBus, PathBuf, PathBuf) {
    let tz = tmpdir("bench-tz");
    fs::write(tz.join("temp"), "86500").unwrap();
    let cap = tmpdir("bench-cap").join("constraint_0_power_limit_uw");
    fs::write(&cap, "250000000").unwrap();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\ncache_ttl_ms = 0\n\
         cpu_temp.mode = real\ncpu_temp.endpoint = thermal_zone:{}\n\
         CpuPowerCap.mode = real\nCpuPowerCap.endpoint = rapl_constraint:{}\n",
        tz.display(),
        cap.display()
    ))
    .unwrap();
    let mut registry = DeviceRegistry::minimum();
    cfg.apply(&mut registry).unwrap();
    let bus = FxpBus::build(
        registry,
        // Cache ZERO: mede I/O cru, não acertos de cache.
        BusConfig {
            mode: OperationMode::Hybrid,
            cache_ttl: Duration::ZERO,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    );
    (bus, tz, cap)
}

fn schema_v1(c: &mut Criterion) {
    let mut group = c.benchmark_group("fxp_schema_v1");
    let msg = Message::read_ok(86.5, "cpu_temp", false, 42);
    group.bench_function("encode_decode_read_ok", |b| {
        b.iter(|| {
            let bytes = encode_to_vec(black_box(&msg)).expect("encode");
            let (dec, _) = decode(&bytes).expect("roundtrip");
            black_box(dec)
        })
    });
    let act = Message::act(
        "CpuPowerCap",
        vbl_fxp::schema::WireValue::Num(50.0),
        7,
        true,
    );
    group.bench_function("encode_decode_act", |b| {
        b.iter(|| {
            let bytes = encode_to_vec(black_box(&act)).expect("encode");
            let (dec, _) = decode(&bytes).expect("roundtrip");
            black_box(dec)
        })
    });
    group.finish();
}

fn local_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("fxp_leitura_local");
    group.throughput(criterion::Throughput::Elements(1));

    // Rota simulada (paridade Etapa 2).
    let mut bus = bus_simulated();
    let mut ledger = ChainLedger::new();
    group.bench_function("simulado", |b| {
        b.iter(|| {
            ledger.reset();
            black_box(
                bus.read_sensor(black_box("cpu_temp"), &mut ledger)
                    .expect("leitura"),
            )
        })
    });

    // Rota real com driver de arquivo (fixture sysfs).
    let (mut bus, _tz, _cap) = bus_real_fixture();
    let mut ledger = ChainLedger::new();
    group.bench_function("real_fixture", |b| {
        b.iter(|| {
            ledger.reset();
            black_box(
                bus.read_sensor(black_box("cpu_temp"), &mut ledger)
                    .expect("leitura"),
            )
        })
    });
    group.finish();
}

fn remote_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("fxp_leitura_remota");
    group.throughput(criterion::Throughput::Elements(1));

    let sock = tmpdir("bench-remoto").join("fxpd.sock");
    let _srv = serve_unix(&sock, |msg| match msg.opcode {
        vbl_fxp::schema::op::READ => Some(Message::read_ok(77.5, "solar_panel", false, msg.seq)),
        vbl_fxp::schema::op::ACT => Some(Message::act_ack(AckAct::Delivered, false, msg.seq)),
        _ => None,
    })
    .expect("servidor unix");
    assert!(wait_ready_unix(&sock, Duration::from_secs(2)));

    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\ncache_ttl_ms = 0\n\
         solar_panel.grandeza = luz\nsolar_panel.unidade = W/m2\n\
         solar_panel.mode = real\nsolar_panel.endpoint = unix:{}\n",
        sock.display()
    ))
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let mut bus = FxpBus::build(
        registry,
        // Cache ZERO: cada iteração é um roundtrip real de schema v1.
        BusConfig {
            mode: OperationMode::Hybrid,
            cache_ttl: Duration::ZERO,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();
    group.bench_function("unix_roundtrip", |b| {
        b.iter(|| {
            ledger.reset();
            black_box(
                bus.read_sensor(black_box("solar_panel"), &mut ledger)
                    .expect("leitura"),
            )
        })
    });
    group.finish();
}

fn local_actuation(c: &mut Criterion) {
    let mut group = c.benchmark_group("fxp_atuacao_local");
    group.throughput(criterion::Throughput::Elements(1));

    // Ator simulado: validação + efeito no simulador (paridade Etapa 2).
    let mut bus = bus_simulated();
    let mut ledger = ChainLedger::new();
    group.bench_function("simulado", |b| {
        b.iter(|| {
            ledger.reset();
            black_box(bus.act(black_box("CpuPowerCap"), Value::Num(50.0), &mut ledger))
        })
    });

    // Ator real: validação no registro + escrita no endpoint (fixture µW).
    let (mut bus, _tz, _cap) = bus_real_fixture();
    let mut ledger = ChainLedger::new();
    group.bench_function("real_fixture", |b| {
        b.iter(|| {
            ledger.reset();
            black_box(bus.act(black_box("CpuPowerCap"), Value::Num(50.0), &mut ledger))
        })
    });
    group.finish();
}

// ══════════════════════════════════════════════════════════════════════════
// v1.1 — lote × individual, timestamp no fio e compressão LZ4
// ══════════════════════════════════════════════════════════════════════════

/// Registra `n` sensores simulados num registro novo.
fn registry_n_sensores(n: usize) -> DeviceRegistry {
    let mut r = DeviceRegistry::new();
    for i in 0..n {
        let _ = r.register(DeviceEntry::sensor(
            &format!("temp_{i}"),
            "temperature",
            "°C",
            1.0,
        ));
    }
    r
}

/// Peer real (PeerServer) anunciando todas as CAPS + bus cliente com os
/// recursos pedidos; devolve (bus, ledger, servidor — que precisa continuar
/// vivo durante o bench).
fn setup_peer_v11(
    features: impl FnOnce(&mut BusConfig),
) -> (FxpBus, ChainLedger, vbl_fxp::transport::Server) {
    // tmpdir() cria um DIRETÓRIO; o socket unix vive dentro dele.
    let sock = tmpdir("bench-v11").join("fxpd.sock");
    let peer = PeerServer::new(
        FxpBus::build(
            registry_n_sensores(8),
            BusConfig {
                mode: OperationMode::Simulated,
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
        PeerConfig {
            psk: None,
            caps: caps::LZ4 | caps::BATCH | caps::TIMESTAMP,
            ..Default::default()
        },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, Path::new(&sock)).expect("servidor");
    std::thread::sleep(Duration::from_millis(20));

    let mut cli_txt = String::from("mode = hibrido\n");
    for i in 0..8 {
        cli_txt.push_str(&format!(
            "temp_{i}.grandeza = temperatura\ntemp_{i}.unidade = C\n\
             temp_{i}.mode = real\ntemp_{i}.endpoint = unix:{}\n",
            sock.display()
        ));
    }
    let mut cli_reg = DeviceRegistry::new();
    FxpConfig::parse(&cli_txt)
        .unwrap()
        .apply(&mut cli_reg)
        .unwrap();
    let bus = bus_cliente_com(cli_reg, features);
    (bus, ChainLedger::new(), _srv)
}

fn bus_cliente_com(reg: DeviceRegistry, features: impl FnOnce(&mut BusConfig)) -> FxpBus {
    let mut cfg = BusConfig {
        mode: OperationMode::Hybrid,
        cache_ttl: Duration::ZERO,
        ..Default::default()
    };
    features(&mut cfg);
    FxpBus::build(reg, cfg, vbl_runtime::FxpSimulator::new())
}

fn v11_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("fxp_v11_batch");
    // Ciclo completo de atualização de 8 sensores remotos:
    // - caminho v1.0: 8 READs = 8 RTTs (cache_ttl 0 ⇒ toda leitura é I/O);
    let (mut bus_ind, mut ledger, _srv1) = setup_peer_v11(|c| {
        c.cache_ttl = Duration::ZERO;
    });
    group.bench_function("ciclo_8_sensores_individual", |b| {
        b.iter(|| {
            ledger.reset();
            for i in 0..8 {
                black_box(
                    bus_ind
                        .read_sensor(black_box(&format!("temp_{i}")), &mut ledger)
                        .expect("leitura"),
                );
            }
        })
    });
    // - v1.1 (§4.7): 1 READ_BATCH = 1 RTT + 7 acertos de cache (TTL 1 s,
    //   cache invalidado no setup de cada iteração para medir sempre o fio).
    let (mut bus_batch, mut ledger2, _srv2) = setup_peer_v11(|c| {
        c.cache_ttl = Duration::from_secs(1);
        c.batch_prefetch = true;
        c.wire_timestamp = true;
    });
    group.bench_function("ciclo_8_sensores_lote_1rtt", |b| {
        b.iter(|| {
            bus_batch.invalidate_cache(); // setup barato: mede sempre o fio
            ledger2.reset();
            for i in 0..8 {
                black_box(
                    bus_batch
                        .read_sensor(black_box(&format!("temp_{i}")), &mut ledger2)
                        .expect("leitura"),
                );
            }
        })
    });
    group.finish();
}

fn v11_timestamp_and_compression(c: &mut Criterion) {
    let mut group = c.benchmark_group("fxp_v11_fio");
    let msg = Message::read_ok(86.5, "cpu_temp", false, 42);

    // Custo do FLAG_TIMESTAMP no roundtrip (§5): 8 bytes + branch.
    let stamped = msg.clone().with_timestamp(1_756_845_000_000_123);
    group.bench_function("roundtrip_com_timestamp", |b| {
        b.iter(|| {
            let bytes = encode_to_vec(black_box(&stamped)).expect("encode");
            let (dec, _) = decode(&bytes).expect("roundtrip");
            black_box(dec)
        })
    });

    // Compressão LZ4 (§4.8): HELLO grande (onde a compressão atua) × plano.
    let mut reg = DeviceRegistry::new();
    for i in 0..60 {
        let _ = reg.register(DeviceEntry::sensor(
            &format!("sensor_numero_{i:02}_do_registro"),
            "temperature",
            "°C",
            1.5,
        ));
    }
    let hello = Message::hello(reg.devices().map(|d| d.to_device_desc()).collect(), 9);
    group.bench_function("encode_decode_hello_plano", |b| {
        b.iter(|| {
            let bytes = encode_to_vec(black_box(&hello)).expect("encode");
            let (dec, _) = decode(&bytes).expect("roundtrip");
            black_box(dec)
        })
    });
    group.bench_function("encode_decode_hello_lz4", |b| {
        b.iter(|| {
            let mut bytes = Vec::new();
            vbl_fxp::schema::encode_with_compression(black_box(&hello), &mut bytes)
                .expect("encode");
            let (dec, _) = decode(&bytes).expect("roundtrip");
            black_box(dec)
        })
    });

    // Lote no codec (§4.7): 64 itens.
    let nomes: Vec<String> = (0..64).map(|i| format!("temp_{i}")).collect();
    let lote = Message::read_batch(nomes.clone(), 1);
    let resp = Message::read_batch_ok(
        nomes
            .iter()
            .map(|n| BatchResult::Ok {
                value: 42.0,
                canonical: n.clone(),
            })
            .collect(),
        1,
    );
    group.bench_function("roundtrip_lote_64", |b| {
        b.iter(|| {
            let b1 = encode_to_vec(black_box(&lote)).expect("encode");
            let (d1, _) = decode(&b1).expect("roundtrip");
            let b2 = encode_to_vec(black_box(&resp)).expect("encode");
            let (d2, _) = decode(&b2).expect("roundtrip");
            black_box((d1, d2))
        })
    });

    group.finish();
}

fn v11_auth(c: &mut Criterion) {
    use vbl_fxp::transport::Connection;
    let mut group = c.benchmark_group("fxp_v11_auth");

    let sock_plana = tmpdir("bench-auth-plana").join("fxpd.sock");
    let peer_plana = PeerServer::new(
        FxpBus::build(
            registry_n_sensores(1),
            BusConfig {
                mode: OperationMode::Simulated,
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
        PeerConfig::default(),
    );
    let _srv_plana =
        vbl_fxp::peer::serve_unix_peer(&peer_plana, Path::new(&sock_plana)).expect("srv");
    std::thread::sleep(Duration::from_millis(20));

    let sock_auth = tmpdir("bench-auth-psk").join("fxpd.sock");
    let peer_auth = PeerServer::new(
        FxpBus::build(
            registry_n_sensores(1),
            BusConfig {
                mode: OperationMode::Simulated,
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
        PeerConfig {
            psk: Some(b"chave-do-bench".to_vec()),
            caps: caps::LZ4 | caps::BATCH | caps::TIMESTAMP,
            ..Default::default()
        },
    );
    let _srv_auth = vbl_fxp::peer::serve_unix_peer(&peer_auth, Path::new(&sock_auth)).expect("srv");
    std::thread::sleep(Duration::from_millis(20));

    group.bench_function("conectar_ler_plano", |b| {
        b.iter(|| {
            let mut c =
                Connection::unix(Path::new(&sock_plana), Duration::from_secs(1)).expect("con");
            let r = c
                .request(&Message::read("temp_0", 1, true), Duration::from_secs(1))
                .expect("r");
            black_box(r)
        })
    });
    group.bench_function("conectar_auth_caps_ler", |b| {
        b.iter(|| {
            let mut c =
                Connection::unix(Path::new(&sock_auth), Duration::from_secs(1)).expect("con");
            c.authenticate(b"chave-do-bench", Duration::from_secs(1))
                .expect("auth");
            c.negotiate(
                caps::LZ4 | caps::BATCH | caps::TIMESTAMP,
                Duration::from_secs(1),
            )
            .expect("caps");
            let r = c
                .request(&Message::read("temp_0", 1, true), Duration::from_secs(1))
                .expect("r");
            black_box(r)
        })
    });
    group.finish();
}

// ══════════════════════════════════════════════════════════════════════════
// v1.2 — TLS 1.3 (§7) e dicionário compartilhado (§4.8)
// ══════════════════════════════════════════════════════════════════════════

fn v12_tls_e_dict(c: &mut Criterion) {
    use std::path::Path;
    use vbl_fxp::transport::Connection;

    let mut group = c.benchmark_group("v12_tls_dict");
    let dir = std::env::temp_dir().join(format!("vbl-bench-v12-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);

    // --- TLS: handshake + frame sobre TLS vs TCP plano (mesma leitura) ---
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).expect("cert");
    let accept = TlsAccept {
        certs_pem: cert.cert.pem(),
        key_pem: cert.signing_key.serialize_pem(),
        sessoes: None,
    };
    let peer_tls = PeerServer::new(
        FxpBus::build(
            registry_n_sensores(1),
            BusConfig {
                mode: OperationMode::Simulated,
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(accept),
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer_port(&peer_tls, 0).expect("srv tls");
    std::thread::sleep(Duration::from_millis(20));
    let fingerprint = vbl_fxp::tls::fingerprint(cert.cert.der());
    let peer_plano = PeerServer::new(
        FxpBus::build(
            registry_n_sensores(1),
            BusConfig {
                mode: OperationMode::Simulated,
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
        PeerConfig::default(),
    );
    let (_srv2, porta_plana) =
        vbl_fxp::peer::serve_tcp_peer_port(&peer_plano, 0).expect("srv plano");
    std::thread::sleep(Duration::from_millis(20));

    // v1.3: o ClientConfig é cached e a sessão RETOMA — este bench mede o
    // caminho retomado (o handshake completo da v1.2 está no relatório
    // FXP-V1.2-REPORT.md; o grupo v13_tls_0rtt isola 0-RTT e sem-0-RTT).
    group.bench_function("tls_handshake_resumido_ler", |b| {
        b.iter(|| {
            let confianca = vbl_fxp::tls::ConfiancaCliente::Pin(vec![fingerprint]);
            let mut conn =
                Connection::tcp_tls("127.0.0.1", porta, &confianca, Duration::from_secs(2), None)
                    .expect("tls con");
            let r = conn
                .request(&Message::read("temp_0", 1, true), Duration::from_secs(1))
                .expect("r");
            black_box(r)
        })
    });
    group.bench_function("tcp_plano_handshake_ler", |b| {
        b.iter(|| {
            let mut conn =
                Connection::tcp("127.0.0.1", porta_plana, Duration::from_secs(2)).expect("con");
            let r = conn
                .request(&Message::read("temp_0", 1, true), Duration::from_secs(1))
                .expect("r");
            black_box(r)
        })
    });

    // --- Dict: HELLO do registro ⇒ dict; leitura em lote com/novos nomes ---
    let sock = dir.join("bench-dict.sock");
    let peer_dict = PeerServer::new(
        FxpBus::build(
            registry_n_sensores(1),
            BusConfig {
                mode: OperationMode::Simulated,
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
        PeerConfig {
            caps: caps::LZ4 | caps::DICT,
            ..Default::default()
        },
    );
    let _srv3 = vbl_fxp::peer::serve_unix_peer(&peer_dict, Path::new(&sock)).expect("srv dict");
    std::thread::sleep(Duration::from_millis(20));

    // Codec puro: mesmo payload com LZ4 simples × LZ4+dict.
    let nomes: Vec<String> = (0..40)
        .map(|i| format!("temp_{i:02}_sensor_de_temperatura_do_rack_{i:02}"))
        .collect();
    let dict = vbl_fxp::schema::compress::dict_from_registry(&nomes);
    let resultados: Vec<vbl_fxp::BatchResult> = nomes
        .iter()
        .map(|n| vbl_fxp::BatchResult::Ok {
            value: 36.5,
            canonical: n.clone(),
        })
        .collect();
    let msg = Message::read_batch_ok(resultados, 1);
    group.bench_function("encode_lz4_simples", |b| {
        b.iter(|| {
            let mut f = Vec::new();
            vbl_fxp::schema::encode_with_compression(&msg, &mut f).expect("enc");
            black_box(f)
        })
    });
    group.bench_function("encode_lz4_dict", |b| {
        b.iter(|| {
            let mut f = Vec::new();
            vbl_fxp::schema::encode_with_compression_dict(&msg, &dict, &mut f).expect("enc");
            black_box(f)
        })
    });
    let mut frame = Vec::new();
    vbl_fxp::schema::encode_with_compression_dict(&msg, &dict, &mut frame).expect("enc");
    group.bench_function("decode_lz4_dict", |b| {
        b.iter(|| {
            let r = vbl_fxp::schema::decode_with_dict(&frame, Some(&dict)).expect("dec");
            black_box(r)
        })
    });

    let _ = std::fs::remove_dir_all(&dir);
    group.finish();
}

fn v13_tls_0rtt_e_zstd(c: &mut Criterion) {
    use std::path::Path;
    use vbl_fxp::transport::Connection;

    let mut group = c.benchmark_group("v13_tls_0rtt");
    let dir = std::env::temp_dir().join(format!("vbl-bench-v13-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);

    // --- TLS v1.3: retomada SEM 0-RTT × retomada COM CAPS adiantado ------
    // (handshake completo não é mais benchável por API pública: o cache de
    // ClientConfig por pin é o que FAZ a retomada funcionar; o número
    // completo da v1.2 serve de base de comparação no relatório.)
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).expect("cert");
    let accept = TlsAccept {
        certs_pem: cert.cert.pem(),
        key_pem: cert.signing_key.serialize_pem(),
        sessoes: None,
    };
    let peer_tls = PeerServer::new(
        FxpBus::build(
            registry_n_sensores(1),
            BusConfig {
                mode: OperationMode::Simulated,
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(accept),
            caps: caps::LZ4,
            ..Default::default()
        },
    );
    let (_srv, porta) = vbl_fxp::peer::serve_tcp_peer_port(&peer_tls, 0).expect("srv tls");
    std::thread::sleep(Duration::from_millis(20));
    let fingerprint = vbl_fxp::tls::fingerprint(cert.cert.der());

    // Aquecimento: a PRIMEIRA conexão com o pin é sempre FULL — fora do
    // bench, para que o medido seja a retomada estável.
    let quente = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &vbl_fxp::tls::ConfiancaCliente::Pin(vec![fingerprint]),
        Duration::from_secs(2),
        None,
    )
    .and_then(|mut c| c.negotiate(caps::LZ4, Duration::from_secs(1)).map(|_| ()));
    quente.expect("aquecimento da sessão");

    group.bench_function("tls_retomado_caps_negociado_ler", |b| {
        // Retomada SEM 0-RTT: CAPS segue pelo caminho normal pós-handshake.
        b.iter(|| {
            let mut conn = Connection::tcp_tls(
                "127.0.0.1",
                porta,
                &vbl_fxp::tls::ConfiancaCliente::Pin(vec![fingerprint]),
                Duration::from_secs(2),
                None,
            )
            .expect("tls con");
            let r = conn
                .request(&Message::read("temp_0", 1, true), Duration::from_secs(1))
                .expect("r");
            black_box(r)
        })
    });
    group.bench_function("tls_retomado_caps_0rtt_ler", |b| {
        // v1.3 §7: o frame CAPS parte como 0-RTT junto do ClientHello —
        // o request nunca espera uma negociação separada.
        b.iter(|| {
            let mut conn = Connection::tcp_tls(
                "127.0.0.1",
                porta,
                &vbl_fxp::tls::ConfiancaCliente::Pin(vec![fingerprint]),
                Duration::from_secs(2),
                Some(caps::LZ4),
            )
            .expect("tls con");
            let r = conn
                .request(&Message::read("temp_0", 1, true), Duration::from_secs(1))
                .expect("r");
            black_box(r)
        })
    });

    // --- zstd (§4.8): treino único + codec contra LZ4/dict do v12 --------
    group.bench_function("zstd_treino_dict_41_nomes", |b| {
        let nomes: Vec<String> = (0..40)
            .map(|i| format!("temp_{i:02}_sensor_de_temperatura_do_rack_{i:02}"))
            .collect();
        b.iter(|| {
            let d = vbl_fxp::schema::compress::zstd_dict_from_registry(&nomes);
            black_box(d)
        })
    });
    let nomes: Vec<String> = (0..40)
        .map(|i| format!("temp_{i:02}_sensor_de_temperatura_do_rack_{i:02}"))
        .collect();
    let zdict = vbl_fxp::schema::compress::zstd_dict_from_registry(&nomes).expect("treina");
    let resultados: Vec<vbl_fxp::BatchResult> = nomes
        .iter()
        .map(|n| vbl_fxp::BatchResult::Ok {
            value: 36.5,
            canonical: n.clone(),
        })
        .collect();
    let msg = Message::read_batch_ok(resultados, 1);
    group.bench_function("encode_zstd_dict", |b| {
        b.iter(|| {
            let mut f = Vec::new();
            vbl_fxp::schema::encode_with_zstd_dict(&msg, &zdict, &mut f).expect("enc");
            black_box(f)
        })
    });
    let mut zframe = Vec::new();
    vbl_fxp::schema::encode_with_zstd_dict(&msg, &zdict, &mut zframe).expect("enc");
    group.bench_function("decode_zstd_dict", |b| {
        b.iter(|| {
            let r = vbl_fxp::schema::decode_with_conexao(
                &zframe,
                Some(&vbl_fxp::schema::compress::DictConexao::Zstd(zdict.clone())),
            )
            .expect("dec");
            black_box(r)
        })
    });

    let _ = std::fs::remove_dir_all(&dir);
    let _ = Path::new(&dir);
    group.finish();
}

/// v1.4 §9: 0-RTT em rede com RTT REAL (> 1 ms) — quantifica o ganho FORA do
/// loopback. Um proxy TCP injeta atraso unilateral por VOO (chunk lido):
/// cada travessia paga +atraso, como a rede cobra. Default 3000 µs (= RTT
/// de 6 ms) — sobrescreva com `FXP_BENCH_RTT_US`.
fn v14_tls_0rtt_rtt(c: &mut Criterion) {
    use std::io::{Read, Write};
    use std::path::Path;
    use vbl_fxp::transport::Connection;

    let atraso = Duration::from_micros(
        std::env::var("FXP_BENCH_RTT_US")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000),
    );

    // Proxy: aceita em porta efêmera e encaminha ao alvo com atraso por
    // chunk — cada chunk é um "voo" aproximado (registros TLS ≤ 16 KiB).
    fn cano_com_atraso(mut de: std::net::TcpStream, mut para: std::net::TcpStream, atraso: Duration) {
        let mut buf = [0u8; 16384];
        loop {
            match de.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    std::thread::sleep(atraso);
                    if para.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = para.shutdown(std::net::Shutdown::Write);
    }
    let proxy = |alvo: u16| -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("proxy bind");
        let porta = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            for cliente in listener.incoming() {
                let Ok(cliente) = cliente else { continue };
                let Ok(remoto) = std::net::TcpStream::connect(("127.0.0.1", alvo)) else {
                    continue;
                };
                let Ok(cliente_le) = cliente.try_clone() else { continue };
                let Ok(remoto_es) = remoto.try_clone() else { continue };
                std::thread::spawn(move || cano_com_atraso(cliente_le, remoto_es, atraso));
                std::thread::spawn(move || cano_com_atraso(remoto, cliente, atraso));
            }
        });
        porta
    };

    let mut group = c.benchmark_group("v14_tls_0rtt_rtt");
    group.throughput(criterion::Throughput::Elements(1));

    // Peer TLS COM store de sessões em disco (v1.4 §7 — o mesmo desenho do
    // fxpd --tls-sessions) e LZ4 anunciado.
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).expect("cert");
    let dir = std::env::temp_dir().join(format!("vbl-bench-v14-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let accept = TlsAccept {
        certs_pem: cert.cert.pem(),
        key_pem: cert.signing_key.serialize_pem(),
        sessoes: Some(dir.join("sessoes.json")),
    };
    let peer_tls = PeerServer::new(
        FxpBus::build(
            registry_n_sensores(1),
            BusConfig {
                mode: OperationMode::Simulated,
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
        PeerConfig {
            tls: Some(accept),
            caps: caps::LZ4,
            ..Default::default()
        },
    );
    let (_srv, porta_real) =
        vbl_fxp::peer::serve_tcp_peer_port(&peer_tls, 0).expect("srv tls");
    std::thread::sleep(Duration::from_millis(20));
    let fingerprint = vbl_fxp::tls::fingerprint(cert.cert.der());
    let porta = proxy(porta_real);

    // Peer PLANO por trás do MESMO atraso (baseline honesto).
    let peer_plano = PeerServer::new(
        FxpBus::build(
            registry_n_sensores(1),
            BusConfig {
                mode: OperationMode::Simulated,
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
        PeerConfig::default(),
    );
    let (_srv2, porta_plana_real) =
        vbl_fxp::peer::serve_tcp_peer_port(&peer_plano, 0).expect("srv plano");
    std::thread::sleep(Duration::from_millis(20));
    let porta_plana = proxy(porta_plana_real);

    // Aquecimento: a primeira conexão com o pin é FULL — fora do bench.
    let quente = Connection::tcp_tls(
        "127.0.0.1",
        porta,
        &vbl_fxp::tls::ConfiancaCliente::Pin(vec![fingerprint]),
        Duration::from_secs(5),
        None,
    )
    .and_then(|mut c| c.negotiate(caps::LZ4, Duration::from_secs(2)).map(|_| ()));
    quente.expect("aquecimento da sessão");

    group.bench_function("tls_0rtt_sobre_rtt", |b| {
        // Retomada com CAPS adiantado: ClientHello + CAPS partem JUNTOS —
        // 1 RTT até a resposta (o voo do 0-RTT não espera o handshake).
        b.iter(|| {
            let mut conn = Connection::tcp_tls(
                "127.0.0.1",
                porta,
                &vbl_fxp::tls::ConfiancaCliente::Pin(vec![fingerprint]),
                Duration::from_secs(5),
                Some(caps::LZ4),
            )
            .expect("tls con");
            let r = conn
                .request(&Message::read("temp_0", 1, true), Duration::from_secs(2))
                .expect("r");
            black_box(r)
        })
    });
    group.bench_function("tls_retomado_sem_0rtt_sobre_rtt", |b| {
        // Retomada SEM 0-RTT: handshake curto (1 RTT) + CAPS + request — o
        // custo da negociação pós-handshake aparece no RTT real.
        b.iter(|| {
            let mut conn = Connection::tcp_tls(
                "127.0.0.1",
                porta,
                &vbl_fxp::tls::ConfiancaCliente::Pin(vec![fingerprint]),
                Duration::from_secs(5),
                None,
            )
            .expect("tls con");
            let r = conn
                .request(&Message::read("temp_0", 1, true), Duration::from_secs(2))
                .expect("r");
            black_box(r)
        })
    });
    group.bench_function("tcp_plano_sobre_rtt", |b| {
        // Sem TLS: conecta + request — o piso do transporte (o que o 0-RTT
        // aproxima: conexão e primeiro pedido no mesmo voo).
        b.iter(|| {
            let mut conn = Connection::tcp("127.0.0.1", porta_plana, Duration::from_secs(5))
                .expect("con");
            let r = conn
                .request(&Message::read("temp_0", 1, true), Duration::from_secs(2))
                .expect("r");
            black_box(r)
        })
    });

    let _ = std::fs::remove_dir_all(&dir);
    let _ = Path::new(&dir);
    group.finish();
}

criterion_group!(
    benches,
    schema_v1,
    local_read,
    remote_read,
    local_actuation,
    v11_batch,
    v11_timestamp_and_compression,
    v11_auth,
    v12_tls_e_dict,
    v13_tls_0rtt_e_zstd,
    v14_tls_0rtt_rtt
);
criterion_main!(benches);
