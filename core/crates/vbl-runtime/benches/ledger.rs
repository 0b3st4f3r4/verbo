//! Benchmarks do Caderno de produção (Etapa 4 — PLAN §4.3/AGENTS §1.4).
//!
//! Métricas:
//! - `caderno_gravacao/*`: latência de gravação por evento (orçamento
//!   ≤ 200 µs — AGENTS §1.4), comparando produção assíncrona × memória ×
//!   no-op;
//! - `caderno_overhead/*`: A/B do logger LIGADO × DESLIGADO (PLAN §4.3:
//!   "overhead do Caderno pode distorcer medições") — mesmo tick de 1000
//!   formas com NoopCaderno (logger off), ProductionLedger (on, assíncrono)
//!   e ChainCaderno (on, memória).
//!
//! O p95 sai do relatório do criterion (`--quick` no CI; completo em
//! `make rust-bench`).

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::path::PathBuf;
use vbl_lang::parse;
use vbl_runtime::ledger::{Ledger, ChainLedger, NoopLedger};
use vbl_runtime::production_ledger::ProductionLedger;
use vbl_runtime::json::Json;
use vbl_runtime::{load, Engine, FxpSimulator};

fn bench_path(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vbl-bench-caderno-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("caderno.vcad")
}

fn example_event(i: usize) -> (&'static str, String, Json) {
    (
        "VAZAMENTO",
        format!("Forma 'F{i}' dissipou 0.15 Joules (0.15 W por 1.00s)"),
        Json::obj([
            ("forma", Json::str(format!("F{i}"))),
            ("watts", Json::num(0.15)),
            ("segundos", Json::num(1.0)),
            ("joules", Json::num(0.15)),
        ]),
    )
}

/// Latência de gravação por evento (orçamento ≤ 200 µs p95).
fn bench_ledger_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("caderno_gravacao");
    group.throughput(Throughput::Elements(1));

    group.bench_function("producao_assincrona_evento", |b| {
        let path = bench_path("producao");
        let mut ledger = ProductionLedger::open(&path).expect("abrir caderno de produção");
        let mut i = 0usize;
        b.iter(|| {
            let (kind, msg, extra) = example_event(i);
            ledger.record(kind, &msg, extra);
            i += 1;
        });
        drop(ledger); // fecha a thread e o arquivo
        let _ = std::fs::remove_file(&path);
    });

    group.bench_function("memoria_cadeia_evento", |b| {
        let mut ledger = ChainLedger::new();
        let mut i = 0usize;
        b.iter(|| {
            let (kind, msg, extra) = example_event(i);
            ledger.record(kind, &msg, extra);
            i += 1;
        });
    });

    group.bench_function("noop_evento", |b| {
        let mut ledger = NoopLedger;
        let mut i = 0usize;
        b.iter(|| {
            let (kind, msg, extra) = example_event(i);
            ledger.record(kind, &msg, extra);
            i += 1;
        });
    });

    group.finish();
}

/// A/B overhead do logger: tick de 1000 formas com cada implementação.
/// O delta Producao−Noop é o custo do logging ligado (PLAN §4.3).
fn bench_ledger_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("caderno_overhead");
    group.throughput(Throughput::Elements(1000));

    let mut program = String::new();
    for i in 0..1000 {
        program.push_str(&format!("event F{i} {{ value: \"v{i}\", horizon: 1000000s }}\n"));
    }
    let (p, d) = parse(&program);
    assert!(!d.has_errors());

    // logger DESLIGADO (referência do A/B)
    group.bench_function("tick_1000_formas_logger_desligado", |b| {
        let dir = std::env::temp_dir().join(format!("vbl-bench-ab-noop-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut engine = Engine::with_ledger(FxpSimulator::new(), 1.0, &dir, NoopLedger);
        load(&mut engine, &p);
        b.iter(|| engine.tick());
        let _ = std::fs::remove_dir_all(dir);
    });

    // logger LIGADO — produção (gravação assíncrona em buffer)
    group.bench_function("tick_1000_formas_logger_producao", |b| {
        let path = bench_path("overhead");
        let production = ProductionLedger::open(&path).expect("abrir caderno");
        let dir = std::env::temp_dir().join(format!("vbl-bench-ab-prod-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut engine = Engine::with_ledger(FxpSimulator::new(), 1.0, &dir, production);
        load(&mut engine, &p);
        b.iter(|| engine.tick());
        drop(engine);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(dir);
    });

    // logger LIGADO — memória (implementação de referência, tudo em RAM)
    group.bench_function("tick_1000_formas_logger_memoria", |b| {
        let dir = std::env::temp_dir().join(format!("vbl-bench-ab-mem-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut engine = Engine::with_ledger(FxpSimulator::new(), 1.0, &dir, ChainLedger::new());
        load(&mut engine, &p);
        b.iter(|| engine.tick());
        let _ = std::fs::remove_dir_all(dir);
    });

    group.finish();
}

criterion_group!(benches, bench_ledger_write, bench_ledger_overhead);
criterion_main!(benches);
