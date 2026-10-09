---
id: 004
title: CI proves each crate alone, the workspace together on Linux and macOS, and the MSRV
status: accepted
intents: [002]
---

## Behavior

CI answers four questions: does each crate still compile and pass its tests on its own, does the
workspace still hold together, does it still compile and pass its tests on macOS, and is the
declared minimum Rust version (1.96) still true. A green run means all four, and the list of
crates CI checks cannot drift from the workspace without a test failing. CI also reports how much
of the code the tests execute; no coverage percentage ever fails a run.

The `gates` job proves the workspace together. It does not prove a crate stands on its own — a
crate can lean on a sibling's dev-dependencies or on workspace build order and stay green for
months.

The macOS and coverage jobs come from the 2026-10-08 audit of what intent 002 has delivered.
Every job ran on `ubuntu-latest`, so code compiled only for macOS — `darwin_group.c`, which the
membrane's build script compiles only for a macOS target, and the membrane's
`cfg(target_os = "macos")` and `cfg(not(target_os = "linux"))` paths — was never compiled or
tested by CI, although macOS on Apple silicon is the product's first target. Nothing measured
coverage either, though intent 002 asks for "a great test system, not a passing one".

Publishing is deliberately not here. The strongest per-crate check, `cargo package`, needs
version fields on path dependencies and a decision to publish at all; that is spec 007 and it is
blocked on owner decisions. This spec ships the strongest check available short of that.

## Design

### Jobs

- **`gates`** — the integration gate. This spec sets only its ignored-test report and the
  test-output capture that feeds it (below); its other steps belong to the specs that add them.
  When spec 003's testkit lands, its cross-crate tests run here automatically under `cargo test`.
- **`crate`** — a matrix job, one entry per workspace member, hardcoded in `ci.yml`. Two steps
  per entry, one command per step (the existing `ci.yml` rule: a multi-command step can pass
  while a command inside it failed):
  1. `cargo build -p <crate> --all-targets`
  2. `cargo test -p <crate>`
- **`msrv`** — `dtolnay/rust-toolchain@1.96` (the pinned toolchain action, not `@stable`), then
  one step: `cargo check --workspace --all-targets`.
- **`macos`** — runs on `macos-26`, the standard arm64 runner. The label pins the major macOS
  version, not the image: GitHub updates the image about weekly, so the minor version and the
  clang that compiles `darwin_group.c` can change without a commit here. It has no `paths`
  filter, because a required check whose workflow does not run stays pending. The toolchain and
  cache actions are those of `gates`, and its clippy and test steps set `CFLAGS=-Werror`: without
  it a C warning prints as a `cargo:warning` line that clippy does not fail on. One command per
  step:
  1. `cargo clippy --workspace --all-targets -- -D warnings`
  2. `cargo test --workspace --exclude plasmosome-guards`
  3. `cargo test -p plasmosome-guards`
  4. the ignored-test report

  Steps 2 and 3 are the split `gates` uses once #135 merges, so each guard test runs once per
  job. The guards run here too because their hook-script tests meet BSD userland on macOS. There
  is no separate build step: these commands compile what they need, and a compile failure fails
  them. fmt, the provenance and attribution checks and, once #135 merges, the Python tool suites
  run in `gates` only; the crate matrix and `msrv` run on Linux only.
