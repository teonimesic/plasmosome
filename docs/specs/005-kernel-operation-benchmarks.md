---
id: 005
title: Continuous benchmarks of the kernel operations a user feels
status: draft
intents: [002]
---

## Behavior

The operations that decide whether this kernel feels like milliseconds or seconds — attach,
detach-and-replay, manifest parse, reconciler step, session-log append and read — each get a
named benchmark. `cargo bench` runs them all locally with full statistics. CI runs them on every
PR in quick mode so they can never silently rot, and reports the numbers; it does not fail on a
regression yet, because a failing gate needs a threshold and a threshold needs measured CI
variance first. Producing that variance measurement is part of this spec.

The intent behind this is direct: a kernel that takes seconds to attach a capability is a
different product from one that takes milliseconds, so speed is measured continuously, not
checked once. No target number is asserted here — any number written before the first
measurement would be invented. The first recorded baseline, from a named machine, becomes the
reference the next change is read against.

These benchmarks measure kernel bookkeeping through `FakeBackend`, not OS enforcement — no real
backend exists yet. That is stated on the benchmark itself so nobody reads a fake-backed attach
time as an enforcement time. When a real backend lands, the same benchmark runs against it and
the gap between the two is the enforcement cost, measured rather than guessed.

## Design

### Harness: criterion

The things measured are in-process Rust operations at microsecond-to-millisecond scale. That
rules the alternatives out:

- **hyperfine** times whole processes. Process spawn and runtime startup would drown a
  microsecond operation. It is the right tool for spec 006's cross-environment benchmark, and
  the wrong one here.
- **A hand-rolled loop-and-`Instant` harness** gives no warmup, no outlier rejection, no
  statistics. Its numbers move when the laptop warms up.
- **iai-callgrind** counts instructions under valgrind — stable, but Linux-only, and this
  repository's first target is macOS.

criterion runs in-process, does warmup and outlier analysis, works on macOS, and writes
machine-readable results to `target/criterion/`. It becomes a workspace dev-dependency.

### The named benchmarks

Names are longitudinal identifiers: renaming one breaks the record and is a deliberate act, not
a cleanup.

| Benchmark | Crate (`benches/`) | Measures |
| --- | --- | --- |
| `attach_detach` | `plasmosome-testkit` | grant through `FakeBackend`, effects into a `Ledger`, revoke, LIFO replay, residue check — the full attach/detach path minus real enforcement |
| `ledger_replay` | `plasmosome-ledger` | replay of 10 / 100 / 1000 effects |
| `manifest_parse` | `plasmosome-core` | parsing a representative full manifest |
| `reconciler_step` | `plasmosome-core` | one desired-vs-observed reconcile pass |
| `session_log_append` | `plasmosome-core` | one append to a log in a `TempDir` |
| `session_log_read` | `plasmosome-core` | reading a 1000-event log back |

`attach_detach` lives in the testkit because it crosses crates; it therefore needs spec 003
merged first. The rest are single-crate and could land independently, but they ship together as
one unit of work.

### CI: run always, gate later

A `bench` job on every PR runs `cargo bench --workspace -- --quick` and writes the medians into
the step summary. Green means the benchmarks compile, run, and finished; it says nothing about
speed. (`clippy --all-targets` in the gate already lint-checks bench code; this job is what
keeps it running.)

The regression gate is explicitly out of scope, and here is the method that unblocks it later:
run the quick suite ten times in CI against one unchanged commit, record every median, and take
the largest swing per benchmark. A future gate flags a change only above three times that swing.
The ten-run record lands in the task's Notes; enabling the gate is its own later task citing
those numbers. Until then the comparison is advisory by construction. Any tighter promise from a
shared runner would be noise dressed as a check.

### Local baselines

Full `cargo bench` runs happen on developer machines. The first baseline is recorded in the
task's Notes with the machine named (chip, core count, memory, toolchain version). A number
without its machine is not a baseline.

## Contract

- The six benchmark names above are stable identifiers.
- `cargo bench --workspace` runs everything; `-- --quick` is the reduced mode CI uses.
- criterion output stays in `target/criterion/`, never committed.
- A benchmark that swaps `FakeBackend` for a real backend keeps its name and gains a variant,
  so the fake-vs-real gap stays visible in one place.

## Acceptance

- All six named benchmarks exist and complete via `cargo bench --workspace`.
- The CI `bench` job runs the quick suite on PRs, is green, and shows medians in the step
  summary.
- No CI job fails on a performance number (the gate is not enabled here).
- The ten-run variance record and the first local baseline (machine named) are in the task's
  Notes.
- The gate is green: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo fmt --all -- --check`, `./.githooks/provenance-guard`.
