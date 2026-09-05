use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use plasmosome_backend::{
    Capability, DrainSpec, EnforcementBackend, FakeBackend, Grant, GrantKind, PluginId,
};
use plasmosome_ledger::{Effect, InverseVia, Ledger};

fn fixture() -> (plasmosome_ledger::SealedLedger, FakeBackend) {
    let mut backend = FakeBackend::new();
    let mut ledger = Ledger::new("attach-detach");
    for (index, capability) in [
        Capability::SessionFile {
            path: "session".into(),
        },
        Capability::UdsSocket {
            path: "/run/egressd.uds".into(),
        },
        Capability::ProxyMap {
            host: "api.example.test".into(),
            route: "proxy".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let entry = backend.grant(Grant {
            plugin: PluginId::from("attach-detach"),
            capability,
            kind: GrantKind::Hot,
        });
        ledger.push(Effect::exact(
            format!("grant-{index}"),
            InverseVia::Backend(entry.handle),
        ));
    }
    match ledger.close() {
        plasmosome_ledger::Closure::ExternalFree(sealed) => (sealed, backend),
        plasmosome_ledger::Closure::OutstandingExternal(_) => unreachable!(),
    }
}

fn bench(c: &mut Criterion) {
    c.bench_function("attach_detach", |b| {
        b.iter_batched(
            fixture,
            |(mut ledger, mut backend)| {
                ledger.detach(&mut backend, DrainSpec::forcing()).unwrap();
            },
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
