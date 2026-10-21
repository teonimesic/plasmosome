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

CI also runs rustdoc over the workspace with warnings denied, in `gates` and `macos`, so a `///`
link to an item that was renamed, made private or never existed fails the run. Nothing else in CI
reads a doc link: `cargo test` compiles doctests with `rustdoc --test`, which does not resolve
links, and clippy runs no rustdoc lint. Until this step existed, executors ran the command by
hand, and #244, which skipped it, merged a link rustdoc could not resolve.

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
  feed it, the `ignore_without_reason` flag on its clippy step, and its rustdoc step (all
  below); its other steps belong to the specs that add them.
- **`crate`** — a matrix job, one entry per workspace member, hardcoded in `ci.yml`. Two steps
  per entry, one command per step (the existing `ci.yml` rule: a multi-command step can pass
  while a command inside it failed):
  1. `cargo build -p <crate> --all-targets`
  2. `cargo test -p <crate>`
- **`msrv`**: installs Rust 1.96.0, then runs `cargo +1.96.0 check --locked --workspace
  --all-targets`. The explicit `+1.96.0` is required. A `rust-toolchain.toml`
  ([spec 026](026-security-standard.md#3-the-toolchain)) outranks the default toolchain an action
  sets, so a bare `cargo check` would run the pinned compiler while the action's log reports 1.96.
  Clippy's `incompatible_msrv` in `gates` already refuses newer standard-library APIs. This job
  catches newer language features in any target, and dependencies whose own `rust-version` is
  above 1.96.

  Amended on 2026-10-20, as [spec 026](026-security-standard.md#spec-004-the-msrv-job) requires.
  Until then this bullet named `dtolnay/rust-toolchain@1.96` and a bare `cargo check`. With the
  pin in place that pair checks with the pinned release, while the action's log says 1.96,
  wherever the pin is installed or auto-install is on. Under the `RUSTUP_AUTO_INSTALL: "0"` that
  `ci.yml` sets, it fails at the bare `cargo check`, because the pin is not installed. Either way
  a plan that follows the old text does not meet this one. The job installs 1.96.0 by name before
  any cargo command, for example with
  `rustup toolchain install 1.96.0 --profile minimal --no-self-update`.
- **`macos`** — runs on `macos-26`, the standard arm64 runner. The label pins the major macOS
  version, not the image: GitHub updates the image about weekly, so the minor version and the
  clang that compiles `darwin_group.c` can change without a commit here. The toolchain and cache
  steps are those of `gates`. Corrected on 2026-10-20: this said actions, but since spec 026's pin
  `gates` installs the toolchain with rustup commands, not an action. Steps:
  1. `cargo clippy --workspace --all-targets -- -D warnings -D clippy::ignore_without_reason`
  2. `cargo test --workspace --exclude plasmosome-guards`
  3. `cargo test -p plasmosome-guards`
  4. the ignored-test report
  5. `cargo doc --locked --workspace --no-deps`, with `RUSTDOCFLAGS` set to `-D warnings` on this
     step only, as [the rustdoc step](#the-rustdoc-step) says

  Steps 2 and 3 are the split `gates` uses, so each guard test runs once per job; the guards run
  here too because their hook-script tests meet BSD userland on macOS. There is no separate build
  step: these commands compile what they need, and a compile failure fails them. fmt, the
  provenance and attribution checks and the Python tool suites run in `gates` only; the crate
  matrix and `msrv` run on Linux only.

  A C warning in `darwin_group.c` fails the build, in `macos` and in the root gate on a Mac;
  clippy alone passes it as a `cargo:warning` line. The preferred way is `cc`'s
  `warnings_into_errors(true)` in the membrane's `build.rs`, which reaches that file alone, where
  a job-wide `CFLAGS` would reach every C dependency and miss the local gate.
- **`coverage`** — runs on `ubuntu-latest`. The toolchain `rust-toolchain.toml` pins, installed
  and printed as [spec 026](026-security-standard.md#3-the-toolchain) requires, with
  `llvm-tools-preview` added by `rustup component add`; then `cargo-llvm-cov` at an exact version
  installed with `--locked`, the way `audit.yml` pins `cargo-audit`. That pins the instrumentation
  as well as the tool, so a Rust release moves the numbers only through a commit that bumps the
  pin. Steps: `cargo llvm-cov clean --workspace`, so no
  earlier run's data is counted; `cargo llvm-cov --workspace --exclude plasmosome-guards
  --no-report`, the one measured run; then reports from it, the tool's summary table to the step
  summary and `lcov.info` as a workflow artifact. No step carries `continue-on-error` or
  `|| true`, narrows the report with `--ignore-filename-regex`, or acts on the percentage, by a
  `--fail-under-*` or `--fail-uncovered-*` flag or by a step of its own. Corrected on 2026-10-20:
  this bullet said the job used the stable toolchain and that `toolchain: stable` floats, which
  spec 026's pin made untrue.

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
its `Doc-tests` line. The report step runs with `if: ${{ !cancelled() }}`, so it runs after a
failed step but not in a cancelled run. It reads the harness's
`test <name> ... ignored, <reason>` lines and writes each ignored test's crate, name and reason
to the step summary; an ignored doctest carries no reason and is listed with `no reason given`.
When every test step succeeded and nothing was ignored, it says so. When a test step did not
succeed or never ran, as after a red clippy step, it names that step and says the list covers
only what ran; it never then says that none was ignored. A test compiled out by `cfg` for the
other OS is not a skip; the other OS's job runs it.

A cancelled run has no report. On a push each commit has its own concurrency group, so a run on
main is cancelled only by hand. A pull request's run is cancelled when a newer push to that pull
request starts another, and review reads the newer run. `if: always()`, which this step had until
the amendment below, differs from `!cancelled()` only in a cancelled run. There the report still
ran `cargo test --no-run` for each test step that had started, so a run cancelled while a test
step compiled finished that build for a summary of a head nobody merges, on a runner the newer
run may be waiting for. GitHub also cancels a job that reaches its `timeout-minutes`. The report
is not promised there either, and the test step's own log shows where it stopped. `ci.yml`'s
attribution read ([spec 029](029-the-contribution-record.md)) and `audit.yml`'s issue job use the
same condition, which GitHub's documentation recommends for a step that runs whatever the earlier
steps' results were.

Amended on 2026-10-21: the report step ran under `if: always()`. Run 39579409827, cancelled on
2026-10-21 by a newer push, ran both report steps and skipped the attribution read.

### The rustdoc step

`gates` and `macos` each run `cargo doc --locked --workspace --no-deps` as one step, directly after
the ignored-test report, with `RUSTDOCFLAGS` set to `-D warnings` on that step alone. It is step 5
of the `macos` list above, the fifth of the shared steps that start at clippy. `--locked` is there
because [spec 026](026-security-standard.md#3-the-toolchain) requires it of every CI cargo command
that resolves dependencies. `--no-deps` documents the workspace members only; a dependency's doc
warnings are not this repository's to fix.

- **Both jobs.** rustdoc documents only what is compiled for the OS it runs on. Items under
  `cfg(target_os = "linux")` are documented only in `gates`, and items under
  `cfg(target_os = "macos")` only in `macos`, as with clippy.
- **On the step, not the job.** Cargo passes `RUSTDOCFLAGS` to every rustdoc call it makes,
  including the doctests `cargo test` compiles. A job-wide value would change how the test steps
  build doctests, and they would no longer run what the root gate runs.
- **Public items only.** The step does not pass `--document-private-items`. It checks the docs a
  caller reads, which is what the root `AGENTS.md` asks `///` to serve. It is also the command
  executors already ran by hand, so a green root gate predicts a green step on the same OS. A broken
  link in a private item's doc may go unreported.
- **What it cannot see.** `-D warnings` turns a warning into an error, and a lint a package allows
  does not warn, so the step passes it. A rustdoc lint can be allowed in a manifest, through
  `[lints.rustdoc]` or the root's `[workspace.lints.rustdoc]`, or in source, through an attribute
  such as `#![allow(rustdoc::broken_intra_doc_links)]` or `allow(warnings)`. Each is a visible line
  in a diff. [Spec 026 §8](026-security-standard.md#8-which-prs-this-spec-governs) already puts
  every manifest `[lints]` change before its reviewer, and a reviewer reads a new attribute as code.
  A target whose manifest sets `doc = false` is not documented at all, and an item marked
  `#[doc(hidden)]` drops out of the docs, so the step reads neither one's links. Spec 026 §8 does
  not list a manifest's target tables, so a reviewer reads a `doc = false` line as code too.
- **Not a guard.** It is a lint run on the footing of the clippy step: it reports rustdoc's lints
  at the levels rustdoc and the packages set, with warnings denied.
  [Spec 013](013-what-earns-a-guard.md) counts a level a package declares as that package's code,
  not a guard, and a broken doc link is undone by the next commit.

The root gate in `AGENTS.md` runs the same check as
`RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`, without `--locked`, like its other
cargo commands.

Amended on 2026-10-21: this section is new, with step 5 of `macos`, the rustdoc sentences under
Behavior, `gates`, Cost, Contract and Acceptance, and the root gate's command. Until then no CI
step ran rustdoc.

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

The rustdoc step is cheap. On 2026-10-21 at `3c5db3b`, on an Apple-silicon Mac with a cold,
separate target, `cargo doc --locked --workspace --no-deps` took 5.6 s by cargo's own count and
wrote 81 MB. At `2c55cd7` the same day, on the same Mac while other builds ran on it, the same
command with `-D warnings` took 14.1 s by the same count and wrote 82 MB. Its first CI timings go
in plasmosome-36oc's notes.

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
- A green `gates` or `macos` run promises: rustdoc documented every workspace member on that OS with
  no warning, at the lint levels the packages declare. It promises nothing about the docs of private
  or `#[doc(hidden)]` items, of a target whose manifest sets `doc = false`, or of items compiled
  only for the other OS.
- A test that needs a prerequisite asserts it; one a CI runner cannot supply is
  `#[ignore = "needs <prerequisite>"]`, and a bare `#[ignore]` fails clippy in every member.
  In every run that is not cancelled, `gates` and `macos` each name every ignored test with its
  crate and reason, say none was only when every test step succeeded, and otherwise say the list
  covers only what ran.
- `coverage` turns red only on a failing test or tool run, never on a percentage. It adds no
  permission beyond the workflow's `contents: read` and pins `cargo-llvm-cov` to an exact version.
- The guard's name states its clause, e.g. `ci_matrix_matches_workspace_members`.

## Acceptance

Each item names the task that delivers it: plasmosome-005 the matrix, `msrv` and the guard;
plasmosome-6ja `macos` and the ignored-test report; plasmosome-52e `coverage`; plasmosome-36oc
the rustdoc step and the root gate's command; plasmosome-ofaf the report step's condition;
"first" means whichever of 6ja and 52e lands first.

- (005) `ci.yml` has the `crate` matrix job covering every workspace member at merge time, two
  steps per entry, one command per step.
- (005) `ci.yml` has the `msrv` job pinned to 1.96 and it is green.
- (005) Each of these makes the `msrv` job fail while `gates` passes, shown by mutation in the PR
  that adds the job: locking a dependency version whose `rust-version` is above 1.96, and a
  language feature newer than 1.96 in an integration test.
- (005) The guard fails when a workspace member is missing from the matrix or the matrix names a
  non-member, and the mutation test (entry removed, failure observed, entry restored) is recorded
  in the PR.
- (6ja) `ci.yml` has the `macos` job on `macos-26` with steps 1 to 4 above and no `if:`, the
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
- (36oc) `gates` and `macos` each run the rustdoc step as described, and no job, workflow or
  other step sets `RUSTDOCFLAGS`.
- (36oc) On one throwaway commit, linked from the PR, one crate's public docs hold three links
  that do not resolve: one compiled on both OSes, one only under `cfg(target_os = "linux")` and
  one only under `cfg(target_os = "macos")`. The rustdoc step is red in both jobs while their
  clippy and test steps are green. The `gates` log names the first two links and not the third;
  the `macos` log names the first and third and not the second. On a Mac the root gate's command
  fails on that commit too.
- (36oc) The root gate in `AGENTS.md` carries the command named above.
- (ofaf) Both report steps run under `if: ${{ !cancelled() }}`, and no workflow uses `always()`.
  In one pull request run that a newer push cancels while a test step runs, both report steps
  are skipped and run no cargo command. In one run where a test fails, both run and name the
  failed step.
- (each) Every new job is green on the PR that introduces it, and the gate in the root
  `AGENTS.md` is green.
