use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use plasmosome_backend::{
    Capability, DrainSpec, EnforcementBackend, FakeBackend, Grant, GrantKind, PluginId,
};
use plasmosome_ledger::{Effect, InverseVia, Ledger};

fn fixture(size: usize) -> (plasmosome_ledger::SealedLedger, FakeBackend) {
    let mut backend = FakeBackend::new();
    let mut ledger = Ledger::new("bench");
    for index in 0..size {
        let entry = backend.grant(Grant {
            plugin: PluginId::from("bench"),
            capability: Capability::SessionFile {
                path: format!("file-{index}"),
            },
            kind: GrantKind::Hot,
        });
        ledger.push(Effect::exact(
            format!("effect-{index}"),
            InverseVia::Backend(entry.handle),
        ));
    }
    match ledger.close() {
        plasmosome_ledger::Closure::ExternalFree(sealed) => (sealed, backend),
        plasmosome_ledger::Closure::OutstandingExternal(_) => unreachable!(),
    }
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("ledger_replay");
    for size in [10, 100, 1000] {
        group.bench_function(size.to_string(), |b| {
            b.iter_batched(
                || fixture(size),
                |(mut ledger, mut backend)| {
                    ledger.detach(&mut backend, DrainSpec::forcing()).unwrap();
                },
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
