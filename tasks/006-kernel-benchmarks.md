---
id: 006
title: Criterion benchmarks for the six kernel operations, plus the CI bench job
status: planned
priority: 2
specs: [005]
intents: [002]
refs:
  [
    docs/specs/005-kernel-operation-benchmarks.md,
    AGENTS.md,
    Cargo.toml,
    crates/plasmosome-ledger/Cargo.toml,
    crates/plasmosome-ledger/src/lib.rs,
    crates/plasmosome-core/Cargo.toml,
    crates/plasmosome-core/src/manifest.rs,
    crates/plasmosome-core/src/reconciler.rs,
    crates/plasmosome-core/src/session_log.rs,
    crates/plasmosome-backend/Cargo.toml,
    crates/plasmosome-backend/src/fake.rs,
    crates/plasmosome-membrane/Cargo.toml,
    crates/plasmid-sdk/Cargo.toml,
    crates/plasmosome-freeze-checks/Cargo.toml,
    crates/plasmosome-testkit/Cargo.toml,
    crates/plasmosome-testkit/src/builders.rs,
    .github/workflows/ci.yml,
  ]
done_when: >-
  the six named benchmarks from spec 005 run via cargo bench --workspace, a CI
  bench job runs the quick suite on PRs and shows medians in the step summary
  without gating on any number, and the ten-run CI variance record plus a
  machine-named local baseline are appended to this task's Notes.
pr:
evidence:
---

## Why

Spec 005: attach speed is a product property here, so it is measured on every PR — starting with
an honest advisory record, because a failing gate without measured runner variance would be
noise dressed as a check.

## Plan

Do not claim this task until spec 005 is `accepted` **and task 004 is `done`** — the
`attach_detach` benchmark lives in `plasmosome-testkit` and uses its builders.

**Deliverable:** the six benchmarks named in spec 005's table, in the crates that table assigns,
plus the CI `bench` job, plus the two measurement records in `## Notes`.

**Out of scope:** any regression gate that fails CI on a number (a later task enables it, citing
the variance record); benchmarks of real backends (none exist); changing any kernel API to make
it easier to benchmark — stop and report instead.

**Read only the files in `refs:` and this task.** Spec 005 decides the harness (criterion, as a
workspace dev-dependency), the six names, the quick mode, and the variance method. The names are
longitudinal identifiers — spell them exactly as the spec does.

Steps:

1. Add criterion to `[workspace.dependencies]` and as a dev-dependency of plasmosome-ledger,
   plasmosome-core, and plasmosome-testkit; add the `benches/` targets with `harness = false`.
2. Add `bench = false` to every `[lib]` and every `[[bin]]` target in the workspace — all six
   existing crates plus the testkit, and the two bins `plasmid` and `membraned`. Without this
   `cargo bench -- --quick` exits 101 before any benchmark runs. Spec 005's "What `cargo bench`
   needs from the manifests" explains why, and `## Notes` below has the proof runs.
3. Write the six benchmarks per the spec's table: `attach_detach` (testkit), `ledger_replay`
   at 10/100/1000 (ledger), `manifest_parse`, `reconciler_step`, `session_log_append`,
   `session_log_read` (core, session log in a `TempDir`).
4. `ledger_replay` and `attach_detach` use `iter_batched` with per-iteration setup building a
   fresh `SealedLedger` and a fresh `FakeBackend`. Do not use `b.iter()` for either.
   `SealedLedger::detach` takes `&mut self`, drains as it replays, and mutates the backend, so
   under `iter()` every iteration after the first measures an empty loop and reports a median
   near zero — which reads as a great result.
5. Confirm each runs: `cargo bench --workspace` completes locally, every median is non-zero, and
   `ledger_replay` at 10/100/1000 shows three distinct magnitudes.
6. Add the CI `bench` job: `cargo bench --workspace -- --quick`, medians into
   `$GITHUB_STEP_SUMMARY`, one command per step.
