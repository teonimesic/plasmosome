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
`Cargo.lock` is named by a short review record in `docs/dependencies/`. A guard reads
`cargo metadata` as the first step of `gates`, before anything else compiles, and refuses a package
no record names. It also refuses the ways of swapping the code behind a recorded name: `[patch]`,
`[replace]`, source replacement or build settings in committed Cargo configuration, and path
dependencies outside the repository. A new dependency is admitted only when what the workspace
uses of it could not be written here in roughly a thousand lines. That rule is the owner's. It
stood in intent 006's first draft and moved out in PR #40, because it is a means and not a goal,
with a note that the spec carrying it was owed.

The other rules are shorter. Every cargo command in CI runs with `--locked`, so the lockfile is what
CI builds. Packages with no `unsafe` code forbid it. In the two that need it, each block carries a
safety argument that clippy checks. An exact `rust-toolchain.toml` pins the compiler. `cargo-deny`
refuses non-crates.io sources and known vulnerabilities on every PR. Every parser fed by a
less-trusted side checks a size bound while reading, and the three that face hostile writers get
fuzz targets. A weekly job opens one `security` issue, which Main acts on within seven days.
`SECURITY.md` routes reports through GitHub's private vulnerability reporting.

[Spec 013](013-what-earns-a-guard.md) still decides which refusals may fail the build: only those
whose harm the next commit cannot undo. [Section 7](#7-how-each-rule-refuses) argues each one. A
rule that fails the bar belongs to the reviewer, and [section 8](#8-reviewer-checklist) lists the
reviewer's items in one place.

**Platform scope.** The standard is macOS-first: it is written for Darwin hosts on arm64, where
cells are being built today. Linux hosts are deferred, not dropped, and every rule applies to them
unchanged. CI runs on Linux runners, so Darwin-only code is compiled and linted only by the root
gate on the author's Mac until a macOS CI job exists. That code is `darwin_group.c`, the macOS
blocks in `vmm.rs`, and five `unsafe` blocks in `readiness.rs` under
`cfg(not(target_os = "linux"))`.

**What exists today**, verified on main at a0a5411 on 2026-10-08:

- `cargo audit` 0.22.2 runs weekly and on PRs that change `Cargo.lock`. Its job is not required:
  branch protection requires only `gates`.
- Actions are pinned by SHA, and Dependabot updates them but not Cargo. Default workflow
  permissions are read-only. `allowed_actions` is `all` and `sha_pinning_required` is off.
- Private vulnerability reporting, Dependabot alerts and security updates, and secret scanning are
  on. Push protection is off. CodeQL's default setup covers Rust, C/C++, Python and Actions.
- There is no `cargo-deny`, toolchain file, `SECURITY.md`, fuzzing, `forbid(unsafe_code)` or
  committed Cargo configuration. CI floats `stable` and never passes `--locked`. None of the 71
  `unsafe` blocks has a safety comment.
- `rust-version` is 1.96. `gates` denies clippy's `incompatible_msrv`, which refuses
  standard-library APIs newer than that. Nothing checks newer language features or dependencies'
  own `rust-version`, and spec 004's `msrv` job is not built.
- The lockfile holds 104 third-party packages, reached through ten direct dependencies. `criterion`
  brings 65 of them, 32 through nothing else. 21 packages run a build script and 4 are proc macros.

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
3. **Hostile plasmid code.** A plasmid's manifest, its broker and the broker's readiness replies
   are written by its author, who may be anyone (intent 010).
4. **Supply-chain compromise.** This is a malicious or hijacked crate or release, a swapped source,
   an action, a compiler or a CI tool. It is defended by records, `--locked`, crates.io as the only
   source, a seven-day wait for new releases, and pinned actions and tools.

**Guest-side syscall filtering is in scope.** Whatever denies the workload a kernel object inside
the guest is part of the boundary this standard covers. That includes AF_VSOCK sockets, user
namespaces and io_uring. Reviews of draft spec 024 found two routes that a single-syscall filter
misses on the qualified 6.12 guest kernel:

- io_uring's `IORING_OP_SOCKET` opens a socket without calling `socket(2)`.
- A network namespace does not confine AF_VSOCK, so the workload could reach the host bridges on
  ports 4090 and 4091.

Spec 017 already requires that "all alternate syscall/loader paths" meet the same denial. Spec 024
owns the mechanism, and owner item O-7 shapes it. This spec names the surface and does not choose.

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
- **An agent running on the host as the owner's user.** Plasmosome does not confine it, and a
  same-UID process defeats every host-side check (spec 001). This is also why files that only the
  controller writes, such as its journals, are not less-trusted input under this standard.

## Contract

### 1. Dependency admission

**What counts.** A dependency is a package with a registry or git source in a git-tracked
`Cargo.lock`. Today that means the root lockfile. Later it also means the fuzz workspace's.

- Path packages inside this repository are first-party code. They are reviewed as code in the diff
  and need no record. Copying a third-party crate into the tree makes every line of it this
  project's code, reviewed as such.
- A path dependency outside the repository is refused.
- Build and dev dependencies count fully. Their build scripts, proc macros and tests run on every
  machine that builds or tests the tree.

**When a dependency may be added.** All of these must hold:

1. It comes from crates.io.
2. No unignored vulnerability or unsound advisory affects it (section 4).
3. What the workspace uses of it could not be written here in roughly a thousand lines, or the
   owner approved an exception.
4. It is the narrowest crate, with the fewest features, that does the job. Default features are off
   unless the record says why they are on.
5. Every version the lockfile diff adds has been public for seven days, unless it fixes an
   advisory. Dependabot's cooldown enforces this on its own PRs. On any other PR it is a reviewer
   item.
6. A record exists, and the independent reviewer checked it.

PR #123 is the model for item 4. Its review found that the membrane's strict JSON reader needs only
serde's deserialization traits, which `serde_core` provides. Switching from `serde` with `derive` to
`serde_core` took the membrane's normal dependency tree from 20 crates to 14, with no behavior lost.

**The thousand-line rule.** What matters is what writing the used part here would take, not the
size of the crate. A 40,000-line crate whose two functions we call could be 200 lines of our own.
Under roughly a thousand lines, write it here. An exception needs an argument in the record and an
approval.

- Until owner question Q2 is answered, only the owner approves an exception.
- The record links the place where the approval was given, such as a PR comment.
- As with intents, an agent may record an approval it is carrying. It may never originate one.

**The record.** Each direct dependency has one file, `docs/dependencies/<crate>.toml`. Every field
is required.

| Field | What it holds |
| --- | --- |
| `name`, `range` | The crate, and the semver-compatible range reviewed, such as `0.8` or `1` |
| `used_by` | Each workspace package that names it, with the kind: `normal`, `build` or `dev` |
| `reason` | What it does for us, in a sentence or two |
| `write_it_instead` | Estimated lines to write what we use, and how the estimate was reached |
| `exception` | Empty, or the argument and a link to the owner's approval |
| `features` | The features enabled across the workspace, and whether default features are on |
| `brings` | Every package in its resolved closure, as `name@range`, normal and build, all platforms |
| `build_time_code` | Build scripts and proc macros in it and in `brings` that build for a supported host |
| `expands_unsafe` | Macros in it or in `brings` that expand `unsafe` into our code, or `none` |
| `native_code` | Native code it compiles or links, or `none` |
| `verdict` | `admitted`, `grandfathered` or `replace` |

A range is the semver-compatible part of a version: `0.8` for 0.8.23, and `1` for 1.0.229. A
package that moves to a new range is new code, so the record that names it must change. Because
`brings` is a closure, a package shared by two dependencies appears in both records, and two people
derive the same list.

**What the review checks.** The PR's independent reviewer does all of these:

- Re-derives the record's lists from the crate at the locked version, re-deriving only the entries
  that changed.
- Reads in full every build script and proc macro the change brings that builds for a supported
  host, and every macro that expands `unsafe` into our code. The supported hosts are
  `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`, where CI runs.
- Puts in the review comment the measurements the record does not keep: lines of Rust, `unsafe`
  counts and crates.io owners.
- Runs `cargo tree -e normal -p <package>` before and after, and asks whether a narrower crate or
  fewer features would serve.

An advisory pass is not a review. On PR #123, `cargo audit` found 0 advisories over 120 crates. That
shows nobody has reported a problem, not that anybody read the code.

**Read before building.** This rule applies before a contributor's PR is built on a machine that
holds credentials, when the PR changes a lockfile, a dependency table, a `build.rs` or a
`.cargo/config*` file. The reviewer, a person or an agent, first runs the guard's metadata-only
check or reads its result from that head's first `gates` step. `cargo metadata` runs no build
script or proc macro. The reviewer then reads the build scripts and proc macros of every package
the lockfile adds or changes.

**Dependencies on main, and dependencies being added now.**

- The guard's task writes a re-derived record for every direct dependency on main. Today that is
  ten crates: `serde`, `serde_json`, `uuid`, `toml`, `toml_edit`, `libc`, `cc`, `proptest`,
  `tempfile` and `criterion`. It adds `sha2` and `serde_core` if PR #123 merges first.
- These records carry verdict `grandfathered`. A record whose `write_it_instead` comes out under
  about a thousand lines, with no approved exception, gets `replace` and a follow-up task under this
  spec instead.
- Records may land first, a few at a time, in docs-only PRs under this spec's task. The guard
  lands once they all exist.
- Until the guard lands, a PR that adds a dependency writes the record's facts into the PR, as
  PR #123 did. A PR still open when the guard lands carries its own record.
- A PR that removes the last direct use of a crate deletes its record, so the record stops vouching
  for its `brings`.

**The guard.** The guard is `every_dependency_has_a_review_record`, in `crates/plasmosome-guards`.
It runs as the first step of `gates`. It reads `cargo metadata --locked` and the git-tracked Cargo
configuration files, and builds nothing. It has two clauses:

1. **An unrecorded package.** A locked dependency whose `name@range` is neither a record's own nor
   in some record's `brings`.
2. **A swapped source.** Any of these:
   - `[patch]` or `[replace]` in a manifest;
   - a git-tracked `.cargo/config*` that sets source replacement, `paths` or `patch`, or any
     `rustflags`, `rustc`, wrapper, `linker` or `runner` key;
   - a path dependency outside the repository.

   A `[patch]`, `[replace]` or source replacement passes only when a record names it.

Its message names the package or setting, and the harm: code that nobody reviewed runs at build
time on every machine that builds main, including ones that hold publish credentials. Compiling the
guard builds only `plasmosome-guards`' own dependencies. A PR that changes those is the case the
read-before-building rule covers. The guard checks that records exist and name what is locked.
Whether their contents are right is the reviewer's to judge.

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
benches, examples and build script.

Today seven packages inherit them: `plasmosome`, `plasmosome-backend`, `plasmosome-ledger`,
`plasmosome-testkit`, `plasmosome-guards`, `plasmid` and `plasmid-sdk`. A `#![forbid(unsafe_code)]`
in `lib.rs` alone does not meet this rule, because it leaves tests, benches and binaries open.

**The register** is the set of packages whose own `[lints]` table sets `unsafe_code = "deny"`.
Cargo does not let a package mix `lints.workspace = true` with local keys. So a register package
repeats both clippy safety lints at `deny` in its own table. Today the register holds two packages:

- **`plasmosome-membrane`.** It has 68 blocks: the VMM child, readiness probes, broker and exec
  launch, signal handling and its tests. It also has one `unsafe extern` block for `darwin_group.c`.
- **`plasmosome-core`.** It has three blocks: the shutdown signal handler in `plasmosomed`, and two
  in its tests.

Inside a register package, `#[allow(unsafe_code)]` goes on each item that needs `unsafe`. It goes on
a whole module only when the crate's `AGENTS.md` names that module as an FFI module. The crate root
never carries it. The register uses `deny`, not `forbid`, because an inner `allow` cannot lift
`forbid`.

**The safety argument.** Every `unsafe` block and `unsafe impl` carries a `// SAFETY:` comment above
it, saying which invariant makes it sound there and what establishes it. Every `unsafe fn`, private
ones included, carries a `# Safety` doc section. A root `clippy.toml` sets
`check-private-items = true`, because `missing_safety_doc` otherwise checks only public functions.

An `unsafe extern` block documents what each declaration assumes about the foreign side. No lint
reads that, so it is a reviewer item. The safety comment is the one inline `//` comment the code
carries. That needs the `AGENTS.md` amendment below.

**What the lint levels do not reach.**

- **`unsafe` expanded from another crate's macro.** rustc does not report lints raised inside
  external macros. So `forbid` lets a dependency's `macro_rules!` or derive put `unsafe` into a
  forbid package, and no safety comment is asked for. The record's `expands_unsafe` field, and the
  reviewer who reads those macros, cover this.
- **Build settings that weaken lints.** `--cap-lints` or `-A` in rustflags silences both the forbid
  and clippy. The guard refuses rustflags in committed Cargo configuration. A workflow's `RUSTFLAGS`
  is a reviewer item, because the guard does not parse YAML.

**Native code is `unsafe` code.** Only register packages compile or link it. In a forbid package
the compiler refuses the `unsafe extern` block that calling it needs. A test can also compile and
run C as a separate program, as the membrane does with `supervision_worker.c`. That needs no
`extern` block, so it is a reviewer item: only register packages may do it.

The fuzz workspace is the one exception. It is never shipped, and it compiles libFuzzer's C++
through `libfuzzer-sys`. Its own code still forbids `unsafe`, as any package outside the register
does.

**Gaining `unsafe` later.** A package joins the register in its own PR. The PR gives the package
its own `[lints]` table with `unsafe_code = "deny"` and both clippy lints, adds the scoped `allow`s
and safety comments, updates the crate's `AGENTS.md`, and says why no safe route exists. Leaving the
register is the reverse change, and it is welcome.

### 3. The toolchain

- **The pin.** `rust-toolchain.toml` names one exact stable release, with clippy and rustfmt. At
  the time of writing that is `channel = "1.99.0"`. It is never `stable` and never `1.99`.
- **CI installs what the file names.** For example, `rustup toolchain install` with no argument
  reads the file, so a bump stays one line. No workflow relies on an action's default toolchain.
  `dtolnay/rust-toolchain` defaults to `stable` and only runs `rustup default`. The file outranks
  that default, while the action's log prints what it installed.
- **Two jobs name their toolchain explicitly.** The `msrv` job runs `cargo +1.96.0` on every
  command. The fuzz job runs `cargo +nightly-YYYY-MM-DD`. Each prints
  `rustc +<toolchain> --version`. The `+` is needed because rustup passes the selected toolchain
  to every process it starts, so a toolchain file under `fuzz/` would not apply when cargo runs
  from the root.
- **`--locked` everywhere.** Every cargo command in CI passes `--locked`, including the guard's
  `cargo metadata`. A manifest change without its lockfile update then fails CI, instead of
  re-resolving on the runner to whatever was published an hour ago.
- **What `msrv` adds.** `gates` already refuses standard-library APIs newer than 1.96 through
  `incompatible_msrv`. The `msrv` job catches what that lint cannot see. One is a language feature
  newer than 1.96. The other is a dependency version whose own `rust-version` is above 1.96, which
  resolver 2 locks without complaint and Dependabot can propose.
- **Bumping the pin** is its own PR. It changes the one line, runs the gates on the new compiler,
  and fixes what that compiler finds rather than allowing it. PR #120's rename of `fetch_update`
  has the right shape. A Rust release that fixes a security problem in `std` on a supported
  platform is adopted without waiting, because `std` ships inside every binary. Raising
  `rust-version` is a separate decision.

### 4. Advisories, sources, licences and updates

`deny.toml` configures `cargo-deny` over every git-tracked lockfile. `cargo-deny` is pinned and
installed with `--locked`, and it replaces `cargo-audit`.

| Check | Setting | Refuses a merge? |
| --- | --- | --- |
| Sources | crates.io only | Yes, on every PR, in `gates` |
| Vulnerability and unsound advisories | Denied unless ignored | Yes, on every PR, in `gates` |
| Licences | `MIT`, `Apache-2.0`, `Unicode-3.0`; `NCSA` for `libfuzzer-sys` only | No; refused at release |
| Unmaintained, notice and yanked advisories | Warned | No; the weekly job reports them |

**Advisories block every PR.** A newly published advisory against any locked package blocks every
merge until it is fixed or ignored, whether or not the PR touched a lockfile. An ignore is an
entry in `deny.toml` with the advisory's ID, a reason, a link and an expiry date. A security-fix PR
blocked by an unrelated advisory carries that advisory's ignore.

**Licences** are reported, not refused, before anything ships. Spec 013's bar is not met until a
release exists (Q8). The release step in spec 007, once accepted, refuses them along with the
advisories. On a PR that changes a lockfile, the reviewer reads the licence report. The `NCSA`
exception applies to `libfuzzer-sys` in the fuzz workspace, which is never shipped.

**Bans are not used.** With `--locked`, a loose version requirement changes nothing until a reviewed
lockfile change.

**The weekly job and its issue.** Each week the job runs every check over every lockfile, warnings
included. It also lists ignores past their expiry and reads the fuzz job's result. When it finds
anything, it opens or updates one GitHub issue labelled `security`. Main reads that issue. Within
seven days, Main produces one of these: a task, an ignore with its reason, link and expiry, or a
bump PR. Main puts the issue to the owner instead when it needs a decision. The job itself refuses
nothing. The issue, and Main's duty to act on it, are what make it more than a report.

**Dependabot for Cargo.** `.github/dependabot.yml` gains a weekly `cargo` entry:

- patch updates, and minor updates of crates at 1.0 or above, are grouped into one PR;
- minor updates of `0.x` crates are grouped separately, because each moves a range;
- each major update gets its own PR;
- a seven-day cooldown applies, which security updates skip.

Every Dependabot PR passes the same gates as any other. How a Dependabot PR enters the work chain
is the spec 012 amendment below.

### 5. Bytes from a less-trusted side

**The rule.** A parser whose input a less-trusted party can write states a bound. The bound is a
maximum size and, where it applies, a maximum nesting depth and a time budget.

- The parser checks the bound while reading, for example by reading through a limit of the bound
  plus one byte. A length check before an unbounded read does not meet this rule. A FIFO, or a
  symlink to `/dev/zero`, must be refused at the bound, not read forever.
- Past the bound it refuses with an error. It never truncates to a success.
- A parser whose writer is hostile also has a fuzz target. Hostile writers are a cell's workload,
  a plasmid author, a broker, and a control-socket client.
- A new parser lands in the same PR as its row below and its bound.

**The inventory today:**

| Parser | Where | Who writes the bytes | Bound today | Needs |
| --- | --- | --- | --- | --- |
| Controller control line | `plasmosome-core` `control.rs`, `protocol.rs` | A control-socket client, including a poisoned agent | 1 MiB (`MAX_LINE_BYTES`), read through a limit; JSON depth 128 | Fuzz target |
| Broker readiness reply | `plasmosome-membrane` `readiness.rs` | A plasmid's broker | 1 MiB (`MAX_RESPONSE_BYTES`) within spec 001's readiness budget; JSON depth 128 | Fuzz target |
| Plasmid manifest | `plasmosome-core` `manifest.rs`, `load` and `parse` | A plasmid author | None: `load` reads the whole file. TOML depth 80 | A 1 MiB bound on both, read through a limit, matching spec 020's limit for a declaration; a fuzz target |
| Membrane control request | `plasmosome-membrane` `control.rs` | The controller, under spec 001's peer-UID check | 1 MiB (`MAX_REQUEST_BYTES`), read through a limit | Bound only |

The controller's own journals are not on this list: the session log, the ledger and spec 008's
recovery journal. Only the controller writes them, and only a same-user process can change them,
which the threat model leaves out. Spec 008 governs torn records.

**Rows added when the code exists:**

| Parser | Governing spec | Who writes the bytes | Needs |
| --- | --- | --- | --- |
| Host side of the guest bridges, ports 4090 and 4091 | 001 §4.2 | The guest shim, and the workload if it reaches AF_VSOCK | 001 §1's 1 MiB line limit; fuzz target |
| `cell.exec` output and guest observations | Draft 024 | The workload | A bound stated by spec 024; fuzz target |
| Registry package descriptor and declarations | 020 §2 | Any publisher | Spec 020's limits; fuzz target |
| Runtime recipe | 001 §4.2 | The operator, after the file's trust checks | A bound only |

**The guest's syscall interface is input too.** The filter that limits the workload's syscalls
inside the guest is in scope, as the threat model says. Spec 024 owns its mechanism and its
acceptance.

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
  uploads crash inputs as artifacts, and a red run reaches the weekly `security` issue. It never
  blocks a merge, because what a fuzzer finds does not depend on the PR under test.

### 6. Reporting a vulnerability

`SECURITY.md` at the root covers four things:

- **How to report.** Through GitHub's private vulnerability reporting, which is enabled, plus any
  contact Q1 adds.
- **Scope.** The crates and binaries here, the cell boundary they build, and their supply chain.
- **What is not covered yet.** It links this spec's [threat model](#threat-model) rather than
  copying it.
- **What a reporter can expect.** An acknowledgement within the time Q1 settles, and a fix or a
  stated decision before public disclosure.

It never names a route nobody receives.

### 7. How each rule refuses

| Rule broken | Refused by | Why that is allowed under spec 013 |
| --- | --- | --- |
| A locked dependency, or a new range of one, that no record names | Guard clause 1, first step of `gates` | Its build-time code runs on every machine that builds main, including the owner's, which holds a crates.io publish token. Code that ran cannot be un-run. |
| `[patch]`, `[replace]`, source replacement, Cargo configuration rustflags or wrappers, or a path dependency outside the repository | Guard clause 2 | The same harm. Each one runs or swaps build-time code that no record names. |
| A non-crates.io source | `cargo-deny` in `gates` | The same harm |
| A stale lockfile | `--locked` on every cargo command in CI | Not a guard. It is cargo refusing to re-resolve. Without it, every check above reads a lockfile that is not what CI built. |
| A vulnerability or unsound advisory | `cargo-deny` in `gates`, on every PR | Main is what the owner builds and runs against hostile input, and no release step sits between a merge and that code. An exploit cannot be undone. The tool cannot tell which advisories are reachable, so dated ignores carry that judgement. |
| An unlisted licence | Reviewer at PR time; release step at release | Below the bar until something ships (Q8) |
| `unsafe` in a forbid package, or outside an allowed item in a register package | The compiler, through the declared lint level | Not a guard. Crossing it takes a visible edit, or a Cargo setting that guard clause 2 refuses. See the spec 013 amendment. |
| A missing `// SAFETY:` comment or `# Safety` section | Clippy, in `gates` and the root gate | Not a guard: a lint level |
| `unsafe` expanded from a dependency's macro | The record's `expands_unsafe` field and the reviewer | Judgement, because no lint sees it |
| An `allow(unsafe_code)` wider than an item or a named FFI module; an undocumented `unsafe extern` declaration; joining the register without both lints or a reason | Reviewer | Judgement |
| Native code in a forbid package | The compiler, for linked code; the reviewer, for test-built programs | Judgement for the second |
| A dependency rule that is a judgement: the thousand-line rule, the narrowest crate, features, a new direct use, record fields, release age, a stale record, an approval link | Reviewer | Judgement |
| Building before reading | Reviewer | A rule about who does what |
| The toolchain rules: CI's toolchain, the explicit `msrv` and fuzz toolchains, bump PRs | Reviewer | Revertible by the next commit |
| A dependency above `rust-version` | The `msrv` job, through pr-review's green-CI merge condition | Not a guard |
| An ignore without a reason, link or expiry | Reviewer; expiry reported weekly | Judgement |
| Unmaintained crates, expired ignores, fuzz crashes | Nothing refuses. The weekly issue and Main act on them. | A rule about who does what |
| Dependabot configuration; CI pins and permissions; workflow `RUSTFLAGS` | Reviewer | Revertible by the next commit |
| A parser without its bound, row or fuzz target | Reviewer | A design, revertible |
| A fixed crash that returns | The corpus replay test | An ordinary regression test |
| A guest syscall route left open | Spec 024's acceptance | Owned there |
| A native artifact without a record, or with an unchecked one | Reviewer; Q6 for the checking | Judgement |
| `SECURITY.md` naming a dead route | Reviewer | Judgement |

### 8. Reviewer checklist

This spec governs a PR, whatever task it maps to, when the PR changes any of these: a lockfile; a
manifest's dependency, `[lints]`, `[patch]` or `[replace]` tables; `.cargo/config*`, `build.rs`,
`clippy.toml`, `rust-toolchain.toml` or `deny.toml`; anything under `.github/` or
`docs/dependencies/`; `unsafe` or native code; a parser of less-trusted bytes or a fuzz corpus; or
`SECURITY.md`.

The reviewer checks these items:

1. **Before building.** A contributor's PR is not built on a machine holding credentials until the
   metadata-only check has run and the new build-time code has been read (section 1).
2. **Records.** The changed entries are re-derived. Build scripts, proc macros and
   `unsafe`-expanding macros for supported hosts are read. The measurements are in the review
   comment. The thousand-line estimate is plausible. An exception links the owner's approval. The
   features and new direct uses match the record. A crate no longer used directly has lost its
   record.
3. **Release age.** Every version the lockfile diff adds is at least seven days old, unless it fixes
   an advisory or the PR is Dependabot's.
4. **`deny.toml`.** An ignore has its reason, link and expiry. A licence report on a lockfile change
   is clean, or the PR adds the licence with a reason.
5. **The register.** A new workspace member inherits the workspace lints. A package joining the
   register keeps both clippy lints at `deny`, says why no safe route exists, and updates its
   `AGENTS.md`.
6. **Safety arguments.** Each new or changed argument is true at that point. Each `unsafe extern`
   declaration is documented. Test-built native code sits only in register packages.
7. **Parsers.** A new parser has its row and a bound read through a limit, and a fuzz target when
   its writer is hostile.
8. **Toolchain and CI.** A bump fixes new lints rather than allowing them. No workflow relies on a
   default toolchain or sets `RUSTFLAGS` that weaken lints. Tools are pinned with `--locked`, and
   actions by SHA. `permissions:` is least-privilege, `persist-credentials: false` is kept, and
   there is no `pull_request_target`.
9. **Native artifacts.** A changed pin updates its record.

## Changes proposed to accepted specs and documents

This PR proposes the exact text below and edits none of these files. Each change lands with the
task named under it.

### Spec 013: lint levels and third-party tools

This lands with the `forbid` task, the first under this spec. Add after the paragraph that begins
"**The guards live in one crate**":

> **Two kinds of build failure are not guards in this sense.** A lint level a package declares in
> its manifest, such as `unsafe_code = "forbid"`, is part of that package's code. Crossing it takes
> a visible change. Either the line itself is edited, or a build setting weakens lints, which
> [spec 026](026-security-standard.md#2-unsafe-code) refuses in committed Cargo configuration and
> puts before the reviewer in workflows. The lint level does not reach `unsafe` that a dependency's
> macro expands; spec 026's dependency records cover that. A supply-chain check run by a pinned
> third-party tool, such as `cargo-deny`, runs as its own CI step rather than in this crate. It is
> held to this spec's bar like a guard, and its configuration states the harm it refuses.
> [Spec 026](026-security-standard.md#7-how-each-rule-refuses) argues each one.

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
>   catches newer language features, and dependencies whose own `rust-version` is above 1.96.

Add to its Acceptance:

> - Locking a dependency version whose `rust-version` is above 1.96 makes the `msrv` job fail while
>   `gates` passes, shown by mutation in the PR that adds the job.

### Spec 012: Dependabot PRs

This lands with the Dependabot task. Add a third item to the list after "Exactly two shapes carry no
task:", and change "two" to "three":

> - A PR opened by Dependabot that changes only dependency versions: lockfiles, version
>   requirements in manifests, and pinned action SHAs. [Spec 026](026-security-standard.md)
>   governs its review. A Dependabot PR that needs more than that, such as a code change to absorb
>   a breaking update, gets a task under spec 026 before that change is pushed.

### Root `AGENTS.md`: the safety comment

This lands with the `unsafe` register task. In "Style", replace "No inline `//` comments." with:

> No inline `//` comments, with one exception: a `// SAFETY:` comment directly above an `unsafe`
> block or `unsafe impl`, saying why it is sound there. Clippy accepts it only in that position
> ([spec 026](docs/specs/026-security-standard.md#2-unsafe-code)).

This relaxes an existing rule rather than adding an instruction whose effect needs measuring under
decision 001. Clippy enforces it whether or not an agent reads the sentence.

### The pr-review skill: which PRs this spec governs

This lands with the `forbid` task. In section 2, after "checks every applicable acceptance item in
every governing spec.", add:

> A PR that changes any path or kind of code listed in
> [spec 026's reviewer checklist](../../../docs/specs/026-security-standard.md#8-reviewer-checklist)
> is also governed by spec 026, whatever its task maps to. Check that list, and do not build a
> contributor's PR that changes a lockfile, a dependency table, a `build.rs` or a `.cargo/config*`
> file on a machine holding credentials before its build-time code has been read.

This is a rule about who does what, so it lands on its reasoning. Here is the failure it prevents.
A PR adding a dependency under a task mapped to spec 017 is reviewed against spec 017 only.
Nobody opens the record, and the reviewer's own machine runs the new build script.

## Open questions for the owner

1. **Q1, the security contact.** Should `SECURITY.md` add a contact beside private vulnerability
   reporting, such as an email address? What acknowledgement time should it promise?
2. **Q2, whether the thousand-line rule is hard or advisory.** If hard, only the owner approves an
   exception. If advisory, the independent reviewer may accept a stated argument. Until it is
   answered, the owner approves.
3. **Q3, owner item O-7, the guest kernel policy.** The options are (a) a BPF LSM, (b) seccomp
   user-notify through a spec 017 amendment, or (c) a small LSM patch. This spec does not choose.
   - (a) and (c) bring a kernel build under this standard.
   - Draft 024's reviews show that (b) must also close the io_uring and AF_VSOCK routes.
4. **Q4, owner item O-11's dependency.** If guest TCP uses smoltcp, it needs a record like any
   other. Its size passes the thousand-line rule. Should the record be written before O-11 is
   decided, as input to the decision?
5. **Q5, secret scanning push protection.** It is off. A secret pushed to a public repository
   cannot be withdrawn. Should it be turned on?
6. **Q6, the native artifacts' advisories.** Who tracks the guest kernel's CVEs, and libkrun's
   advisories, including those of libkrun's own Rust dependencies? How? Today they are pinned and
   not tracked.
7. **Q7, the Actions settings.** These are admin settings, not guards:
   - Should `sha_pinning_required` be turned on? It would make the action pin a platform refusal.
   - Should `allowed_actions` be narrowed from `all`?
   - Should the default workflow `permissions` stay read-only, and be kept that way?
8. **Q8, spec 013's bar before anything ships.** Licences are reported and not refused today,
   because nothing that ships carries a third-party crate yet. Should spec 013 admit cheap
   third-party checks before a release exists? If it should, the licence check moves into `gates`
   as a refusal.

## Acceptance

Each line names the broken implementation it catches.

**Dependency admission**

1. A manifest change pushed without its lockfile update fails `gates`. Catches CI that re-resolves
   on the runner, because some cargo command lacks `--locked`.
2. A PR adding a package no record names fails the first step of `gates`, before clippy or the tests
   compile anything. This holds whether the package is direct or transitive. Catches a guard that
   reads only direct dependencies, and a guard ordered after compilation.
3. A lockfile that moves a package to a new range without a record change fails. Examples are
   `toml` 0.8 to 0.9, or a transitive `syn` 2 to 3. A compatible bump, such as `toml` 0.8.23 to
   0.8.24, passes. Catches a guard that matches names only.
4. Each of these fails guard clause 2:
   - `[patch]` or `[replace]` in a manifest;
   - a `.cargo/config.toml` with `[source]` replacement, `paths`, `rustflags` holding
     `--cap-lints allow`, or `rustc-wrapper`;
   - a path dependency outside the repository.

   An in-tree path package does not fail it. Catches checks keyed on package names, which a
   vendored source with an edited `build.rs` passes.
5. A package only in `fuzz/Cargo.lock` without a record fails the guard. The fuzz workspace's path
   packages from this repository do not. Catches a guard that reads only the root lockfile, and a
   definition of "dependency" that counts first-party crates.
6. The guard's PR mutation-tests both clauses. It adds each violation, observes the failure and
   reverts it, as `crates/plasmosome-guards/AGENTS.md` requires.
7. When the guard lands, `docs/dependencies/` holds a record for every direct dependency on main,
   with every field filled and re-derived in review. A record under about a thousand lines without
   a linked approval has verdict `replace`. Catches blanket grandfathering.
8. Each native artifact the runtime pins has a record stating its version, digest, source and check
   cadence. Catches "pinned" passed off as "tracked".

**`unsafe` code**

9. Each of the seven forbid packages refuses an `unsafe` block, shown once per package on its
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
15. Locking a dependency version whose `rust-version` is above 1.96 makes the `msrv` job fail while
    `gates` passes. Catches an `msrv` job without `+1.96.0`, which would check the pinned compiler.

**Advisories, sources, licences and updates**

16. Each of these fails `gates`: a git dependency, or a crate from another registry, in any
    git-tracked lockfile, the fuzz lockfile included. Catches `cargo-deny` run over the root
    manifest only.
17. With an unignored vulnerability advisory against a locked package, a PR that changes only
    Markdown fails `gates`. An ignore with its reason, link and expiry lets it pass. Catches
    advisories checked only on lockfile changes.
18. A scheduled run that finds an unmaintained advisory or an expired ignore opens or updates the
    one `security` issue. Catches a report that nobody receives.
19. `.github/dependabot.yml` has the weekly `cargo` entry, with its groups and its seven-day
    cooldown. A test branch mimicking a compatible bump that adds a package fails the guard. Catches
    a Dependabot path that bypasses the records.
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
24. The scheduled fuzz job is not a required check, starts from the committed corpus, and uploads
    crash inputs as artifacts. Catches a nondeterministic job that blocks unrelated PRs, and a
    crash lost with its runner.

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
- **Linux hosts**, deferred with the platform.
