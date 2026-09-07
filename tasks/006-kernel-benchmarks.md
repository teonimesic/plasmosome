---
id: 006
title: Criterion benchmarks for the six kernel operations, plus the CI bench job
status: in_review
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
pr: https://github.com/teonimesic/plasmosome/pull/86
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
4. Both stateful benchmarks use `iter_batched`, never `b.iter()`. `ledger_replay` setup builds a
   populated `SealedLedger` and fresh `FakeBackend`; its measured routine replays the ledger.
   `attach_detach` setup builds an empty `Ledger` and fresh `FakeBackend`; its measured routine
   grants capabilities, pushes their effects, closes the ledger, replays it, and checks residue.
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

### 2026-09-05 — local and ten-run records

The previous author recorded a complete `cargo bench --workspace` run finishing in 77.79 seconds
and a quick suite in 5.76 seconds. These are historical wall times, not proof for the corrected
timing boundaries below. No per-benchmark full-run baseline or exact measured commit was recorded.

Machine baseline: Mac15,8, Apple arm64, 16 logical CPUs, 64 GiB RAM, rustc 1.97.1
(8bab26f4f, 2026-07-14), cargo 1.97.1 (c980f4866, 2026-06-30).

The previous author recorded ten consecutive local quick runs of
`cargo bench -p plasmosome-ledger --bench ledger_replay -- --quick` as the following values
(microseconds). The original record called these medians, but retained neither Criterion JSON nor
an exact commit SHA; their estimator cannot be verified. They are not the ten CI runs of the
complete suite required by spec 005.

| run | 10 effects | 100 effects | 1000 effects |
| ---: | ---: | ---: | ---: |
| 1 | 2.8711 | 33.7330 | 347.3100 |
| 2 | 3.0094 | 33.3620 | 354.5000 |
| 3 | 2.9422 | 33.2460 | 355.3000 |
| 4 | 2.9692 | 34.0880 | 359.4100 |
| 5 | 3.1120 | 33.4910 | 351.8800 |
| 6 | 2.9244 | 32.8770 | 356.1300 |
| 7 | 2.9072 | 33.7240 | 344.6900 |
| 8 | 2.9724 | 32.6500 | 344.7800 |
| 9 | 2.9117 | 32.9040 | 355.2800 |
| 10 | 2.9023 | 33.2450 | 342.7300 |

Recomputed from the ten recorded values per column, using Python's
`statistics.quantiles(values, n=4, method="inclusive")`: sort each column and linearly interpolate
at zero-based ranks `(n - 1) * 0.25` and `(n - 1) * 0.75` for Q1 and Q3. The median averages the
middle two values; IQR is Q3 minus Q1; relative IQR is `100 * IQR / median`.

| benchmark | median of recorded values | IQR | relative IQR |
| --- | ---: | ---: | ---: |
| `ledger_replay/10` | 2.9333 µs | 0.063275 µs | 2.16% |
| `ledger_replay/100` | 33.3040 µs | 0.676500 µs | 2.03% |
| `ledger_replay/1000` | 353.1900 µs | 9.882500 µs | 2.80% |

The first full run exposed a file-descriptor exhaustion bug in the append fixture: Criterion's
`SmallInput` batches retained too many open `SessionLog` files. Switching that stateful fixture
to `BatchSize::PerIteration` fixed the harness; the subsequent full run passed. This is why the
append benchmark uses per-iteration setup even though its operation is otherwise independent.

### Handoff — 2026-09-05

Implemented on `task-006-kernel-benchmarks` from `origin/main`. Added Criterion 0.7 workspace
dependency, six named harness-free benchmarks, `bench = false` for every workspace library and
binary target, and an advisory PR `bench` job that publishes benchmark medians to
`GITHUB_STEP_SUMMARY`. The stateful `ledger_replay` and `attach_detach` fixtures use
`iter_batched`; `session_log_append` uses `PerIteration` to avoid retaining open temporary log
files across setup batches.

