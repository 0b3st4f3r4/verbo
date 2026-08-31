//! Benchmarks do Caderno de produção (Etapa 4 — PLAN §4.3/AGENTS §1.4).
//!
//! Métricas:
//! - `caderno_gravacao/*`: latência de gravação por evento (orçamento
//!   ≤ 200 µs — AGENTS §1.4), comparando produção assíncrona × memória ×
//!   no-op;
//! - `caderno_overhead/*`: A/B do logger LIGADO × DESLIGADO (PLAN §4.3:
//!   "overhead do Caderno pode distorcer medições") — mesmo tick de 1000
//!   formas com NoopCaderno (logger off), CadernoProducao (on, assíncrono)
//!   e ChainCaderno (on, memória).
//!
//! O p95 sai do relatório do criterion (`--quick` no CI; completo em
//! `make rust-bench`).

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::path::PathBuf;
use vbl_lang::parse;
use vbl_runtime::caderno::{Caderno, ChainCaderno, NoopCaderno};
use vbl_runtime::caderno_producao::CadernoProducao;
use vbl_runtime::json::Json;
use vbl_runtime::{carregar, Engine, FxpSimulator};

fn caminho_bench(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vbl-bench-caderno-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("caderno.vcad")
}

fn evento_exemplo(i: usize) -> (&'static str, String, Json) {
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
fn bench_caderno_gravacao(c: &mut Criterion) {
    let mut grupo = c.benchmark_group("caderno_gravacao");
    grupo.throughput(Throughput::Elements(1));

    grupo.bench_function("producao_assincrona_evento", |b| {
        let caminho = caminho_bench("producao");
        let mut caderno = CadernoProducao::abrir(&caminho).expect("abrir caderno de produção");
        let mut i = 0usize;
        b.iter(|| {
            let (kind, msg, extra) = evento_exemplo(i);
            caderno.record(kind, &msg, extra);
            i += 1;
        });
        drop(caderno); // fecha a thread e o arquivo
        let _ = std::fs::remove_file(&caminho);
    });

    grupo.bench_function("memoria_cadeia_evento", |b| {
        let mut caderno = ChainCaderno::new();
        let mut i = 0usize;
        b.iter(|| {
            let (kind, msg, extra) = evento_exemplo(i);
            caderno.record(kind, &msg, extra);
            i += 1;
        });
    });

    grupo.bench_function("noop_evento", |b| {
        let mut caderno = NoopCaderno;
        let mut i = 0usize;
        b.iter(|| {
            let (kind, msg, extra) = evento_exemplo(i);
            caderno.record(kind, &msg, extra);
            i += 1;
        });
    });

    grupo.finish();
}

/// A/B overhead do logger: tick de 1000 formas com cada implementação.
/// O delta Producao−Noop é o custo do logging ligado (PLAN §4.3).
fn bench_caderno_overhead(c: &mut Criterion) {
    let mut grupo = c.benchmark_group("caderno_overhead");
    grupo.throughput(Throughput::Elements(1000));

    let mut programa = String::new();
    for i in 0..1000 {
        programa.push_str(&format!("event F{i} {{ value: \"v{i}\", horizon: 1000000s }}\n"));
    }
    let (p, d) = parse(&programa);
    assert!(!d.has_errors());

    // logger DESLIGADO (referência do A/B)
    grupo.bench_function("tick_1000_formas_logger_desligado", |b| {
        let dir = std::env::temp_dir().join(format!("vbl-bench-ab-noop-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut engine = Engine::com_caderno(FxpSimulator::novo(), 1.0, &dir, NoopCaderno);
        carregar(&mut engine, &p);
        b.iter(|| engine.tick());
        let _ = std::fs::remove_dir_all(dir);
    });

    // logger LIGADO — produção (gravação assíncrona em buffer)
    grupo.bench_function("tick_1000_formas_logger_producao", |b| {
        let caminho = caminho_bench("overhead");
        let producao = CadernoProducao::abrir(&caminho).expect("abrir caderno");
        let dir = std::env::temp_dir().join(format!("vbl-bench-ab-prod-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut engine = Engine::com_caderno(FxpSimulator::novo(), 1.0, &dir, producao);
        carregar(&mut engine, &p);
        b.iter(|| engine.tick());
        drop(engine);
        let _ = std::fs::remove_file(&caminho);
        let _ = std::fs::remove_dir_all(dir);
    });

    // logger LIGADO — memória (implementação de referência, tudo em RAM)
    grupo.bench_function("tick_1000_formas_logger_memoria", |b| {
        let dir = std::env::temp_dir().join(format!("vbl-bench-ab-mem-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut engine = Engine::com_caderno(FxpSimulator::novo(), 1.0, &dir, ChainCaderno::new());
        carregar(&mut engine, &p);
        b.iter(|| engine.tick());
        let _ = std::fs::remove_dir_all(dir);
    });

    grupo.finish();
}

criterion_group!(benches, bench_caderno_gravacao, bench_caderno_overhead);
criterion_main!(benches);
