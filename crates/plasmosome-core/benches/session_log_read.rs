use criterion::{Criterion, criterion_group, criterion_main};
use plasmosome_core::{SessionLog, read_events};
use std::hint::black_box;

fn bench(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.ndjson");
    let log = SessionLog::open(path.clone()).expect("open session log");
    for index in 0..1000 {
        log.append("bench", serde_json::json!({"index": index}))
            .expect("append to session log");
    }
    c.bench_function("session_log_read", |b| {
        b.iter(|| black_box(read_events(&path).expect("read session log")))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