- **`coverage`** — runs on `ubuntu-latest`. The stable toolchain with `llvm-tools-preview`, then
  `cargo-llvm-cov` at an exact version installed with `--locked`, the way `audit.yml` pins
  `cargo-audit`. That pins the tool, not the instrumentation: `toolchain: stable` floats, so a
  Rust release can still move the numbers. One command per step: `cargo llvm-cov clean
  --workspace`, so no earlier run's data is counted; `cargo llvm-cov --workspace --exclude
  plasmosome-guards --no-report`, the one measured run; then reports from it, the tool's summary
  table to the step summary and `lcov.info` as a workflow artifact. No step carries
  `continue-on-error` or `|| true`, and no command carries a `--fail-under-*` or
  `--fail-uncovered-*` flag.

The workflow cancels a superseded run only for a pull request. Every push to main runs to
completion, so main keeps the reference runs that the coverage report and the owner question
below rely on.

### What the matrix does and does not prove

`cargo -p` inside a workspace still shares the lockfile, and features unify per invocation. With
no feature flags declared anywhere today, the matrix catches the real current risks: a crate
whose tests only compile because a sibling's dev-dependency happens to be in the graph, and a
crate that breaks alone but not in a workspace-wide build. It is not full standalone packaging —
that is `cargo package`, deferred to spec 007. This limit is stated here so nobody mistakes the
matrix for the packaging check.

One entry proves less than the rest: a green `crate (plasmosome-guards)` does not show that crate
standing on its own. Its tests shell out to `cargo` and run it against the workspace root, so they
read the whole workspace by construction and cannot do otherwise.

### What a hosted Mac cannot run

GitHub's arm64 macOS runners are themselves virtual machines, and
[GitHub states](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
that nested virtualization is not supported on them. A test that boots a guest — through
Hypervisor.framework, Virtualization.framework or libkrun — cannot pass there, and neither can
one that needs an entitlement or privilege the runner lacks. On 2026-10-09 at `e1cf004` no
workspace test needed any of these, so at that commit every test runs on `macos`.

A test that needs a prerequisite asserts it, so a missing prerequisite fails the test. Where a CI
runner cannot supply it, the test is marked `#[ignore = "needs <prerequisite>"]`, and the
workspace lint table denies `clippy::ignore_without_reason`, so a bare `#[ignore]` fails clippy.
It never returns early when the prerequisite is missing: it would report `ok` having tested
nothing, and that pass reads the same as a real one. No mechanical check sees an early return, so
review holds that line.

The ignored-test report runs in `gates` and in `macos`. Each `cargo test` step runs under an
explicit `shell: bash`, which GitHub runs with `-eo pipefail`, and appends its output to one file
through `tee -a`, so a failing test still fails its step. The report step runs with
`if: always()`. It reads the harness's own `test <name> ... ignored, <reason>` lines from that
file and writes each ignored test's name and reason to the step summary, or says none was
ignored. An ignored doctest, which the lint cannot reach, is listed with `no reason given`. A test
compiled out by `cfg` for the other OS is not a skip; the other OS's job runs it.

### Coverage is a report, not a gate

The `coverage` job goes red only when a test or the tool fails, never on a percentage. That is a
choice made here: a failing test says the product is broken, and a lower percentage does not say
whether it is. Spec 005 made the same choice for benchmarks, run always and gate later; a
threshold here would likewise be a later decision with its own argument.

The step summary names its platform and the `plasmosome-guards` exclusion beside the tool's
table. On Linux the `cfg(target_os = "macos")` and `cfg(not(target_os = "linux"))` lines are not
compiled, so they are absent from the count rather than counted as uncovered; a local run on a
Mac includes them, so the two totals differ. Linux is chosen because macOS runners are the scarce
ones (see Cost), and an advisory job should not queue ahead of a required one.
`plasmosome-guards` is excluded: its tests read the repository rather than exercise product code,
and they drive nested cargo builds. The report is not compared with the base branch. The latest
completed run on main is the reference; a comparison in CI would need another run's artifacts and
a wider token.

### Which red blocks a merge

Two layers decide. Branch protection blocks mechanically; on 2026-10-09 it required only `gates`.
The pr-review merge gate requires CI to be green for the validated head, so a red job anywhere in
`ci.yml` blocks a merge by rule, `coverage` included; only a failing test or tool run turns
`coverage` red. Adding a check to branch protection is an owner admin action. `macos` is meant to
be added, because a red `macos` means the product is broken where it ships first; `coverage` is
meant to stay out. No review rule cites the coverage number: a reviewer need not read, answer or
quote it.

**Owner question:** add `macos` to branch protection once `plasmosome-rw6` and `plasmosome-5r3`
are closed and `macos` has been green on the PR that introduces it and on main? Both are timing
failures a freshly built three-core runner meets on every run, rw6's first-exec cost against
five-second deadlines and 5r3's race under load, so requiring `macos` earlier turns known flakes
into merge blocks.

### Keeping the matrix honest

A hardcoded matrix rots when a crate is added or renamed. A new guard in `plasmosome-guards`
reads `Cargo.toml`'s `members` and `.github/workflows/ci.yml`, and fails when the matrix and the
member list differ in either direction. Per that crate's own rules the check is mutation-tested:
remove a matrix entry, watch it fail, restore it.

### Cost

The matrix multiplies CI minutes by roughly the member count, minus cache hits.
`Swatinem/rust-cache` is reused per matrix entry with the crate name in the cache key. Accepted:
this repository is small and correctness of the per-crate promise is worth more than the
minutes. Revisit if a run ever exceeds fifteen minutes.

