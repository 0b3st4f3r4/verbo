//! Orçamentos de latência do FXP (AGENTS §1.2/§1.3, PLAN Etapa 3):
//! - schema v1: encode+decode sem perda;
//! - leitura local simulada e real (fixture sysfs): meta ≤ 1 ms p95;
//! - leitura remota Unix (schema v1 sobre socket): meta ≤ 10 ms p95;
//! - atuação local (ack do driver): meta ≤ 10 µs (overhead de integração).

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vbl_fxp::bus::{BusConfig, FxpBus};
use vbl_fxp::registry::{DeviceRegistry, FxpConfig, ModoOperacao};
use vbl_fxp::schema::{decode, encode_to_vec, AckAct, Mensagem};
use vbl_fxp::transport::{esperar_pronto_unix, servir_unix};
use vbl_runtime::caderno::ChainCaderno;
use vbl_runtime::fxp::{Fxp, Value};

fn tmpdir(nome: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vbl-bench-{}-{}-{}",
        nome,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("criar tmpdir");
    dir
}

fn bus_simulado() -> FxpBus {
    FxpBus::construir(
        DeviceRegistry::minimo(),
        BusConfig { modo: ModoOperacao::Simulado, ..Default::default() },
        vbl_runtime::FxpSimulator::novo(),
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
    let mut registry = DeviceRegistry::minimo();
    cfg.aplicar(&mut registry).unwrap();
    let bus = FxpBus::construir(
        registry,
        // Cache ZERO: mede I/O cru, não acertos de cache.
        BusConfig { modo: ModoOperacao::Hibrido, cache_ttl: Duration::ZERO, ..Default::default() },
        vbl_runtime::FxpSimulator::novo(),
    );
    (bus, tz, cap)
}

fn schema_v1(c: &mut Criterion) {
    let mut grupo = c.benchmark_group("fxp_schema_v1");
    let msg = Mensagem::read_ok(86.5, "cpu_temp", false, 42);
    grupo.bench_function("encode_decode_read_ok", |b| {
        b.iter(|| {
            let bytes = encode_to_vec(black_box(&msg)).expect("encode");
            let (dec, _) = decode(&bytes).expect("roundtrip");
            black_box(dec)
        })
    });
    let act = Mensagem::act("CpuPowerCap", vbl_fxp::schema::WireValue::Num(50.0), 7, true);
    grupo.bench_function("encode_decode_act", |b| {
        b.iter(|| {
            let bytes = encode_to_vec(black_box(&act)).expect("encode");
            let (dec, _) = decode(&bytes).expect("roundtrip");
            black_box(dec)
        })
    });
    grupo.finish();
}

fn leitura_local(c: &mut Criterion) {
    let mut grupo = c.benchmark_group("fxp_leitura_local");
    grupo.throughput(criterion::Throughput::Elements(1));

    // Rota simulada (paridade Etapa 2).
    let mut bus = bus_simulado();
    let mut caderno = ChainCaderno::new();
    grupo.bench_function("simulado", |b| {
        b.iter(|| {
            caderno.reset();
            black_box(bus.read_sensor(black_box("cpu_temp"), &mut caderno).expect("leitura"))
        })
    });

    // Rota real com driver de arquivo (fixture sysfs).
    let (mut bus, _tz, _cap) = bus_real_fixture();
    let mut caderno = ChainCaderno::new();
    grupo.bench_function("real_fixture", |b| {
        b.iter(|| {
            caderno.reset();
            black_box(bus.read_sensor(black_box("cpu_temp"), &mut caderno).expect("leitura"))
        })
    });
    grupo.finish();
}

fn leitura_remota(c: &mut Criterion) {
    let mut grupo = c.benchmark_group("fxp_leitura_remota");
    grupo.throughput(criterion::Throughput::Elements(1));

    let sock = tmpdir("bench-remoto").join("fxpd.sock");
    let _srv = servir_unix(
        &sock,
        |msg| match msg.opcode {
            vbl_fxp::schema::op::READ =>
                Some(Mensagem::read_ok(77.5, "solar_panel", false, msg.seq)),
            vbl_fxp::schema::op::ACT =>
                Some(Mensagem::act_ack(AckAct::Entregue, false, msg.seq)),
            _ => None,
        },
    )
    .expect("servidor unix");
    assert!(esperar_pronto_unix(&sock, Duration::from_secs(2)));

    let mut registry = DeviceRegistry::minimo();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\ncache_ttl_ms = 0\n\
         solar_panel.grandeza = luz\nsolar_panel.unidade = W/m2\n\
         solar_panel.mode = real\nsolar_panel.endpoint = unix:{}\n",
        sock.display()
    ))
    .unwrap();
    cfg.aplicar(&mut registry).unwrap();
    let mut bus = FxpBus::construir(
        registry,
        // Cache ZERO: cada iteração é um roundtrip real de schema v1.
        BusConfig { modo: ModoOperacao::Hibrido, cache_ttl: Duration::ZERO, ..Default::default() },
        vbl_runtime::FxpSimulator::novo(),
    );
    let mut caderno = ChainCaderno::new();
    grupo.bench_function("unix_roundtrip", |b| {
        b.iter(|| {
            caderno.reset();
            black_box(bus.read_sensor(black_box("solar_panel"), &mut caderno).expect("leitura"))
        })
    });
    grupo.finish();
}

fn atuacao_local(c: &mut Criterion) {
    let mut grupo = c.benchmark_group("fxp_atuacao_local");
    grupo.throughput(criterion::Throughput::Elements(1));

    // Ator simulado: validação + efeito no simulador (paridade Etapa 2).
    let mut bus = bus_simulado();
    let mut caderno = ChainCaderno::new();
    grupo.bench_function("simulado", |b| {
        b.iter(|| {
            caderno.reset();
            black_box(bus.act(black_box("CpuPowerCap"), Value::Num(50.0), &mut caderno))
        })
    });

    // Ator real: validação no registro + escrita no endpoint (fixture µW).
    let (mut bus, _tz, _cap) = bus_real_fixture();
    let mut caderno = ChainCaderno::new();
    grupo.bench_function("real_fixture", |b| {
        b.iter(|| {
            caderno.reset();
            black_box(bus.act(black_box("CpuPowerCap"), Value::Num(50.0), &mut caderno))
        })
    });
    grupo.finish();
}

criterion_group!(benches, schema_v1, leitura_local, leitura_remota, atuacao_local);
criterion_main!(benches);
