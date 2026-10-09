---
id: 026
title: The security standard, and what refuses work in its name
status: draft
intents: [006, 002]
---

## Behavior

Intent 006 asks Plasmosome to hold high security standards in its isolation and in everything it
depends on: the kernels it runs on, its own code, and what it pulls in. Until now nothing said what
those standards were, so nothing could refuse a change in their name. This spec is the standard. It
has six rules. They cover what may become a dependency, where `unsafe` code may live, which
compiler builds the tree, which advisories and sources are accepted, how bytes from a less-trusted
side are parsed, and how a vulnerability is reported. For each rule it says what refuses a change
that breaks it: a CI check, the compiler, or the independent reviewer.

The rule with the most reach is dependency admission. Every third-party package in a git-tracked
`Cargo.lock` is named by a short review record in `docs/dependencies/`. The first step of `gates`
reads the tracked manifests and Cargo configuration with a TOML parser and never runs cargo. It
refuses the ways of swapping or wrapping the code cargo builds: `[patch]`, `[replace]`, source
replacement, `include`, build settings in committed Cargo configuration, and path dependencies
outside the repository. Only then does a guard read `cargo metadata` and refuse a package no record
names. A new dependency is admitted only when what the workspace uses of it could not be written
here in roughly a thousand lines. That rule is the owner's, from intent 006's first draft.

The other rules are shorter. Every cargo command in CI that resolves dependencies runs with
`--locked`, so the lockfile is what CI builds. Packages with no `unsafe` code forbid it. In those
that need it, each block carries a safety argument that clippy checks. An exact
`rust-toolchain.toml` pins the compiler. `cargo-deny` refuses non-crates.io sources on every PR,
and known vulnerabilities on every PR that changes a manifest or lockfile. Advisories against code
already on main open one `security` issue. Every parser fed by a less-trusted side checks a size
bound while reading, and the three that face hostile writers get fuzz targets. `SECURITY.md` routes
reports through GitHub's private vulnerability reporting.

