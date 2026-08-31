---
id: 005
title: Continuous benchmarks of the kernel operations a user feels
status: accepted
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

### What `cargo bench` needs from the manifests

`cargo bench --workspace -- --quick` works only once every `[lib]` and every `[[bin]]` target in
the workspace carries `bench = false`. Without that, cargo builds each of those targets in bench
profile and runs its libtest harness before it reaches any `harness = false` target, and libtest
rejects `--quick`. The run stops there, so no criterion benchmark starts at all.

This is not a question of whether a crate has tests. A crate with no test in it still fails,
because the harness is linked either way. Setting `bench = false` on a target removes it from
`cargo bench` and nothing else: `cargo test --workspace` still runs every unit test, doc test and
integration test, and `cargo clippy --all-targets` is unaffected. Integration test targets need no
change, because `cargo bench` does not select them.

One command per benchmark also works and needs no manifest change:
`cargo bench --workspace --bench <name> -- --quick` names a single `harness = false` target and
skips the libtest ones. Use it to rerun one benchmark. It is not what CI runs, because CI has to
run all six. `--benches` is not a way out — a lib target counts as a bench target, so
`--benches` selects it too and fails the same way.

The cost of this arrangement is that a crate added to the workspace without `bench = false` on
its lib and bins breaks the bench job again, and no rule in the gate today catches that. Task 006
records the runs that establish all of the above.

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

`reconciler_step` measures a placeholder: `Reconciler::reconcile` is two `BTreeSet` differences
over an observed set that is empty by default, and `plasmosome-core`'s own crate docs call the
reconciler a placeholder. It is filed now as a longitudinal identifier, so that the name is in
the record before the real implementation arrives — expect no signal from it until then.

### Stateful benchmarks use `iter_batched`, never `iter`

`ledger_replay` and `attach_detach` use criterion's `iter_batched` with per-iteration setup that
constructs a fresh `SealedLedger` and a fresh `FakeBackend` for every iteration. Neither may use
`b.iter()`.

Both operations consume what they measure. `SealedLedger::detach` takes `&mut self` and drains
its pending effects as it replays them, and it mutates the backend as well, so revoking the same
handle twice returns `UnknownHandle`. `b.iter()` calls its closure thousands of times against one
fixture: the first call does the work and every call after it does nothing. The reported median
would then be the cost of an empty loop — near zero, and near zero reads as an excellent result.
A stateful operation benchmarked with `iter()` reports the cost of doing nothing.

### CI: run always, gate later

A `bench` job on every PR runs `cargo bench --workspace -- --quick` and writes the medians into
the step summary. Green means the benchmarks compile, run, and finished; it says nothing about
speed. (`clippy --all-targets` in the gate already lint-checks bench code; this job is what
keeps it running.)

The regression gate is explicitly out of scope, and here is the method that unblocks it later:
run the quick suite ten times in CI against one unchanged commit and record every median. The
dispersion figure per benchmark is the inter-quartile range across those ten medians, stated as a
percentage of their median. A future gate flags a change only above three times that percentage.

Swing is relative on purpose. An absolute figure in nanoseconds means nothing when carried across
benchmarks that differ by three orders of magnitude; a percentage means the same thing for the
microsecond ones and the millisecond ones.

The inter-quartile range replaces the obvious choice, which was the largest swing over the ten
runs. A range is a statistic of two points and both of them are the extremes, so one cold-cache
outlier sets the figure for the whole record. It is also self-defeating as a threshold: at three
times an observed range, any range wider than about 66% of baseline lets a genuine 2x regression
through. The inter-quartile range discards the extremes and moves far less between records.

The ten-run record lands in the task's Notes; enabling the gate is its own later task citing
those numbers. Until then the comparison is advisory by construction. Any tighter promise from a
shared runner would be noise dressed as a check.

### Local baselines

Full `cargo bench` runs happen on developer machines. The first baseline is recorded in the
task's Notes with the machine named (chip, core count, memory, toolchain version). A number
without its machine is not a baseline.

## Contract

- The six benchmark names above are stable identifiers.
- `cargo bench --workspace` runs everything and `-- --quick` is the reduced mode CI uses,
  both conditional on `bench = false` on every `[lib]` and `[[bin]]` target.
- criterion output stays in `target/criterion/`, never committed.
- A benchmark that swaps `FakeBackend` for a real backend keeps its name and gains a variant,
  so the fake-vs-real gap stays visible in one place.

## Acceptance

- All six named benchmarks exist and complete via `cargo bench --workspace`, and every `[lib]`
  and `[[bin]]` target in the workspace carries `bench = false`.
- Each benchmark's reported median is non-zero, and each one that takes an input size scales with
  it — `ledger_replay` at 10, 100 and 1000 shows three distinct magnitudes. This is the check
  that catches a stateful benchmark measuring an empty loop.
- The CI `bench` job runs the quick suite on PRs, is green, and shows medians in the step
  summary.
- No CI job fails on a performance number (the gate is not enabled here).
- The ten-run variance record and the first local baseline (machine named) are in the task's
  Notes.
- The gate in the root `AGENTS.md` is green.
