---
id: 004
title: CI proves each crate alone, the workspace together on Linux and macOS, and the MSRV
status: accepted
intents: [002]
---

## Behavior

CI answers four questions the current single job cannot: does each crate still compile and pass
its tests on its own, does the workspace still hold together, does it still build and pass its
tests on macOS, and is the declared minimum Rust version (1.96) still true. A green run means all
four, and the list of crates CI checks cannot drift from the workspace without a test failing.
CI also reports how much of the code the tests execute; no coverage number ever fails a run.

Today `ci.yml` runs one `gates` job: fmt, clippy, workspace tests, workspace guards, provenance
guard. That proves the workspace together. It does not prove a crate stands on its own — a crate
can lean on a sibling's dev-dependencies or on workspace build order and stay green for months.

The macOS and coverage jobs come from the 2026-10-08 audit of what intent 002 has delivered.
Every job ran on `ubuntu-latest`, so code compiled only for macOS — `darwin_group.c`, which the
membrane's build script compiles only for a macOS target, and the membrane's
`cfg(target_os = "macos")` paths — was never built, linted or tested by CI, although macOS on
Apple silicon is the product's first target. Nothing measured coverage either, though the owner's
standing instruction is to aim for high unit coverage and to analyse it after each feature.

Publishing is deliberately not here. The strongest per-crate check, `cargo package`, needs
version fields on path dependencies and a decision to publish at all; that is spec 007 and it is
blocked on owner decisions. This spec ships the strongest check available short of that.

## Design

### Jobs

- **`gates`** — unchanged. It remains the integration gate; when spec 003's testkit lands, its
  cross-crate tests run here automatically under `cargo test --workspace`.
- **`crate`** — a matrix job, one entry per workspace member, hardcoded in `ci.yml`. Two steps
  per entry, one command per step (the existing `ci.yml` rule: a multi-command step can pass
  while a command inside it failed):
  1. `cargo build -p <crate> --all-targets`
  2. `cargo test -p <crate>`
- **`msrv`** — `dtolnay/rust-toolchain@1.96` (the pinned toolchain action, not `@stable`), then
  one step: `cargo check --workspace --all-targets`.
- **`macos`** — runs on `macos-26`: Apple silicon (M1, three cores) on the macOS version the
  project is developed on. The label is pinned rather than `macos-latest`, so the OS under test
  changes only by a commit here. The same toolchain and cache actions as `gates`, then one
  command per step:
  1. `cargo clippy --workspace --all-targets -- -D warnings`
  2. `cargo test --workspace`
  3. the ignored-test report described below

  There is no separate build step: both commands compile what they need, and a compile failure
  fails them. fmt, the provenance and attribution checks and the Python tool suites stay in
  `gates` only, as do the crate matrix and `msrv`: this job exists for the Rust code a second OS
  compiles differently.
- **`coverage`** — runs on `ubuntu-latest` and is never required. The stable toolchain with
  `llvm-tools-preview`, then `cargo-llvm-cov` at an exact version installed with `--locked`, the
  way `audit.yml` pins `cargo-audit`, so an upstream release cannot change the report without a
  commit here. It measures one run, `cargo llvm-cov --workspace --exclude plasmosome-guards
  --no-report`, and reports it twice: a summary in the step summary, and `lcov.info` as a
  workflow artifact for anyone who wants the uncovered lines.

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
one that needs an entitlement or privilege the runner lacks. No workspace test needs any of
these at `a0a5411`, so today `macos` runs every test.

When such a test arrives it is marked `#[ignore = "needs <prerequisite>"]`, the attribute spec
003 already gives the end-to-end layer. It never checks for the prerequisite at run time and
returns early: that test reports `ok` having tested nothing, and its pass reads the same as a
real one. The test harness counts an ignored test apart from a passed one, and the report step
names every ignored test with its reason, or says that none was ignored. A test compiled out by
`cfg` for the other OS is not a skip; the other OS's job runs it.

### Coverage is a report, not a gate

The `coverage` job fails when the tests or the tool fail, never on a number. Spec 013 sets the
bar for a check that refuses work: a consequence the next commit cannot undo. Lower coverage is
undone by the next commit that adds the test, so a threshold does not clear that bar. Unlike
spec 005's bench job, this is not a gate waiting for variance data; it is a report by design.
It serves the owner's instruction by making the number visible on every run.