[Spec 013](013-what-earns-a-guard.md) still decides which refusals may fail the build: only those
whose harm the next commit cannot undo. [Section 7](#7-how-each-rule-refuses) argues each one. A
rule that fails the bar belongs to the reviewer.

**Platform scope.** The standard is macOS-first: it is written for Darwin hosts on arm64, where
cells are being built today. Linux hosts are deferred, not dropped, and every rule applies to them
unchanged. CI runs on Linux runners, so code under `cfg(target_os = "macos")` or
`cfg(not(target_os = "linux"))`, and C built only on macOS, is compiled and linted only by the root
gate on the author's Mac until a macOS CI job exists.

**What exists today**, on main at e1cf004 on 2026-10-08:

- `cargo audit` runs weekly and on PRs that change `Cargo.lock`, in a job that is not required.
  Branch protection requires only `gates`.
- There is no `cargo-deny`, toolchain file, `SECURITY.md`, fuzzing, `forbid(unsafe_code)` or
  committed Cargo configuration. CI floats `stable` and never passes `--locked`. No `unsafe` block
  has a safety comment.
- Actions are pinned by SHA, and Dependabot updates them but not Cargo. Private vulnerability
  reporting, Dependabot alerts and secret scanning are on. Push protection is off.
- `gates` denies clippy's `incompatible_msrv`, which refuses standard-library APIs newer than
  `rust-version` 1.96. Nothing checks newer language features or dependencies' own `rust-version`.
- The lockfile holds 104 third-party packages, reached through ten direct dependencies.

## Threat model

Trust runs from the owner, and the operator files the owner writes, to the host kernel and the
binaries built from this tree. Everything else is a less-trusted side. The standard defends against
four parties now.

1. **A hostile workload in a cell.** This is code in the guest actively trying to reach beyond its
   grants: the guest kernel, the host bridges, other plasmids' files. The cell boundary is spec 001
   §4.2 and spec 017. This standard adds that the host treats every byte from the guest as hostile,
   and that the guest's syscall surface is part of the boundary.
2. **A poisoned agent following instructions faithfully.** The phrase is spec 011's. A poisoned
   agent in a cell reaches the host only through the control socket and the capabilities granted
   to its cell. Its requests are well formed and look authorized. The controller's parsers must
   survive whatever it sends, because a controller crash stops supervision of every cell.
3. **Hostile plasmid bytes.** A plasmid's manifest and its broker's readiness replies are written by
   its author, who may be anyone (intent 010).
4. **Supply-chain compromise.** This is a malicious or hijacked crate or release, a swapped source,
   an action, a compiler or a CI tool. It is defended by records, `--locked`, crates.io as the only
   source, a seven-day wait for new releases, and pinned actions and tools.

**Guest-side syscall filtering is in scope.** Whatever denies the workload a kernel object inside
the guest, such as AF_VSOCK sockets, user namespaces or io_uring, is part of the boundary. Reviews
of draft spec 024 found two routes a single-syscall filter misses on the 6.12 guest kernel.
io_uring's `IORING_OP_SOCKET` opens a socket without calling `socket(2)`. A network namespace does
not confine AF_VSOCK, which reaches the host bridges on ports 4090 and 4091. Spec 017 already
requires that "all alternate syscall/loader paths" meet the same denial. Spec 024 owns the
mechanism, shaped by owner item O-7. This spec names the surface and does not choose.

**Not covered yet.** These are stated so nobody reads silence as a claim.

- **Mandatory access control inside the guest.** The qualified guest kernel has `CONFIG_SECURITY`
  unset (O-7). Until O-7 is settled, file and exec enforcement inside the guest is not claimed. A
  process in a cell may read whatever the guest's ordinary permissions allow, including files staged
  for another plasmid in the same cell. Network egress is designed to be enforced on the host
  (spec 001 §4.2), and that design is in scope.
- **Credential custody for a foreign harness.** A harness that makes its own TLS connections needs
  the real credential in the guest, or an intercepting proxy on the host. Neither is designed. A
  credential delivered into a cell is assumed readable, and detach does not revoke what was read.
- **An orphan after `SIGKILL` of `membraned`** (O-10). The helper and VM keep running.
- **Native artifacts and the host.** libkrun, the guest kernel, Hypervisor.framework and the macOS
  kernel are trusted. libkrun and the guest kernel are pinned by commit and digest (spec 001 §4.2),
  but they are pinned, not tracked. `cargo-deny` reads only this tree's lockfiles, not libkrun's,
  and no feed reports the guest kernel's CVEs here. Section 1's native records say how often each
  is checked. Who checks is Q6.
- **Same-user processes on the host.** Plasmosome does not confine an agent running on the host as
  the owner's user, and a same-UID process defeats every host-side check (spec 001). A broker is
  such a process: it runs unconfined, behind spec 001's and spec 017's peer-UID checks. So the
  standard defends against a broker's bytes, not its code. The readiness bound still keeps a faulty
  broker from taking the membrane down.

## Contract

### 1. Dependency admission

**What counts.** A dependency is a package with a registry or git source in a git-tracked
`Cargo.lock`. Today that means the root lockfile. Later it also means the fuzz workspace's.

- Path packages inside this repository are first-party code. They are reviewed as code in the diff
  and need no record.
- Copying a third-party crate into the tree makes it first-party. It is still held to the
  thousand-line rule, and `cargo-deny` can no longer see its advisories, so the PR that copies it
  says how they will be watched.
- A path dependency outside the repository is refused.
- Build and dev dependencies count fully. Their build scripts, proc macros and tests run on every
  machine that builds or tests the tree.

**When a dependency may be added.** All of these must hold:

1. It comes from crates.io.
2. No unignored vulnerability or unsound advisory affects it (section 4).
3. What the workspace uses of it could not be written here in roughly a thousand lines, or the
   owner approved an exception.
4. It is the narrowest crate, with the fewest features, that does the job. Default features are off
   unless the PR says why they are on. For example, a reader that needs only serde's
   deserialization traits depends on `serde_core`, not `serde` with `derive`.
5. Every version the lockfile diff adds has been public for seven days, unless it fixes an
   advisory. Dependabot's cooldown enforces this on its own PRs. On any other PR it is a reviewer
   item.
6. A record exists, and the independent reviewer checked it.

**The thousand-line rule.** What matters is what writing the used part here would take, not the
size of the crate. A 40,000-line crate whose two functions we call could be 200 lines of our own.
Under roughly a thousand lines, write it here. An exception needs an argument in the record and an
approval. Until Q2 is answered, only the owner approves one. The record links where the approval
was given. As with intents, an agent may record an approval it is carrying, and never originate
one.

**The record.** Each direct dependency has one file, `docs/dependencies/<crate>.toml`. Every field
is required.

| Field | What it holds |
| --- | --- |
| `name`, `range` | The crate, and the semver-compatible range reviewed, such as `0.8` or `1` |
| `reason` | What it does for us, in a sentence or two |
| `write_it_instead` | Estimated lines to write what we use, and how the estimate was reached |
| `exception` | Empty, or the argument and a link to the owner's approval |
| `brings` | Every package in its resolved closure, as `name@range`, normal and build, all platforms |
| `build_time_code` | Build scripts and proc macros in it and in `brings` that build for a supported host |
| `expands_unsafe` | Macros in it or in `brings` that expand `unsafe` into our code, or `none` |
| `native_code` | Native code it compiles or links, or `none` |
| `verdict` | `admitted`, `kept-pending-owner` or `replace` |

A range is the semver-compatible part of a version: `0.8` for 0.8.23, and `1` for 1.0.229. A
package that moves to a new range is new code, so the record that names it must change. Because
`brings` is a closure, a package shared by two dependencies appears in both records, and two people
derive the same list. Features and users are read from `cargo tree`, not kept in the record.

**What the review checks.** The PR's independent reviewer does all of these:

- Re-derives the record entries that changed.
- Reads every build script and proc macro the change brings that builds for a supported host, and
  every macro that expands `unsafe` into our code. A new package is read in full. A new version of
  a package already read is read as the diff between the two locked versions. The supported hosts
  are `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`, where CI runs.
- Puts in the review comment the measurements the record does not keep: lines of Rust, `unsafe`
  counts and crates.io owners.
- Compares `cargo tree -e features` before and after, and asks whether a narrower crate or fewer
  features would serve.

An advisory pass is not a review. It shows that nobody has reported a problem, not that anybody
read the code.

**The git-only step.** It is the first step of `gates`. It lists tracked files with `git ls-files`
and parses every `.cargo/config*` and every `Cargo.toml` with a TOML parser, such as Python's
standard `tomllib`. It never invokes cargo or rustc, because any cargo command, `cargo metadata`
included, runs a configured `rustc` or wrapper. It refuses:

- `[patch]` or `[replace]` in a manifest;
- in a tracked `.cargo/config` or `.cargo/config.toml`: `include`, source replacement, `paths`,
  `patch`, `env`, or any `rustflags`, `rustc`, wrapper, `linker` or `runner` key in any table;
- a path dependency whose canonical path is outside the repository, so a tracked symlink that leads
  outside counts as outside.

None of these has an exception today. If one is ever needed, this spec gains the record field that
names it. Its message names the file and key, and the harm: a program nobody reviewed runs at the
first cargo command on every machine that builds main, including ones that hold publish
credentials. The script lives in `.githooks/` beside `provenance-guard`, and the guards crate
mutation-tests it.

**The record guard.** `every_dependency_has_a_review_record`, in `crates/plasmosome-guards`, runs
second, before clippy or the tests compile anything. It reads `cargo metadata --locked`, which runs
no build script or proc macro. It refuses a locked dependency whose `name@range` is neither a
record's own nor in some record's `brings`. Its message names the package and the same harm, from
build scripts and proc macros. It checks that records exist and name what is locked; whether their
contents are right is the reviewer's to judge. Compiling it builds `plasmosome-guards`' own
dependencies first, so a PR that changes those relies on the rule below.

**Read before running.** A reviewer, person or agent, runs no cargo command on a contributor's PR on
their own machine until the git-only step has passed for that head. The reviewer reads that result
from a CI run only if the PR leaves the workflows and the step's script unchanged; otherwise the
reviewer runs main's copy of the script, which needs no cargo. Before running anything, the reviewer
also reads any change to `build.rs` or `rust-toolchain*`, and the build-time code of every package
the lockfile adds or changes. A contributor's own tests and proc macros run too, so the reviewer
reads the diff before running it.

**Dependencies on main.** The record guard's task writes a re-derived record for every direct
dependency on main: today `serde`, `serde_json`, `uuid`, `toml`, `toml_edit`, `libc`, `cc`,
`proptest`, `tempfile` and `criterion`, plus `sha2` and `serde_core` if PR #123 merges first. Each
carries verdict `kept-pending-owner` with its estimate filled in. Whether to replace any is Q4.
Records may land first, a few per docs-only PR, and the guard lands once they all exist. Until then,
a PR that adds a dependency writes the record's facts into the PR. A PR that removes the last direct
use of a crate deletes its record, so the record stops vouching for its `brings`.

**Native artifacts.** Each native artifact the runtime pins (spec 001 §4.2), such as libkrun and
the guest kernel, has a record in `docs/dependencies/native/<name>.toml`. It states what the
artifact is for, its version, its digest or commit, its source, and how often it is checked for
updates and advisories. A PR that changes a pin updates the record. Who does the checking is Q6.

**CI tools and actions.** Every tool CI installs is pinned to a version and installed with
`--locked`. Every action is pinned to a commit SHA. Workflows keep least-privilege `permissions:`,
check out with `persist-credentials: false`, and never use `pull_request_target`. These are
reviewer items. Q7 asks the owner whether to enforce the action pin as a repository setting.

### 2. `unsafe` code

**Every package forbids it, except the register.** The root manifest's `[workspace.lints]` sets
`unsafe_code = "forbid"`. It also sets two clippy lints to `deny`: `undocumented_unsafe_blocks` and
`missing_safety_doc`. Every package outside the register inherits all three with
`lints.workspace = true`. That covers every target a package compiles: library, binaries, tests,
benches, examples and build script. A `#![forbid(unsafe_code)]` in `lib.rs` alone does not meet
this rule, because it leaves tests, benches and binaries open.

**The register** is the set of packages whose own `[lints]` table sets `unsafe_code = "deny"`.
Today it holds `plasmosome-membrane` and `plasmosome-core`. Cargo does not let a package mix
`lints.workspace = true` with local keys, so a register package repeats both clippy safety lints at
`deny` in its own table.

Inside a register package, `#[allow(unsafe_code)]` goes on each item that needs `unsafe`. It goes on
a whole module only when the crate's `AGENTS.md` names that module as an FFI module. The crate root
never carries it. The register uses `deny`, not `forbid`, because an inner `allow` cannot lift
`forbid`. A package joins the register in its own PR, which keeps both clippy lints and says why no
safe route exists. Leaving the register is welcome.

**The safety argument.** Every `unsafe` block and `unsafe impl` carries a `// SAFETY:` comment,
directly above it or above the statement that holds it, saying which invariant makes it sound there
and what establishes it. Every `unsafe fn`, private ones included, carries a `# Safety` doc section.
A root `clippy.toml` sets `check-private-items = true`, because `missing_safety_doc` otherwise
checks only public functions.

An `unsafe extern` block documents what each declaration assumes about the foreign side. No lint
reads that, so it is a reviewer item. The safety comment is the one inline `//` comment the code
carries. That needs the `AGENTS.md` amendment below.

**What the lint levels do not reach.**

- **`unsafe` expanded from another crate's macro.** rustc does not report lints raised inside
  external macros. So `forbid` lets a dependency's `macro_rules!` or derive put `unsafe` into a
  forbid package, and no safety comment is asked for. The record's `expands_unsafe` field covers
  the range as reviewed; a compatible bump can add such a macro unseen. A PR whose code in a forbid
  package first calls a macro some record lists there says why.
- **Build settings that weaken lints.** `--cap-lints` or `-A` in rustflags silences both the forbid
  and clippy. The git-only step refuses that in committed Cargo configuration. A workflow can reach
  the same settings through `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, any `CARGO_BUILD_*` or
  `CARGO_TARGET_*` variable, `RUSTC_WRAPPER`, `CARGO_HOME` or `--config`. That is a reviewer item,
  because the step does not parse YAML.

**Native code is `unsafe` code.** Only register packages compile or link it. In a forbid package
the compiler refuses the `unsafe extern` block that calling it needs. A test can also compile and
run C as a separate program, as the membrane does with `supervision_worker.c`. That needs no
`extern` block, so it is a reviewer item: only register packages may do it.

The fuzz workspace is the one exception. It is never shipped, and it compiles libFuzzer's C++
through `libfuzzer-sys`. The root `[workspace.lints]` does not reach a separate workspace, so the
fuzz manifest declares `unsafe_code = "forbid"` for its own code.

### 3. The toolchain

- **The pin.** `rust-toolchain.toml` names one exact stable release, with clippy and rustfmt. At
  the time of writing that is `channel = "1.99.0"`. It is never `stable` and never `1.99`.
- **CI installs what the file names**, for example with `rustup toolchain install` and no argument,
  so a bump stays one line. No workflow relies on an action's default toolchain:
  `dtolnay/rust-toolchain` installs `stable` by default, and its log names that, not the pin.
- **Two jobs name their toolchain explicitly.** The `msrv` job runs `cargo +1.96.0` on every
  command. The fuzz job runs `cargo +nightly-YYYY-MM-DD`. Each prints
  `rustc +<toolchain> --version`. The `+` is needed because rustup passes the selected toolchain
  to every process it starts, so a toolchain file under `fuzz/` would not apply when cargo runs
  from the root.
- **`--locked` everywhere it applies.** Every cargo command in CI that resolves dependencies passes
  `--locked`, including the record guard's `cargo metadata`. `cargo fmt` takes no such flag. A
  manifest change without its lockfile update then fails CI, instead of re-resolving on the runner
  to whatever was published an hour ago.
- **What `msrv` adds.** `gates` already refuses standard-library APIs newer than 1.96 through
  `incompatible_msrv`. The `msrv` job catches what that lint cannot see. One is a language feature
  newer than 1.96, in any target. The other is a dependency version whose own `rust-version` is
  above 1.96, which resolver 2 locks without complaint and Dependabot can propose.
- **Bumping the pin** is its own PR. It changes the one line, runs the gates on the new compiler,
  and fixes what that compiler finds rather than allowing it. The weekly job reports when a newer
  stable release exists. One that fixes a security problem in `std` on a supported platform is
  adopted without waiting, because `std` ships inside every binary. Raising `rust-version` is a
  separate decision.

### 4. Advisories, sources, licences and updates

`deny.toml` configures `cargo-deny` over every git-tracked lockfile. `cargo-deny` replaces
`cargo-audit`. It is pinned to a version that loads today's advisory database and installed with
`--locked`. On 2026-10-08, 0.20.2 loaded it and 0.18.5 did not.

| Check | Setting | Refuses a merge? |
| --- | --- | --- |
| Sources | crates.io only | Yes, on every PR, in `gates` |
| Vulnerability and unsound advisories | Denied unless ignored | Yes, in `gates`, on a PR that changes a `Cargo.toml` or `Cargo.lock` |
| Licences | `MIT`, `Apache-2.0`, `Unicode-3.0`; `NCSA` for `libfuzzer-sys` only | No; refused at release |
| Unmaintained, notice and yanked advisories | Warned | No; reported |

**Advisories.** A PR that changes a manifest or lockfile cannot merge while a vulnerability or
unsound advisory affects any locked package, unless that advisory is ignored. Advisories against
code already on main surface through the same check run on each push to main and weekly, not by
refusing unrelated PRs. Whether the owner wants every PR refused instead is Q9.

- **Ignores.** `cargo-deny` accepts only `id` and `reason` in an ignore entry. The reason carries
  the expiry and the link in a fixed form: `reason = "until 2026-12-01; <link>; <why>"`. A
  security-fix PR blocked by an unrelated advisory carries that advisory's ignore.
- **Tool failures.** A failure to fetch or load the advisory database, or any other tool error, is
  reported as a tool failure under its own message, never as an advisory refusal or a pass. On a PR
  that needs the check, it fails closed. A PR that bumps the pin fixes it.

**Licences** are reported, not refused, until something ships (Q8). The release step in spec 007,
once accepted, refuses them with the advisories. On a PR that changes a lockfile, the reviewer reads
the licence report.

**Bans are not used.** With `--locked`, a loose version requirement changes nothing until a reviewed
lockfile change.

**The `security` issue.** The check on each push to main, and a weekly scheduled run, cover every
lockfile with warnings included. The weekly run also flags ignores whose date has passed or whose
reason does not parse, a newer stable Rust, and the fuzz job's result. Any finding opens or updates
one GitHub issue labelled `security`. That is the job's whole duty, and it refuses nothing. The
later duty is Main's: within seven days, a task, a dated ignore or a bump PR, or the issue put to
the owner. The pipeline-health report lists open `security` issues with their age, so a missed week
shows.

**Dependabot for Cargo.** `.github/dependabot.yml` gains weekly `cargo` entries for the root and,
once it exists, for `fuzz/`, with a seven-day cooldown that security updates skip. An update that
moves a range never shares a PR with compatible updates. Dependabot groups by update type, not by
version position, and whether it calls `toml` 0.8 to 0.9 a minor update is unmeasured. If it does,
the config names the direct `0.x` crates in their own group: today `toml`, `toml_edit`, `libc` and
`criterion`. Every Dependabot PR passes the same gates as any other, and its reviewer files its task
(see the pr-review amendment).

### 5. Bytes from a less-trusted side

**The rule.** A parser whose input a less-trusted party can write states a bound. The bound is a
maximum size and, where it applies, a maximum nesting depth and a time budget.

- The parser checks the bound while reading, for example by reading through a limit of the bound
  plus one byte. A length check before an unbounded read does not meet this rule. A FIFO, or a
  symlink to `/dev/zero`, must be refused at the bound, not read forever.
- Past the bound it refuses with an error. It never truncates to a success.
- A parser whose writer is hostile also has a fuzz target. Hostile writers are a cell's workload,
  a plasmid author, a broker, and a control-socket client.
- A new parser lands in the same PR as its row below and its bound. That includes the host side of
  the guest bridges, guest observations under spec 024, and registry packages under spec 020.

**The inventory today:**

| Parser | Where | Who writes the bytes | Bound today | Needs |
| --- | --- | --- | --- | --- |
| Controller control line | `plasmosome-core` `control.rs`, `protocol.rs` | A control-socket client, including a poisoned agent | 1 MiB (`MAX_LINE_BYTES`), read through a limit; JSON depth 128 | Fuzz target |
| Broker readiness reply | `plasmosome-membrane` `readiness.rs` | A plasmid's broker | 1 MiB (`MAX_RESPONSE_BYTES`) within spec 001's readiness budget; JSON depth 128 | Fuzz target |
| Plasmid manifest | `plasmosome-core` `manifest.rs`, `load` and `parse` | A plasmid author | None: `load` reads the whole file. TOML depth 80 | A 1 MiB bound on both, read through a limit, matching spec 020's limit for a declaration; a fuzz target |
| Membrane control request | `plasmosome-membrane` `control.rs` | The controller, under spec 001's peer-UID check | 1 MiB (`MAX_REQUEST_BYTES`), read through a limit | Bound only |

The controller's own journals are not on this list: the session log, the ledger and spec 008's
recovery journal. Only the controller writes them. Any other process that could rewrite them runs
as the owner's user, such as a broker or a host agent. Such a process can already kill or replace
the controller, so a bound on reading the journals would protect nothing. Spec 008 governs torn
records.

**Fuzz targets.**

- They live in a `fuzz/` workspace that is never shipped. Its lockfile is covered by the records and
  by `cargo-deny`.
- The control-line target drives the controller's real request handler over a fake backend, so
  per-verb parameter parsing is fuzzed, not only framing.
- Every target asserts no panic within the fuzzer's memory and time limits. The control-line target
  also asserts two things: every reply is a well-formed JSON response, and the connection answers
  the next request after a malformed line.
- A minimized corpus, seeds plus every input that ever crashed a target, is committed in the
  owning crate. A test in that crate replays it under `cargo test`, so a fixed crash cannot return
  unnoticed. The replay finds nothing new; it guards only the saved inputs.
- A scheduled job runs each target for ten minutes a week, starting from the committed corpus. It
  uploads crash inputs as artifacts, and a red run reaches the `security` issue. It never blocks a
  merge, because what a fuzzer finds does not depend on the PR under test.

### 6. Reporting a vulnerability

`SECURITY.md` at the root says how to report: through GitHub's private vulnerability reporting,
plus any contact Q1 adds. Its scope is the crates and binaries here, the cell boundary they build,
and their supply chain. It links this spec's [threat model](#threat-model) for what is not covered
yet, rather than copying it. It promises an acknowledgement within the time Q1 settles, and a fix or
a stated decision before public disclosure. It never names a route nobody receives.

### 7. How each rule refuses

| Rule broken | Refused by | Why that is allowed under spec 013 |
| --- | --- | --- |
| `include`, source replacement, `[patch]`, `[replace]`, configured rustflags, `rustc`, wrapper, linker, runner or `env`, or a path dependency outside the repository | The git-only step, first in `gates`, before any cargo command | Each runs or swaps a program nobody reviewed at the first cargo command, on every machine that builds main, including the owner's, which holds a crates.io publish token. Code that ran cannot be un-run. |
| A locked dependency, or a new range of one, that no record names | The record guard, second in `gates` | The same harm, from build scripts and proc macros |
| A non-crates.io source | `cargo-deny` in `gates` | The same harm |
| A stale lockfile | `--locked` on every cargo command that resolves dependencies | Not a guard. It is cargo refusing to re-resolve. Without it, every check above reads a lockfile that is not what CI built. |
| A vulnerability or unsound advisory, on a PR that changes a manifest or lockfile | `cargo-deny` in `gates` | The merge brings that code onto main, which the owner builds and runs against hostile input, with no release step between. An exploit cannot be undone. |
| An advisory database or tool failure | The advisory step, as a tool failure | Fails closed: the step cannot vouch for the change |
| An unlisted licence | Reviewer at PR time; release step at release | Below the bar until something ships (Q8) |
| `unsafe` in a forbid package or outside an allowed item; a missing `// SAFETY:` comment or `# Safety` section | The compiler and clippy, through declared lint levels | Not a guard. See the spec 013 amendment. |
| `unsafe` expanded from a dependency's macro | The record's `expands_unsafe` field and the reviewer | Judgement, because no lint sees it |
| An `allow(unsafe_code)` wider than an item or a named FFI module; an undocumented `unsafe extern` declaration; joining the register without both lints or a reason | Reviewer | Judgement |
| Native code in a forbid package | The compiler, for linked code; the reviewer, for test-built programs | Judgement for the second |
| A dependency rule that is a judgement: the thousand-line rule, the narrowest crate, features, record contents, release age, a stale record, an approval link, a copied crate | Reviewer | Judgement |
| Running a contributor's PR before reading it | Reviewer | A rule about who does what |
| A dependency above `rust-version`, or a newer language feature | The `msrv` job, through pr-review's green-CI merge condition | Not a guard |
| An ignore without a date or link, or past its date | Reviewer; the weekly run flags it | Judgement |
| An advisory against code already on main; unmaintained crates; fuzz crashes; a newer stable Rust | Nothing refuses. The `security` issue, then Main | A rule about who does what. Refusing every PR for advisories is Q9. |
| The toolchain rules; Dependabot configuration and tasks; CI pins and permissions; Cargo settings in workflows | Reviewer | Revertible by the next commit |
| A parser without its bound, row or fuzz target | Reviewer | A design, revertible |
| A fixed crash that returns | The corpus replay test | An ordinary regression test |
| A guest syscall route left open | Spec 024's acceptance | Owned there |
| A native artifact without a record, or with an unchecked one | Reviewer; Q6 for the checking | Judgement |
| `SECURITY.md` naming a dead route | Reviewer | Judgement |

### 8. Which PRs this spec governs

This spec governs a PR, whatever task it maps to, when the PR changes any of these: a lockfile; a
manifest's dependency, `[lints]`, `[patch]` or `[replace]` tables; `.cargo/config*`, `build.rs`,
`clippy.toml`, `rust-toolchain*` or `deny.toml`; anything under `.github/`, `.githooks/` or
`docs/dependencies/`; `unsafe` or native code; a parser of less-trusted bytes or a fuzz corpus; or
`SECURITY.md`. Its reviewer checks the reviewer rows of section 7.

## Changes proposed to accepted specs and documents

This PR proposes the exact text below and edits none of these files. Each change lands with the
task named under it.

### Spec 013: lint levels, the step before cargo, and third-party tools

This lands with the `forbid` task, the first under this spec. Add after the paragraph that begins
"**The guards live in one crate, named for what they are.**":

> **Three things outside this crate fail builds.** A lint level a package declares in its
> manifest, such as `unsafe_code = "forbid"`, is part of that package's code, not a guard. Crossing
> it takes a visible change: the line itself, or a build setting that weakens lints, which
> [spec 026](026-security-standard.md#2-unsafe-code) refuses in committed Cargo configuration and
> puts before the reviewer in workflows. It does not reach `unsafe` that a dependency's macro
> expands; spec 026's dependency records cover that. Spec 026's git-only check of Cargo
> configuration runs before cargo, because any cargo command runs a configured `rustc` or wrapper.
> It is a script in `.githooks/`, mutation-tested from this crate. A supply-chain check by a pinned
> third-party tool, such as `cargo-deny`, runs as its own CI step. The last two are held to this
> spec's bar like any guard, and [spec 026](026-security-standard.md#7-how-each-rule-refuses)
> argues each one.

### Spec 003: where fuzzing sits

This lands with the fuzz task. Add after the paragraph that begins "A unit test exercises one
crate":

> **Fuzzing belongs to the unit layer, and runs two ways.** A fuzz target feeds arbitrary bytes to
> one parser through its crate's API. Its minimized corpus, committed in the owning crate, replays
> as an ordinary test, so `cargo test -p <crate>` answers for those saved inputs and finds nothing
> new. The open-ended search runs on a schedule in CI and reports rather than gates, because its
> findings do not depend on the PR under test.
> [Spec 026](026-security-standard.md#5-bytes-from-a-less-trusted-side) says which parsers need a
> target.

### Spec 004: the `msrv` job

This lands with whichever comes first: the task that builds this job, or the toolchain task.
Replace the `msrv` bullet under "Jobs" with:

> - **`msrv`**: installs Rust 1.96.0, then runs `cargo +1.96.0 check --locked --workspace
>   --all-targets`. The explicit `+1.96.0` is required. A `rust-toolchain.toml`
>   ([spec 026](026-security-standard.md#3-the-toolchain)) outranks the default toolchain an action
>   sets, so a bare `cargo check` would run the pinned compiler while the action's log reports 1.96.
>   Clippy's `incompatible_msrv` in `gates` already refuses newer standard-library APIs. This job
>   catches newer language features in any target, and dependencies whose own `rust-version` is
>   above 1.96.

Add to its Acceptance:

> - Each of these makes the `msrv` job fail while `gates` passes, shown by mutation in the PR that
>   adds the job: locking a dependency version whose `rust-version` is above 1.96, and a language
>   feature newer than 1.96 in an integration test.

### Root `AGENTS.md`: the safety comment

This lands with the `unsafe` register task. In "Style", replace "No inline `//` comments." with:

> No inline `//` comments, with one exception: a `// SAFETY:` comment directly above an `unsafe`
> block or `unsafe impl`, or above the statement that holds it, saying why it is sound there.
> Clippy requires it ([spec 026](docs/specs/026-security-standard.md#2-unsafe-code)).

This relaxes an existing rule rather than adding an instruction whose effect needs measuring under
decision 001. Clippy enforces it whether or not an agent reads the sentence.

### The pr-review skill: governed PRs, running before reading, and Dependabot

This lands with the `forbid` task. In section 2, after "checks every applicable acceptance item in
every governing spec.", add:

> A PR that changes any path or kind of code listed in
> [spec 026 section 8](../../../docs/specs/026-security-standard.md#8-which-prs-this-spec-governs)
> is also governed by spec 026, whatever its task maps to. Run no cargo command on a contributor's
> PR on your own machine until spec 026's git-only step has passed for that head, and read any
> change to `build.rs` or `rust-toolchain*` first. A Dependabot PR gets a Beads task under spec 026
> before it merges, filed by whoever reviews it, who adds the `task:` footer to the PR body and
> adds it again if Dependabot rewrites the body.

This is a rule about who does what, so it lands on its reasoning. It prevents three failures. A PR
adding a dependency under a task mapped to spec 017 is reviewed against spec 017 only, and nobody
opens the record. The reviewer's own machine runs a contributor's wrapper or build script. A
Dependabot PR merges with no task, outside spec 012's two shapes.

### The check-pipeline-health skill: the `security` issue

This lands with the `cargo-deny` task, which also extends `tools/check-pipeline-health`. After the
paragraph that begins "Read the actual JSON and exit status.", add:

> The `github` source lists open issues labelled `security`, each with its age. Report one older
> than seven days that has no linked task, ignore or bump PR as a missed duty under
> [spec 026](../../../docs/specs/026-security-standard.md#4-advisories-sources-licences-and-updates).

## Open questions for the owner

1. **Q1, the security contact.** Should `SECURITY.md` add a contact beside private vulnerability
   reporting, such as an email address? What acknowledgement time should it promise?
2. **Q2, whether the thousand-line rule is hard or advisory.** If hard, only the owner approves an
   exception. If advisory, the independent reviewer may accept a stated argument. Until it is
   answered, the owner approves.
3. **Q3, owner item O-7, the guest kernel policy.** The options are (a) a BPF LSM, (b) seccomp
   user-notify through a spec 017 amendment, or (c) a small LSM patch. This spec does not choose.
   (a) and (c) bring a kernel build under this standard; (b) must also close the io_uring and
   AF_VSOCK routes that draft 024's reviews found.
4. **Q4, crates already on main.** Your rule was written about taking on new packages. Should a
   crate already on main whose used part comes out under about a thousand lines be replaced?
   Plausible candidates are `uuid`, `tempfile` and `cc`. Until you answer, each keeps verdict
   `kept-pending-owner`.
5. **Q5, secret scanning push protection.** It is off. A secret pushed to a public repository
   cannot be withdrawn. Should it be turned on?
6. **Q6, the native artifacts' advisories.** Who tracks the guest kernel's CVEs, and libkrun's
   advisories, including those of libkrun's own Rust dependencies? How? Today they are pinned and
   not tracked.
7. **Q7, the Actions settings.** Should `sha_pinning_required` be turned on, making the action pin a
   platform refusal? Should `allowed_actions` be narrowed from `all`?
8. **Q8, spec 013's bar before anything ships.** Licences are reported, not refused, because nothing
   that ships carries a third-party crate yet. Should spec 013 admit cheap third-party checks before
   a release exists? If so, the licence check refuses in `gates`.
9. **Q9, how strictly advisories block.** This text refuses a PR for an advisory only when the PR
   changes a manifest or lockfile. Do you want every PR refused while any unignored advisory affects
   a locked package? That stops the line until someone acts, and a database the pinned tool cannot
   load would stop every merge. Spec 013's bar does not reach PRs that bring no new code.

## Acceptance

Each line names the broken implementation it catches.

**Dependency admission**

1. A manifest change pushed without its lockfile update fails `gates`. Catches CI that re-resolves
   on the runner, because some cargo command lacks `--locked`.
2. Each of these fails the git-only step, and a `rustc-wrapper` set to a script that logs its calls
   leaves the log empty:
   - `[patch]` or `[replace]` in a manifest;
   - a `.cargo/config.toml` with source replacement, `paths`, `rustflags` holding
     `--cap-lints allow`, or `rustc-wrapper`;
   - an extensionless `.cargo/config` with `rustflags`;
   - a `.cargo/config.toml` holding only `include`;
   - a path dependency outside the repository, and one reached through a tracked symlink.

   An in-tree path package passes. Catches a step that runs cargo first, and checks keyed on package
   names, which a vendored source with an edited `build.rs` passes.
3. A PR adding a package no record names fails the record guard, before clippy or the tests compile
   anything. This holds whether the package is direct or transitive. Catches a guard that reads only
   direct dependencies, and a guard ordered after compilation.
4. A lockfile that moves a package to a new range without a record change fails. Examples are
   `toml` 0.8 to 0.9, or a transitive `syn` 2 to 3. A compatible bump, such as `toml` 0.8.23 to
   0.8.24, passes. Catches a guard that matches names only.
5. A package only in `fuzz/Cargo.lock` without a record fails the record guard. The fuzz
   workspace's path packages from this repository do not. Catches a guard that reads only the root
   lockfile, and a definition of "dependency" that counts first-party crates.
6. Each refusal of the git-only step and the record guard is mutation-tested in the PR that adds it,
   as `crates/plasmosome-guards/AGENTS.md` requires.
7. When the record guard lands, `docs/dependencies/` holds a record for every direct dependency on
   main, with every field filled, the estimate included, and verdict `kept-pending-owner` until Q4
   is answered. Catches a record without the estimate the owner needs to answer Q4.
8. Each native artifact the runtime pins has a record stating its version, digest, source and check
   cadence. Catches "pinned" passed off as "tracked".

**`unsafe` code**

9. Each package outside the register refuses an `unsafe` block, shown once per package on its
   library or only target. Each kind of target refuses one somewhere: binary, integration test,
   bench and build script. Catches `#![forbid(unsafe_code)]` placed only in `lib.rs`, and a package
   left without the workspace lints.
10. Adding `#[allow(unsafe_code)]` beside new `unsafe` in a forbid package still fails. Catches
    `deny` where `forbid` is required.
11. In each register package, `unsafe` in a function whose module and ancestors carry no allow fails
    to compile, and the crate root carries no allow. Catches a package-wide `allow`.
12. In each register package, removing a `// SAFETY:` comment from a block that Linux compiles fails
    `gates`. A private `unsafe fn` without `# Safety` fails clippy. Catches a register table that
    dropped the clippy lints, and `missing_safety_doc` checking public items only.
13. When the task lands, every `unsafe` block on main carries a safety comment, and both the root
    gate on a Mac and `gates` on Linux pass. Catches comments added only to code that Linux
    compiles.

**The toolchain**

14. `rust-toolchain.toml` names an exact `x.y.z` release. No workflow passes a toolchain name except
    the `msrv` and fuzz jobs. Every other job's log prints a `rustc` version equal to the pin. The
    `msrv` and fuzz jobs print their own explicit toolchain. Catches an action that installs its
    default `stable`, and a nested toolchain file that rustup overrides.
15. Each of these makes the `msrv` job fail while `gates` passes: locking a dependency version whose
    `rust-version` is above 1.96, and a language feature newer than 1.96 in an integration test.
    Catches an `msrv` job without `+1.96.0`, and one without `--all-targets`.

**Advisories, sources, licences and updates**

16. Each of these fails `gates`: a git dependency, or a crate from another registry, in any
    git-tracked lockfile, the fuzz lockfile included. Catches `cargo-deny` run over the root
    manifest only.
17. With an unignored vulnerability advisory against a locked package, a PR that changes a
    `Cargo.toml` fails `gates`, and a Markdown-only PR passes. An ignore whose reason has the stated
    form lets the first pass. Pointing the check at a database it cannot load fails with a
    tool-failure message, not an advisory. Catches a trigger keyed on `Cargo.lock` alone, and a tool
    failure reported as a pass.
18. Two dispatched scheduled runs against a planted expired ignore leave exactly one open `security`
    issue, updated by the second run. The pipeline-health report then lists it with its age.
    Catches a job that opens a new issue every run, and a report that omits the issue.
19. `.github/dependabot.yml` has the weekly `cargo` entry and its cooldown. Its groups keep
    range-moving updates apart: a review compares any `0.x` names in the config with the direct
    dependencies below 1.0 in `cargo metadata`. A test branch mimicking a compatible bump that adds
    a package fails the record guard. Catches `toml` 0.8 to 0.9 grouped with patch updates, and a
    Dependabot path that bypasses the records.
20. No workflow installs a tool without a version and `--locked`. No `uses:` names a tag. Every
    workflow keeps least-privilege `permissions:` and `persist-credentials: false`. Catches a
    floating `cargo install cargo-deny`.

**Bytes from a less-trusted side**

21. `PlasmidManifest::load` refuses promptly with the size error for a FIFO that writes 1 MiB plus
    one byte and never closes, and for a symlink to `/dev/zero`. `parse` refuses a string over
    1 MiB, and a 1 MiB file is not refused for size. Catches a `len()` check followed by
    `read_to_string`, and a check after reading.
22. Fuzz targets exist for the control line, the readiness reply and the manifest. The control-line
    target drives the real request handler, and asserts well-formed replies and a connection that
    keeps answering. Catches a target that fuzzes only framing, or discards the result.
23. Planting a panic that one committed seed reaches makes `cargo test -p <crate>` fail, and
    reverting the panic restores green. Catches a replay test that never reads its corpus.
24. The scheduled fuzz job is not a required check and starts from the committed corpus. One
    dispatched run against a planted crash leaves an artifact holding the crashing input. Catches a
    job that blocks unrelated PRs, and an upload step pointed at the wrong path.

**Disclosure and the whole**

25. `SECURITY.md` names private vulnerability reporting and Q1's contact, links the threat model,
    and states Q1's acknowledgement time. Catches a file that names a route nobody receives.
26. Every rule in sections 1 to 6 has a row in section 7. Catches a rule that nothing checks.
27. The root gate in `AGENTS.md` is green.

## Out of scope

- **The guest syscall-filter mechanism.** Spec 024 and O-7 own it.
- **Signing release binaries, build provenance and reproducible builds.** These belong to spec 007.
- **Developer workstation tools**, beyond the pins the repo already holds.
- **Per-version audits of every package**, in the manner of `cargo-vet`, across 104 packages.
  Ranges, `--locked`, the cooldown and the advisory check cover how new code arrives. A compromised
  compatible release reaching the lockfile would be the evidence to reopen this choice.
