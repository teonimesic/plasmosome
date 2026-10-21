use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use plasmosome_backend::{
    CellId, CellOwner, Diff, DrainSpec, EnforcementBackend, FakeBackend, PluginId, ResidueReport,
};
use plasmosome_ledger::{Closure, DetachReport, Ledger, SealedLedger};
use plasmosome_testkit::builders::{GrantClosure, proxy_map, session_file, uds_socket};

fn attach_detach(
    (mut ledger, mut backend): (Ledger, FakeBackend),
) -> (SealedLedger, FakeBackend, DetachReport) {
    let before = backend.snapshot_os_state().unwrap();
    let prepared = GrantClosure::for_owner(CellOwner {
        cell: CellId::from("attach-detach-cell"),
        plugin: PluginId::from("attach-detach"),
    })
    .hot(session_file("/session"))
    .hot(uds_socket("/run/egressd.uds"))
    .hot(proxy_map("api.example.test", "proxy"))
    .prepare();
    for grant in &prepared {
        ledger.push(grant.backend_effect());
    }
    for grant in prepared {
        backend.grant(grant.operation, grant.kind).unwrap();
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
    let after = backend.snapshot_os_state().unwrap();
    let residue =
        ResidueReport::from_diff(Diff::between(before.state(), after.state()), Vec::new());
    assert_eq!(residue, ResidueReport::Empty, "{residue}");
    assert!(after.grants().is_empty() && after.incomplete().is_empty());
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
