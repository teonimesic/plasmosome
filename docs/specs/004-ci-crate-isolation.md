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

Before `macos`, every job ran on `ubuntu-latest`, so code compiled only for macOS —
`darwin_group.c`, which the membrane's build script compiles only for a macOS target, and the
membrane's `cfg(target_os = "macos")` and `cfg(not(target_os = "linux"))` paths — was never
compiled or tested by CI, although macOS on Apple silicon is the product's first target. Nothing
measured coverage either, though intent 002 asks for "a great test system, not a passing one".

Publishing is deliberately not here. The strongest per-crate check, `cargo package`, needs
version fields on path dependencies and a decision to publish at all; that is spec 007 and it is
blocked on owner decisions. This spec ships the strongest check available short of that.

## Design

### Jobs

- **`gates`** — the integration gate; the testkit's cross-crate tests run here. This spec sets
  only its ignored-test report, the `--no-fail-fast` and output capture on its test steps that
  feed it, and the `ignore_without_reason` flag on its clippy step (below); its other steps
  belong to the specs that add them.
- **`crate`** — a matrix job, one entry per workspace member, hardcoded in `ci.yml`. Two steps
  per entry, one command per step (the existing `ci.yml` rule: a multi-command step can pass
  while a command inside it failed):
  1. `cargo build -p <crate> --all-targets`
  2. `cargo test -p <crate>`
- **`msrv`** — `dtolnay/rust-toolchain@1.96` (the pinned toolchain action, not `@stable`), then
  one step: `cargo check --workspace --all-targets`.
- **`macos`** — runs on `macos-26`, the standard arm64 runner. The label pins the major macOS
  version, not the image: GitHub updates the image about weekly, so the minor version and the
  clang that compiles `darwin_group.c` can change without a commit here. The toolchain and cache
  actions are those of `gates`. Steps:
  1. `cargo clippy --workspace --all-targets -- -D warnings -D clippy::ignore_without_reason`
  2. `cargo test --workspace --exclude plasmosome-guards`
  3. `cargo test -p plasmosome-guards`
  4. the ignored-test report

  Steps 2 and 3 are the split `gates` uses, so each guard test runs once per job; the guards run
  here too because their hook-script tests meet BSD userland on macOS. There is no separate build
  step: these commands compile what they need, and a compile failure fails them. fmt, the
  provenance and attribution checks and the Python tool suites run in `gates` only; the crate
  matrix and `msrv` run on Linux only.

  A C warning in `darwin_group.c` fails the build, in `macos` and in the root gate on a Mac;
  clippy alone passes it as a `cargo:warning` line. The preferred way is `cc`'s
  `warnings_into_errors(true)` in the membrane's `build.rs`, which reaches that file alone, where
  a job-wide `CFLAGS` would reach every C dependency and miss the local gate.
- **`coverage`** — runs on `ubuntu-latest`. The stable toolchain with `llvm-tools-preview`, then
  `cargo-llvm-cov` at an exact version installed with `--locked`, the way `audit.yml` pins
  `cargo-audit`. That pins the tool, not the instrumentation: `toolchain: stable` floats, so a
  Rust release can still move the numbers. Steps: `cargo llvm-cov clean --workspace`, so no
  earlier run's data is counted; `cargo llvm-cov --workspace --exclude plasmosome-guards
  --no-report`, the one measured run; then reports from it, the tool's summary table to the step
  summary and `lcov.info` as a workflow artifact. No step carries `continue-on-error` or
  `|| true`, narrows the report with `--ignore-filename-regex`, or acts on the percentage, by a
  `--fail-under-*` or `--fail-uncovered-*` flag or by a step of its own.

`macos` and `coverage` run on every pull request and every push to main: the workflow's triggers
carry no `paths` filter and neither job has an `if:`. Each step runs one command, except that a
captured test step is one pipeline under `shell: bash`, which GitHub runs with `-eo pipefail`, so
the pipeline fails when any command in it fails; the implementing task rewords `ci.yml`'s header
to say so. On a push each commit has its own concurrency group, so no run on main is cancelled;
only a pull request cancels its superseded run.

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
one that needs an entitlement or privilege the runner lacks. On 2026-10-09 at `f2c2508` no
workspace test needed any of these, so at that commit every test runs on `macos`.

A test that needs a prerequisite asserts it, so a missing prerequisite fails the test. Where a CI
runner cannot supply it, the test is marked `#[ignore = "needs <prerequisite>"]`. A bare
`#[ignore]` fails clippy in every workspace member, including one with no `[lints]` table and any
member added later, because CI's and the root gate's clippy commands carry
`-D clippy::ignore_without_reason`. A test never
returns early when its prerequisite is missing: it would report `ok` having tested nothing, and
that pass reads the same as a real one. No mechanical check sees an early return, so review
holds that line.

