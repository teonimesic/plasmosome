use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use plasmosome_backend::{
    CellId, CellOwner, Diff, DrainSpec, EnforcementBackend, FakeBackend, Grant, GrantKind,
    PluginId, ResidueReport,
};
use plasmosome_ledger::{Closure, DetachReport, Effect, InverseVia, Ledger, SealedLedger};
use plasmosome_testkit::builders::{proxy_map, session_file, uds_socket};

fn attach_detach(
    (mut ledger, mut backend): (Ledger, FakeBackend),
) -> (SealedLedger, FakeBackend, DetachReport) {
    let before = backend.snapshot_os_state();
    for (index, capability) in [
        session_file("/session"),
        uds_socket("/run/egressd.uds"),
        proxy_map("api.example.test", "proxy"),
    ]
    .into_iter()
    .enumerate()
    {
        let entry = backend.grant(Grant {
            owner: CellOwner {
                cell: CellId::from("attach-detach-cell"),
                plugin: PluginId::from("attach-detach"),
            },
            capability,
            kind: GrantKind::Hot,
        });
        ledger.push(Effect::exact(
            format!("grant-{index}"),
            InverseVia::Backend(entry.handle),
        ));
    }
    let Closure::ExternalFree(mut sealed) = ledger.close() else {
        unreachable!();
    };
    let report = sealed
        .detach(
            &mut backend,
            &CellId::from("attach-detach-cell"),
            DrainSpec::forcing(),
        )
        .unwrap();
    let after = backend.snapshot_os_state();
    let residue = ResidueReport::from_diff(Diff::between(&before, &after), Vec::new());
    assert_eq!(residue, ResidueReport::Empty, "{residue}");
    (sealed, backend, report)
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
