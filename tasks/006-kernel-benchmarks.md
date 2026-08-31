---
id: 006
title: Criterion benchmarks for the six kernel operations, plus the CI bench job
status: todo
priority: 2
specs: [005]
intents: [002]
refs:
  [
    docs/specs/005-kernel-operation-benchmarks.md,
    crates/plasmosome-ledger/src/lib.rs,
    crates/plasmosome-core/src/manifest.rs,
    crates/plasmosome-core/src/reconciler.rs,
    crates/plasmosome-core/src/session_log.rs,
    crates/plasmosome-backend/src/fake.rs,
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
2. Write the six benchmarks per the spec's table: `attach_detach` (testkit), `ledger_replay`
   at 10/100/1000 (ledger), `manifest_parse`, `reconciler_step`, `session_log_append`,
   `session_log_read` (core, session log in a `TempDir`).
3. Confirm each runs: `cargo bench --workspace` completes locally.
4. Add the CI `bench` job: `cargo bench --workspace -- --quick`, medians into
   `$GITHUB_STEP_SUMMARY`, one command per step.
5. Rerun the bench job ten times against one unchanged commit; append every median and the
   largest per-benchmark swing to `## Notes`.
6. Run the full suite locally once; append the baseline with chip, core count, memory, and
   toolchain version to `## Notes`.

| Check | Proves |
| --- | --- |
| `cargo bench --workspace` completes with all six names present | the benchmarks exist and run |
| green CI `bench` job with medians in the summary | they cannot silently rot, and the numbers are visible |
| no job fails on a performance number | the gate stays advisory as the spec requires |
| ten-run variance record in Notes | a future gate threshold can be derived, not invented |
| machine-named local baseline in Notes | the first reference point is honest about where it came from |

**Done when:** `done_when:` above holds and the gate passes: `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`,
`./.githooks/provenance-guard`.

STOP when done — do not start the next piece of work.

## Notes