The ignored-test report runs in `gates` and in `macos`. In those two jobs each `cargo test` step
runs with `--no-fail-fast`, so one failing test binary does not hide the rest, and captures its
output with `2>&1 | tee -a` into one file, so each test's line follows the `Running` line that
names its test executable. Two crates can hold test files of one name, as `tests/daemon_cli.rs`
in `plasmosome-core` and `plasmosome-membrane`, so the report maps each executable to its package
with `cargo test --no-run --message-format=json` over the same packages; a doctest's crate is on
its `Doc-tests` line. The report step runs with `if: always()`. It reads the harness's
`test <name> ... ignored, <reason>` lines and writes each ignored test's crate, name and reason
to the step summary; an ignored doctest carries no reason and is listed with `no reason given`.
When every test step succeeded and nothing was ignored, it says so. When a test step did not
succeed or never ran, as after a red clippy step, it names that step and says the list covers
only what ran; it never then says that none was ignored. A test compiled out by `cfg` for the
other OS is not a skip; the other OS's job runs it.

### Coverage is a report, not a gate

The `coverage` job goes red only when a test or the tool fails, never on a percentage. That is a
choice made here: a failing test says the product is broken, and a lower percentage does not say
whether it is. Spec 005 made the same choice for benchmarks, run always and gate later; a
threshold here would likewise be a later decision with its own argument.

The step summary names its platform and the `plasmosome-guards` exclusion beside the tool's
table. On Linux the macOS-only lines are not compiled, so they are absent from the count rather
than counted as uncovered; a local run on a Mac includes them, so the two totals differ. Linux is
chosen because macOS runners are the scarce ones (see Cost), and a job that never fails on a
number should not queue ahead of `macos`. `plasmosome-guards` is excluded: its tests read the
repository rather than exercise product code, and they drive nested cargo builds. The report is
not compared with the base branch. The latest successful run on main is the reference; a
comparison in CI would need another run's artifacts and a wider token.

### Which red blocks a merge

