use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use plasmosome_backend::{
    CellId, CellOwner, DrainSpec, EnforcementBackend, FakeBackend, FileAccess, GrantId, GrantKind,
    PluginId, SessionFileRecipe, UniverseOp,
};
use plasmosome_ledger::{Effect, InverseVia, Ledger};

fn session_file(path: &str) -> UniverseOp {
    UniverseOp::WriteSessionFile {
        id: GrantId::new(),
        path: path.to_string(),
        recipe: SessionFileRecipe {
            contents: Vec::new(),
            mode: FileAccess::ReadOnly,
            guest_path: "/run/plasmosome/session".to_string(),
        },
        owner: CellOwner {
            cell: CellId::from("bench-cell"),
            plugin: PluginId::from("bench"),
        },
    }
}

fn fixture(size: usize) -> (plasmosome_ledger::SealedLedger, FakeBackend) {
    let mut backend = FakeBackend::new();
    let mut ledger = Ledger::new("bench");
    for index in 0..size {
        let operation = session_file(&format!("/file-{index}"));
        ledger.push(Effect::exact(
            format!("effect-{index}"),
            InverseVia::Backend(operation.handle()),
        ));
        backend.grant(operation, GrantKind::Hot).unwrap();
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
                    let report = ledger
                        .detach(
                            &mut backend,
                            &CellId::from("bench-cell"),
                            DrainSpec::forcing(),
                        )
                        .unwrap();
                    (ledger, backend, report)
                },
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