The summary leads with the total line coverage, then the tool's per-file table, and names its
platform. On Linux the `cfg(target_os = "macos")` lines are not compiled, so they are absent from
the count rather than counted as uncovered; a local run on a Mac includes them, so the two totals
differ. Linux is chosen because macOS runners are the scarce ones (see Cost), and an advisory job
should not queue ahead of a required one. `plasmosome-guards` is excluded: its tests read the
repository rather than exercise product code, and they drive nested cargo builds. The report is
not compared with the base branch. The run on each push to main is the reference; a comparison
in CI would need another run's artifacts and a wider token, for a number nobody must act on.

### Required checks and review

Branch protection on main requires only `gates`. `macos` is meant to be one more required check,
because a red `macos` means the product is broken where it ships first. Adding a required check
is an owner admin action. Until it is done, the pr-review merge gate (CI green for the validated
head) already treats a red `macos` as blocking; branch protection makes that mechanical.

**Owner question:** add `macos` to main's required status checks once it has been green on the
PR that introduces it and on main?

`coverage` is never required, and no review rule cites it: a reviewer need not read, answer or
quote the number.

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
public repository. Their cost is time. The macOS runner has three cores to the Linux runner's
four, and the account runs at most five macOS jobs at once, so with many PRs open `macos` waits
for a runner. For scale, `gates` took 61 s with a warm cache on main at `a0a5411`; the first
`macos` and `coverage` timings go in the notes of the tasks that add them. `coverage` builds the
workspace a second time, instrumented, plus the pinned tool on a cache miss, and runs every test
outside `plasmosome-guards`: on an M3 Max with a cold target at `a0a5411` that took 93 s and
measured 93.09% of lines, macOS lines included. The fifteen-minute revisit applies to every job.

## Contract

- Job names: `gates` (unchanged), `crate (<member>)` one per workspace member, `msrv`, `macos`,
  `coverage`.
- A green `crate (<member>)` run promises: the crate and all its targets build via `-p`, and its
  tests pass, without any sibling named on the command line.
- A green `msrv` run promises: the workspace type-checks on Rust 1.96 exactly.
- A green `macos` run promises: on macOS 26 on Apple silicon, every workspace target passes clippy
  with warnings denied, `cfg(target_os = "macos")` code included, and every test that is not
  ignored builds and passes. It promises nothing a booted guest would show, nothing about older
  macOS versions (spec 001 admits macOS 14 and later) and nothing about Intel Macs.
- A test that needs what a hosted runner lacks is `#[ignore = "needs <prerequisite>"]`, never an
  early return. The `macos` step summary names each ignored test and its reason, or says none was.
- `coverage` never fails on a coverage number, adds no permission beyond the workflow's
  `contents: read`, and pins `cargo-llvm-cov` to an exact version. Its summary leads with the
  total line coverage and names its platform.
- The guard's name states its clause, e.g. `ci_matrix_matches_workspace_members`.

## Acceptance

- `ci.yml` has the `crate` matrix job covering every workspace member at merge time, two steps
  per entry, one command per step.
- `ci.yml` has the `msrv` job pinned to 1.96 and it is green.
- The guard fails when a workspace member is missing from the matrix or the matrix names a
  non-member, and the mutation test (entry removed, failure observed, entry restored) is recorded
  in the PR.
- `ci.yml` has the `macos` job on `macos-26` with the three steps above, one command per step.
- On a throwaway commit, an assertion broken in a `cfg(target_os = "macos")` test turns `macos`
  red while `gates` stays green, and a clippy warning placed only in `cfg(target_os = "macos")`
  code fails the `macos` clippy step. Both runs are linked from the PR.
- On a throwaway commit, a test marked `#[ignore = "needs a hypervisor"]` appears by name with that
  reason in the `macos` step summary, and the introducing PR's own run names exactly the tests
  ignored in its tree, or says there are none.
- `coverage` measures with `--workspace --exclude plasmosome-guards` and nothing narrower, adds no
  `permissions` entry, installs `cargo-llvm-cov` at an exact version with `--locked`, and uploads
  `lcov.info`; its summary leads with the total line coverage and names its platform. On a
  throwaway commit, deleting the only test that reaches some lines lowers that file's reported
  coverage while `coverage` stays green; that run is linked from the PR.
- The PR that introduces `macos` puts the required-check question above to the owner.
- All new jobs are green on the PR that introduces them.
- The gate in the root `AGENTS.md` is green.