7. Rerun the bench job ten times against one unchanged commit; append every median to `## Notes`,
   with the per-benchmark inter-quartile range across those ten medians as a percentage of their
   median. Not the range — spec 005 says why.
8. Run the full suite locally once; append the baseline with chip, core count, memory, and
   toolchain version to `## Notes`.

| Check | Proves |
| --- | --- |
| `cargo bench --workspace` completes with all six names present | the benchmarks exist and run |
| every median non-zero, `ledger_replay` distinct at 10/100/1000 | no benchmark is measuring an empty loop after its fixture was consumed |
| green CI `bench` job with medians in the summary | they cannot silently rot, and the numbers are visible |
| no job fails on a performance number | the gate stays advisory as the spec requires |
| ten-run variance record in Notes, as a relative inter-quartile range | a future gate threshold can be derived, not invented, and does not move with one outlier |
| machine-named local baseline in Notes | the first reference point is honest about where it came from |

**Done when:** `done_when:` above holds and the gate in the root `AGENTS.md` passes.

STOP when done — do not start the next piece of work.

## Notes

### 2026-08-30 — why the bench invocation carries a manifest change

`cargo bench --workspace -- --quick`, as this task was first written, exits 101 without running
a single benchmark. Cargo builds every `[lib]` and `[[bin]]` target in bench profile and runs its
libtest harness before it reaches any `harness = false` target, and libtest has no `--quick`
option. The fix is `bench = false` on each of those targets; the command itself is then correct.

Measured on a scratch two-then-three crate workspace outside this repository, built to mirror its
shape: criterion 0.7.0 with a `harness = false` bench, `#[cfg(test)]` modules in the libs, an
integration test per crate, and later a crate carrying a bin and no tests at all. rustc 1.97.1,
cargo 1.97.1, macOS.

| # | State | Command | Exit |
| --- | --- | --- | --- |
| T1 | no `bench = false` anywhere | `cargo bench --workspace -- --quick` | **101** |
| T2 | no `bench = false` anywhere | `cargo bench --workspace --benches -- --quick` | **101** |
| T3 | no `bench = false` anywhere | `cargo bench --workspace --bench probe -- --quick` | 0 |
| T4 | `bench = false` on both libs | `cargo bench --workspace -- --quick` | 0 |
| T5 | `bench = false` on both libs | `cargo bench --workspace --benches -- --quick` | 0 |
| T6 | third crate added, has a bin and no tests, no `bench = false` | `cargo bench --workspace -- --quick` | **101** |
| T7 | that crate's `[lib]` set, its `[[bin]]` not | `cargo bench --workspace -- --quick` | **101** |
| T8 | that crate's `[lib]` and `[[bin]]` both set | `cargo bench --workspace -- --quick` | 0 |

The failure text is the same every time: `error: Unrecognized option: 'quick'`, preceded by
`Running unittests src/lib.rs` (T1, T2, T6) or `Running unittests src/main.rs` (T7).

Four things that are easy to get wrong, each settled by a run above:

- **`--benches` does not help** (T2). A lib target is a bench target by default, so `--benches`
  selects it and fails identically.
- **A crate with no tests still fails** (T6). The libtest harness is linked whether or not any
  test exists, so `bench = false` is needed on every lib, not only the ones with tests.
- **Bins need it too** (T7). They are selected by `cargo bench` exactly as libs are. In this
  repository that means `plasmid` and `membraned`.
- **Integration tests need nothing.** `cargo bench` never selected them in any state; only
  `benches/probe.rs` ran in T4.

`bench = false` costs nothing elsewhere. In state T8: `cargo test --workspace` exit 0, running
every unit, integration and doc test; `cargo clippy --workspace --all-targets -- -D warnings`
exit 0. And `--quick` is worth passing — the same suite took 0.46s with it against 10.46s
without.

T3 is the fallback for rerunning one benchmark by name without touching any manifest. CI cannot
use it, because CI has to run all six.