Two layers decide. The
[pr-review merge gate](../../.agents/skills/pr-review/SKILL.md#4-answer-findings-and-verify-the-final-head)
requires CI green for the validated head, so a red or pending job anywhere in `ci.yml`, `macos`
and `coverage` included, blocks a merge by rule. Branch protection makes a block mechanical and
binding on every merger, including merges made outside pr-review; on 2026-10-09 it held only
`gates`. Adding a check to it is an owner admin action. `coverage` is meant to stay out of it,
and no review rule cites the coverage number: a reviewer need not read, answer or quote it.

**Owner question:** once `macos` has been green on ten consecutive pushes to main, add it to
branch protection? That changes no merge outcome under pr-review; it makes the existing block
mechanical for every merger.

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
time, instrumented. On 2026-10-09 at `a0a5411`, one `cargo llvm-cov` 0.6.21 run over the same
crates, on an M3 Max with a cold target, took 93 s and measured 93.09% of lines, macOS lines
included. The fifteen-minute revisit applies to every job.

## Contract

- Job names: `gates`, `crate (<member>)` one per workspace member, `msrv`, `macos`, `coverage`.
- A green `crate (<member>)` run promises: the crate and all its targets build via `-p`, and its
  tests pass, without any sibling named on the command line.
- A green `msrv` run promises: the workspace type-checks on Rust 1.96 exactly.
- A green `macos` run promises: on macOS 26 on Apple silicon, every workspace target passes clippy
  with warnings denied, `cfg(target_os = "macos")` code included; `darwin_group.c` compiles
  without a C warning; and every test that is not ignored builds and passes. It promises nothing
  a booted guest would show, nothing about other macOS versions (spec 001 admits macOS 14 and
  later) and nothing about Intel Macs.
- A test that needs a prerequisite asserts it; one a CI runner cannot supply is
  `#[ignore = "needs <prerequisite>"]`, and a bare `#[ignore]` fails clippy in every member.
  `gates` and `macos` each name every ignored test with its crate and reason, say none was only
  when every test step succeeded, and otherwise say the list covers only what ran.
- `coverage` turns red only on a failing test or tool run, never on a percentage. It adds no
  permission beyond the workflow's `contents: read` and pins `cargo-llvm-cov` to an exact version.
- The guard's name states its clause, e.g. `ci_matrix_matches_workspace_members`.

## Acceptance

Each item names the task that delivers it: plasmosome-005 the matrix, `msrv` and the guard;
plasmosome-6ja `macos` and the ignored-test report; plasmosome-52e `coverage`; "first" means
whichever of 6ja and 52e lands first.

- (005) `ci.yml` has the `crate` matrix job covering every workspace member at merge time, two
  steps per entry, one command per step.
- (005) `ci.yml` has the `msrv` job pinned to 1.96 and it is green.
- (005) The guard fails when a workspace member is missing from the matrix or the matrix names a
  non-member, and the mutation test (entry removed, failure observed, entry restored) is recorded
  in the PR.
- (6ja) `ci.yml` has the `macos` job on `macos-26` with the four steps above and no `if:`, the
  workflow's triggers carry no `paths` filter, and its header states the pipeline exception.
- (first) On a push each commit has its own concurrency group; only pull requests cancel.
- (6ja) On throwaway commits, each run recorded in the PR: an assertion broken in a
  `cfg(target_os = "macos")` test turns `macos` red while `gates` stays green; a clippy warning
  only in `cfg(target_os = "macos")` code fails the `macos` clippy step, and that run's report
  says no test step ran; an unused variable in `darwin_group.c` fails `macos` and the root gate
  on a Mac; and a bare `#[ignore]` in a member whose manifest has no `[lints]` table fails CI's
  clippy and the root gate.
- (6ja) On a green throwaway commit with `#[ignore = "needs a hypervisor"]` tests in
  `tests/daemon_cli.rs` of both `plasmosome-core` and `plasmosome-membrane`, another in
  `plasmosome-guards`, and an ` ```ignore ` doctest, the `gates` and `macos` summaries each name
  all four with their own crates, the doctest with `no reason given`.
- (6ja) On a throwaway commit where a failing test runs in an earlier binary than an ignored one,
  `gates` and `macos` go red at that test step, still name the ignored test, and say the step did
  not succeed. The introducing PR's own runs name exactly the tests ignored in each run, or say
  there are none.
- (6ja) `crates/plasmosome-testkit/AGENTS.md`'s layer table shows the reasoned
  `#[ignore = "..."]` form, never a bare `#[ignore]`.
- (6ja) The task stays open after its PR merges until `macos` has been green on ten consecutive
  pushes to main; the owner question above is put to the owner then, and the notes record it.
- (52e) `coverage` has exactly the steps above, with `--exclude plasmosome-guards` and nothing
  narrower; uploads `lcov.info`; adds no `permissions` entry; installs `cargo-llvm-cov` at an
  exact version with `--locked`; and carries none of the forbidden settings, flags or steps.
- (52e) On throwaway commits, each linked from the PR: deleting the only test that reaches some
  lines lowers that file's reported coverage while `coverage` stays green, and a failing test
  turns `coverage` red.
- (each) Every new job is green on the PR that introduces it, and the gate in the root
  `AGENTS.md` is green.
