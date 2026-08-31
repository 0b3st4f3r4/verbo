//! Benchmarks de transição (AGENTS §1.3: ≤ 100 µs p95 na máquina de
//! referência, medido com criterion).
//!
//! Cenários:
//! - `transicao_revisao`: regra que dispara + reclassificação persistida
//!   (latência de transição — o orçamento do AGENTS);
//! - `tick_1_forma` / `tick_100_formas` / `tick_1000_formas`: custo do tick
//!   por escala (varredura O(N + vencidos));
//! - `subvert_mesmo_tick`: interrupção de prioridade máxima.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::path::PathBuf;
use vbl_lang::parse;
use vbl_runtime::{load, Engine, Fxp, FxpSimulator};

fn dir_temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "vbl-bench-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Latência de transição: 1 forma, 1 regra que dispara
/// (reclassify_as_equilibrium + persistência) — cenário do ADR-001.
fn bench_transition_review(c: &mut Criterion) {
    let mut group = c.benchmark_group("transicao");
    group.throughput(Throughput::Elements(1));
    group.bench_function("revisao_dispara_reclassify_1_forma", |b| {
        b.iter_batched(
            || {
                let source = "\
nonequilibrium P { value: \"v\", horizon: 60s, source_path: \"attention\", maintenance_deadline: 3s }
review P { when attention < 30% -> reclassify_as_equilibrium }";
                let (p, d) = parse(source);
                assert!(!d.has_errors());
                let dir = dir_temp("revisao");
                let mut engine = Engine::new(FxpSimulator::new(), 1.0, &dir);
                load(&mut engine, &p);
                engine.fxp.set_sensor("attention", 15.0);
                (engine, dir)
            },
            |(mut engine, dir)| {
                engine.tick(); // dispara regra + reclassifica + persiste
                let _ = std::fs::remove_dir_all(dir);
            },
            criterion::BatchSize::PerIteration,
        );
    });
    group.finish();
}

/// Custo do tick por escala — varredura O(N + vencidos).
fn bench_tick_scales(c: &mut Criterion) {
    let mut group = c.benchmark_group("tick");
    for &n in &[1usize, 100, 1000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let mut forms = String::new();
            for i in 0..n {
                forms.push_str(&format!("event F{i} {{ value: \"v{i}\", horizon: 1000000s }}\n"));
            }
            let (p, d) = parse(&forms);
            assert!(!d.has_errors());
            let dir = dir_temp("tick");
            let mut engine = Engine::new(FxpSimulator::new(), 1.0, &dir);
            load(&mut engine, &p);
            b.iter(|| engine.tick());
            let _ = std::fs::remove_dir_all(dir);
        });
    }
    group.finish();
}

/// `subvert` — interrupção de prioridade máxima, dissolução no mesmo tick.
fn bench_subvert(c: &mut Criterion) {
    c.bench_function("subvert_mesmo_tick", |b| {
        b.iter_batched(
            || {
                let source = "\
nonequilibrium T { value: \"lucro\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }
review T { when cpu_temp > 85°C -> subvert, act(CpuPowerCap, 50) }";
                let (p, d) = parse(source);
                assert!(!d.has_errors());
                let dir = dir_temp("subvert");
                let mut engine = Engine::new(FxpSimulator::new(), 1.0, &dir);
                load(&mut engine, &p);
                engine.fxp.set_sensor("cpu_temp", 86.5);
                (engine, dir)
            },
            |(mut engine, dir)| {
                engine.tick();
                let _ = std::fs::remove_dir_all(dir);
            },
            criterion::BatchSize::PerIteration,
        );
    });
}

/// Integração FXP: overhead por comando `act` local (orçamento ≤ 10 µs).
fn bench_fxp_act(c: &mut Criterion) {
    c.bench_function("fxp_act_local", |b| {
        b.iter_batched(
            || {
                let dir = dir_temp("act");
                let engine = Engine::new(FxpSimulator::new(), 1.0, &dir);
                (engine, dir)
            },
            |(mut engine, dir)| {
                for _ in 0..100 {
                    engine
                        .fxp
                        .act("Fan", vbl_runtime::Value::Num(100.0), &mut engine.ledger);
                }
                let _ = std::fs::remove_dir_all(dir);
            },
            criterion::BatchSize::PerIteration,
        );
    });
}

criterion_group!(benches, bench_transition_review, bench_tick_scales, bench_subvert, bench_fxp_act);
criterion_main!(benches);