`macos` and `coverage` cost no money: GitHub's standard runners, macOS included, are free on a
public repository. Their cost is time. On 2026-10-09 the macOS runner had three cores to the
Linux runner's four, and the account could run at most five macOS jobs at once, shared with its
other repositories, so with many PRs open `macos` waits for a runner. For scale, `gates` took
61 s with a warm cache on main at `a0a5411` on 2026-10-09; the first `macos` and `coverage`
timings go in the notes of the tasks that add them. `coverage` builds the workspace a second
time, instrumented, plus the pinned tool on a cache miss. On 2026-10-09 at `a0a5411`, one
`cargo llvm-cov` 0.6.21 run over the same crates, on an M3 Max with a cold target, took 93 s and
measured 93.09% of lines, macOS lines included. The fifteen-minute revisit applies to every job.

## Contract

- Job names: `gates`, `crate (<member>)` one per workspace member, `msrv`, `macos`, `coverage`.
- A green `crate (<member>)` run promises: the crate and all its targets build via `-p`, and its
  tests pass, without any sibling named on the command line.
- A green `msrv` run promises: the workspace type-checks on Rust 1.96 exactly.
- A green `macos` run promises: on macOS 26 on Apple silicon, every workspace target passes clippy
  with warnings denied, `cfg(target_os = "macos")` code included; `darwin_group.c` compiles with
  its warnings treated as errors; and every test that is not ignored builds and passes. It
  promises nothing a booted guest would show, nothing about other macOS versions (spec 001 admits
  macOS 14 and later) and nothing about Intel Macs.
- A test that needs a prerequisite asserts it; one a CI runner cannot supply is
  `#[ignore = "needs <prerequisite>"]`. `gates` and `macos` each name every ignored test and its
  reason in their step summary, or say none was.
- `coverage` turns red only on a failing test or tool run, never on a percentage. It adds no
  permission beyond the workflow's `contents: read` and pins `cargo-llvm-cov` to an exact version.
- The guard's name states its clause, e.g. `ci_matrix_matches_workspace_members`.

## Acceptance

Each item names the task that delivers it: plasmosome-005 the matrix, `msrv` and the guard;
plasmosome-6ja `macos` and the ignored-test report; plasmosome-52e `coverage`.

- (005) `ci.yml` has the `crate` matrix job covering every workspace member at merge time, two
  steps per entry, one command per step.
- (005) `ci.yml` has the `msrv` job pinned to 1.96 and it is green.
- (005) The guard fails when a workspace member is missing from the matrix or the matrix names a
  non-member, and the mutation test (entry removed, failure observed, entry restored) is recorded
  in the PR.
- (6ja) `ci.yml` has the `macos` job on `macos-26` with no `paths` filter, `CFLAGS=-Werror` on its
  clippy and test steps, and the four steps above, one command per step. The workflow cancels
  superseded runs only for pull requests, and the workspace denies `clippy::ignore_without_reason`.
- (6ja) On throwaway commits, each linked from the PR: an assertion broken in a
  `cfg(target_os = "macos")` test turns `macos` red while `gates` stays green; a clippy warning
  only in `cfg(target_os = "macos")` code fails the `macos` clippy step; an unused variable in
  `darwin_group.c` fails `macos`; a bare `#[ignore]` fails clippy; and a test that asserts a
  prerequisite the runner lacks, without `#[ignore]`, fails its test step.
- (6ja) On a throwaway commit with one test marked `#[ignore = "needs a hypervisor"]` and one
  failing test, `gates` and `macos` both go red at their test step and both summaries still name
  the ignored test with that reason. The introducing PR's own runs name exactly the tests ignored
  in each run, or say there are none.
- (6ja) The PR that introduces `macos` puts the owner question above to the owner.
- (52e) `coverage` runs the clean and measuring commands above, with `--exclude plasmosome-guards`
  and nothing narrower; uploads `lcov.info`; adds no `permissions` entry; installs
  `cargo-llvm-cov` at an exact version with `--locked`; and carries no `continue-on-error`,
  `|| true`, `--fail-under-*` or `--fail-uncovered-*`.
- (52e) On throwaway commits, each linked from the PR: deleting the only test that reaches some
  lines lowers that file's reported coverage while `coverage` stays green, and a failing test
  turns `coverage` red.
- (each) Every new job is green on the PR that introduces it, and the gate in the root
  `AGENTS.md` is green.
