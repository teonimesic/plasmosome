use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use plasmosome_core::SessionLog;
use std::hint::black_box;

fn bench(c: &mut Criterion) {
    c.bench_function("session_log_append", |b| {
        b.iter_batched_ref(
            || {
                let dir = tempfile::tempdir().unwrap();
                let log = SessionLog::create(dir.path().join("session.ndjson")).unwrap();
                (dir, log)
            },
            |(_, log)| {
                black_box(log.append("bench", serde_json::json!({"value": 1})));
            },
            BatchSize::PerIteration,
        )
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