Verification: RED-first `cargo bench --workspace -- --quick` initially failed on the missing
`DesiredCell` import; after the import fix it passed (5.76s wall time). A first full
`cargo bench --workspace` exposed the file-descriptor issue documented above; the rerun passed
in 77.79s wall time. `cargo test --workspace` passed, and
`cargo clippy --workspace --all-targets -- -D warnings` plus `cargo fmt --all -- --check` passed.

Commit: see the task branch commit history; no generated `target/criterion/` files are tracked.

### 2026-09-07 — review corrections; measurement pending

The reconciler fixture is now constructed once before timing; each iteration calls the
non-mutating `reconcile()` without cloning the desired state or constructing a reconciler.
The append fixture uses `iter_batched_ref` with `PerIteration`, so Criterion drops both the log
and its temporary directory after stopping the measurement, without accumulating open files.

The CI summary reads `median.point_estimate` from each of the eight expected
`target/criterion/<name>/new/estimates.json` files and prints nanoseconds in both the job log and
step summary. The old text parser emitted the center estimate's unit and upper bound for the
actual CI lines, which include the benchmark name. Merely shifting the fields would still not
produce a median: Criterion 0.7 reports a slope estimate, or a mean when no slope exists, in its
console `time:` interval. Missing result files now fail summary publication rather than silently
omitting a benchmark. No performance threshold is enforced.

All four existing review findings are addressed in source or the corrected historical table.
No benchmark, build, test, formatter, or linter was run during this editing batch. The historical
gate claims above do not validate these corrections. Independent review is still required; no
independent review comment was present on PR #86 when this work began.

Before readiness or merge, run the full local suite on the corrected head and record all eight
JSON median estimates with the exact commit, chip, core count, memory, and toolchain. Run the CI
quick suite ten times against that same unchanged head, record the run URLs and all eight medians
from the summary logs, then calculate per-benchmark relative IQR with the method above. Those two
measurement records remain pending, as does the root gate on the corrected head.

The existing `attach_detach` benchmark also timed only detach: all grants and ledger recording
ran in setup, and there was no residue check. That followed the original setup instruction in
both this plan and spec 005, but contradicted the spec's full attach/detach contract. Main directed
the correction to follow that accepted contract: only an empty ledger and backend are set up
outside timing; grants, recording, closure, detach/replay, and the existing
`ResidueReport::from_diff` check now run inside it. The plan and spec setup paragraph are aligned
with that contract. This changes the meaning of the old attach result, so it also needs a fresh
baseline, not comparison against the historical detach-only timing.

### 2026-09-07 — independent review: exclude retained fixture destruction

The independent reviewer found that `ledger_replay` still consumed its fixture and returned
`()`. `detach` advances the replay cursor but retains the ledger's effect storage and the fake
backend's grant history, so dropping those collections inside the measured closure added
size-dependent teardown to replay. `attach_detach` had the same ownership boundary.

Both `iter_batched` routines now return the sealed ledger, backend, and detach report.
Criterion 0.7 retains these outputs until after `measurement.end`, excluding fixture and report
destruction while still timing the requested operations. The attach routine still checks residue
before returning its output.

Main's full run on `cf26471d7adb009a8f89adc54b6aaa5d43002a3a` completed in 89.54 seconds
and its summary matched the eight JSON medians, but it preceded this fix. Its baseline and the
interrupted CI variance collection are historical, not acceptance proof for the final timing
boundaries. Fresh local and ten-run CI measurements must follow this correction and the completed
independent review.

The reviewer also found that the manifest fixture used ignored root fields for requirements,
tools, and drain timeout. It now follows the existing `GITHUB_PR` parser fixture's table grammar:
`requires.capabilities`, a tool binding under `provides`, and `lifecycle.drain_ms`, with a WASM
implementation and pinned network range. The old fixture exercised only ID, version, and network
parsing, not the representative manifest promised by spec 005; its median also needs replacement.
