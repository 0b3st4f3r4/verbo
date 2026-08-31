//! Benchmarks do escalonador (FORMAL §4.2): O(log N) por mutação,
//! varredura O(N + vencidos) por tick.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use vbl_runtime::scheduler::{Prazo, Scheduler};

fn bench_agendar(c: &mut Criterion) {
    let mut grupo = c.benchmark_group("escalonador");
    for &n in &[100usize, 1_000, 10_000, 100_000] {
        grupo.bench_with_input(BenchmarkId::new("agendar", n), &n, |b, &n| {
            let mut s = Scheduler::new();
            for i in 0..n {
                s.agendar(&format!("F{i}"), Prazo::Horizon, (i as f64) * 0.001, 1);
            }
            b.iter(|| {
                s.agendar("nova", Prazo::Horizon, 1e9, 1);
            });
        });
    }
    grupo.finish();
}

fn bench_drenar(c: &mut Criterion) {
    let mut grupo = c.benchmark_group("escalonador");
    for &n in &[100usize, 1_000, 10_000, 100_000] {
        grupo.bench_with_input(BenchmarkId::new("drenar_todos", n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let mut s = Scheduler::new();
                    for i in 0..n {
                        s.agendar(&format!("F{i}"), Prazo::Horizon, 1.0, 1);
                    }
                    s
                },
                |mut s| {
                    let v = s.drenar_vencidos(2.0);
                    assert_eq!(v.len(), n);
                },
                criterion::BatchSize::PerIteration,
            );
        });
    }
    grupo.finish();
}

criterion_group!(benches, bench_agendar, bench_drenar);
criterion_main!(benches);
