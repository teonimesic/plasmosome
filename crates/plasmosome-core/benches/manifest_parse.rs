use criterion::{Criterion, criterion_group, criterion_main};
use plasmosome_core::PlasmidManifest;
use std::hint::black_box;

const MANIFEST: &str = r#"
id = "github-pr"
description = "Read and open pull requests."
version = "1.2.3"
impl.wasm = "components/github-pr.wasm"

[requires]
capabilities = ["network:hosts=api.github.com"]

[provides]
"github:tools" = { tools = { "pr.read" = "Read a pull request.", "pr.open" = "Open a pull request." } }

[network]
hosts = ["api.github.com"]
ports = [443]
pin_cidrs = ["140.82.112.0/20"]

[lifecycle]
drain_ms = 750
"#;

fn bench(c: &mut Criterion) {
    c.bench_function("manifest_parse", |b| {
        b.iter(|| black_box(PlasmidManifest::parse(black_box(MANIFEST)).unwrap()))
    });
    let two_bindings = MANIFEST.replace(
        "\n[network]",
        "\"github:issues\" = { tools = { \"issue.read\" = \"Read an issue.\" } }\n\n[network]",
    );
    let provides = PlasmidManifest::parse(&two_bindings).unwrap().provides;
    assert_eq!(provides, ["github:tools", "github:issues"]);
    c.bench_function("manifest_parse_two_bindings", |b| {
        b.iter(|| black_box(PlasmidManifest::parse(black_box(&two_bindings)).unwrap()))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
