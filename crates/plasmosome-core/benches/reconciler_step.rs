use criterion::{Criterion, criterion_group, criterion_main};
use plasmosome_core::reconciler::DesiredCell;
use plasmosome_core::{CellId, DesiredState, GenomeName, MockMode, PlasmidRecord, Reconciler};
use std::collections::BTreeMap;
use std::hint::black_box;

fn bench(c: &mut Criterion) {
    let desired = DesiredState {
        generation: 1,
        cells: [(
            CellId::from("cell-1"),
            DesiredCell {
                genome: Some(GenomeName::from("researcher")),
                plasmids: vec![PlasmidRecord {
                    plasmid: "github-pr".into(),
                    mock: MockMode::Simulate,
                }],
            },
        )]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
    };
    let reconciler = Reconciler::new(desired);
    c.bench_function("reconciler_step", |b| {
        b.iter(|| black_box(black_box(&reconciler).reconcile()))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
