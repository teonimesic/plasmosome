use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use plasmosome_backend::{
    Capability, Diff, DrainSpec, EnforcementBackend, FakeBackend, Grant, GrantKind, PluginId,
    ResidueReport,
};
use plasmosome_ledger::{Effect, InverseVia, Ledger};

fn attach_detach((mut ledger, mut backend): (Ledger, FakeBackend)) {
    let before = backend.snapshot_os_state();
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
        plasmosome_ledger::Closure::ExternalFree(mut sealed) => {
            sealed.detach(&mut backend, DrainSpec::forcing()).unwrap();
        }
        plasmosome_ledger::Closure::OutstandingExternal(_) => unreachable!(),
    }
    let after = backend.snapshot_os_state();
    let residue = ResidueReport::from_diff(Diff::between(&before, &after), Vec::new());
    assert_eq!(residue, ResidueReport::Empty, "{residue}");
}

fn bench(c: &mut Criterion) {
    c.bench_function("attach_detach", |b| {
        b.iter_batched(
            || (Ledger::new("attach-detach"), FakeBackend::new()),
            attach_detach,
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
