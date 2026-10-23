# Fuzzing

This separate, unpublished workspace searches seven parsers with libFuzzer. Each target calls
the same check body its owning crate replays under `cargo test`. Six wrappers include that body
by path; operator_file calls core's opt-in `fuzzing` feature, which exposes only the same check.

The weekly `fuzz` workflow runs every target for 600 seconds from its committed seed corpus.
It runs on a schedule or an owner's dispatch and never gates a pull request. The runner captures
both child streams privately. On failure it reports only the target, explicit seed, crash input's
SHA-256 and length, and fuzz executable's SHA-256. It uploads no input, log or discovered corpus.

The owner can attempt a private reproduction using that seed and matching executable. A digest
and seed cannot recover the input or guarantee another machine repeats the search. Report a
vulnerability privately through SECURITY.md. Commit a minimized input only with its fix, under
the owning crate's `tests/fuzz/corpus/<target>/` as `crash-<digest>`, and prove its replay fails
before the fix and passes after it. Inputs must be regular files at most 4,096 bytes. Manifest
inputs must also satisfy the existing 49-bracket/brace/dot guard; never save a declaration deeper
than 100 levels.

| Target | Owning crate |
| --- | --- |
| control_line | plasmosome-core |
| manifest | plasmosome-core |
| readiness_reply | plasmosome-membrane |
| hello_reply | plasmosome-membrane |
| cli_reply | plasmosome-cli |
| guest_control_reply | plasmosome-backend |
| operator_file | plasmosome-core |

## Local use and maintenance

An owner-approved local run needs nightly-2026-10-14 and cargo-fuzz 0.13.2 installed first.
For example, after those installations:
```shell
cargo +nightly-2026-10-14 fuzz run --fuzz-dir fuzz manifest -- -seed=20261024 -max_total_time=600 -max_len=4096 -timeout=10
```
That direct local command prints private inputs on a crash. Keep its output private.
The CI runner in plasmosome-tools is for x86_64 Linux; it uses sha256sum and an explicit build
directory, with new inputs and crash files in a fresh private directory.

The pinned stable compiles, lints and formats every wrapper in ordinary CI.
After changing a dependency in core, backend, membrane or cli, refresh this lockfile too:
```shell
cargo update --workspace --manifest-path fuzz/Cargo.toml
```
Keep every shared package at the root lockfile's version. Re-derive dependency records from both
workspaces as docs/dependencies/README.md describes. cargo-deny reads both tracked lockfiles.
The /fuzz Dependabot entry belongs to plasmosome-qtw. plasmosome-16f3 carries red weekly runs
into the security report. Owner-run acceptance stays open until the real runs are recorded.
