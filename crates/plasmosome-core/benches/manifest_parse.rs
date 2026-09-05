use criterion::{Criterion, criterion_group, criterion_main};
use plasmosome_core::PlasmidManifest;
use std::hint::black_box;

const MANIFEST: &str = r#"
id = "github-pr"
version = "1.2.3"
requires = ["git"]
provides_tools = ["pr.read", "pr.open"]
drain_ms = 750
[network]
hosts = ["api.github.com"]
ports = [443]
"#;

fn bench(c: &mut Criterion) {
    c.bench_function("manifest_parse", |b| {
        b.iter(|| black_box(PlasmidManifest::parse(black_box(MANIFEST)).unwrap()))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
