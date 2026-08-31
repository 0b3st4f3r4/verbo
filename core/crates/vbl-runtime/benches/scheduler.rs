//! Benchmarks do escalonador (FORMAL §4.2): O(log N) por mutação,
//! varredura O(N + vencidos) por tick.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use vbl_runtime::scheduler::{Deadline, Scheduler};

fn bench_schedule(c: &mut Criterion) {
    let mut group = c.benchmark_group("escalonador");
    for &n in &[100usize, 1_000, 10_000, 100_000] {
        group.bench_with_input(BenchmarkId::new("agendar", n), &n, |b, &n| {
            let mut s = Scheduler::new();
            for i in 0..n {
                s.schedule(&format!("F{i}"), Deadline::Horizon, (i as f64) * 0.001, 1);
            }
            b.iter(|| {
                s.schedule("nova", Deadline::Horizon, 1e9, 1);
            });
        });
    }
    group.finish();
}

fn bench_drain(c: &mut Criterion) {
    let mut group = c.benchmark_group("escalonador");
    for &n in &[100usize, 1_000, 10_000, 100_000] {
        group.bench_with_input(BenchmarkId::new("drenar_todos", n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let mut s = Scheduler::new();
                    for i in 0..n {
                        s.schedule(&format!("F{i}"), Deadline::Horizon, 1.0, 1);
                    }
                    s
                },
                |mut s| {
                    let v = s.drain_due(2.0);
                    assert_eq!(v.len(), n);
                },
                criterion::BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_schedule, bench_drain);
criterion_main!(benches);
