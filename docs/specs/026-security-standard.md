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
compiler builds the tree, which advisories, licences and sources are accepted, how bytes from a
less-trusted side are parsed, and how a vulnerability is reported. For each rule it says what
breaks it and what refuses the change: CI, the independent reviewer, or both.

The rule with the most reach is dependency admission. Every third-party package in every
`Cargo.lock` in the tree is named by a review record in `docs/dependencies/`, and a guard refuses a
lockfile holding a package no record names. A new dependency is admitted only when what the
workspace uses of it could not be written here in roughly a thousand lines. If it could, write it
instead. That rule is the owner's. It stood in intent 006's first draft and moved out in PR #40,
because it is a means and not a goal, with a note that the spec carrying it was owed. The PR's
independent reviewer re-derives each record from the crate's own source, and reads every build
script and proc macro the addition brings in.

The other rules are shorter. Every package with no `unsafe` code forbids it for all its targets,
so adding some fails to compile. In the two packages that need it, each `unsafe` block carries a
safety argument that clippy checks. A `rust-toolchain.toml` pins one exact compiler, and CI installs
exactly that one. `cargo-deny` refuses unknown sources, unlisted licences and wildcard versions on
every PR, and refuses known vulnerabilities on every PR that changes a lockfile. Every parser fed by
a less-trusted side states a size bound and checks it while reading. The parsers that face hostile
writers also get fuzz targets, whose saved inputs replay under `cargo test`. `SECURITY.md` routes
reports through GitHub's private vulnerability reporting, which is already on.

[Spec 013](013-what-earns-a-guard.md) still decides which refusals may fail the build: only those
whose harm the next commit cannot undo. [Section 7](#7-how-each-rule-refuses) argues each one. Where
the argument fails, the refusal belongs to the reviewer, and [section 8](#8-reviewer-checklist)
lists the reviewer's items in one place, so a review can be checked against them.

**Platform scope.** The standard is macOS-first: it is written for Darwin hosts on arm64, where
cells are being built today. Linux hosts are deferred, not dropped. Every rule applies unchanged
when they arrive. CI runs on Linux runners, so Darwin-only code is compiled and linted only by the
root gate on the author's Mac until a macOS CI job exists. Today that code is `darwin_group.c` and
the macOS blocks in `vmm.rs`.

**What exists today**, verified on main at 379962b on 2026-10-08:

- `cargo audit` 0.22.2, pinned, runs weekly and on PRs that change `Cargo.lock`
  (`.github/workflows/audit.yml`). Its job is not a required check. Branch protection requires only
  `gates`, so a red `audit` run refuses nothing by itself.
- Every action is pinned to a commit SHA, and Dependabot proposes action updates. Dependabot does
  not propose Cargo version updates.
- Repository settings: Dependabot alerts and security updates are on. Private vulnerability
  reporting is on. Secret scanning is on and push protection is off. CodeQL's default setup covers
  Rust, C/C++, Python and Actions, weekly and on PRs.
- There is no `cargo-deny`, no `rust-toolchain.toml`, no `SECURITY.md`, no fuzzing and no
  `forbid(unsafe_code)`. CI floats `stable`, which broke clippy on Rust 1.99 (PR #120). None of the
  71 `unsafe` blocks carries a safety comment.
- The declared minimum Rust version is `rust-version = "1.96"`. Spec 004 plans an `msrv` job that
  is not built (task 005). The `cargo +1.96.0 check` in PR #120 was a one-off, not a gate.
- The lockfile holds 104 third-party packages, reached through ten direct dependencies.
  `criterion`, used only by benchmarks, brings 65 of them, and 32 arrive through nothing else.
  21 packages run a build script and 4 are proc macros.

## Threat model

Trust runs from the owner, and the operator files the owner writes, to the host kernel and the
Plasmosome binaries built from this tree with their pinned artifacts. Everything below that is a
less-trusted side. The standard defends against four of them now.

1. **A hostile workload in a cell.** This is code in the guest that is actively trying to reach
   beyond its grants: the guest kernel, the host bridges, other plasmids' files. The cell boundary
   itself is spec 001 §4.2 and spec 017. This standard adds that the host treats every byte from
   the guest as hostile, and that the guest's syscall surface is part of the boundary.
2. **A poisoned agent following instructions faithfully.** The phrase is spec 011's. Its requests
   are well formed, look authorized, and do what injected text told it. It is never asked to
   cooperate and never approves its own widening (spec 011). This standard adds that the
   controller's parsers survive whatever it sends. A controller crash stops supervision of every
   cell at once.
3. **Hostile plasmid code.** A plasmid's manifest, its broker process and the broker's readiness
   replies are written by its author, who may be anyone (intent 010). The kernel parses them under
   stated bounds and trusts nothing in them beyond the declaration.
4. **Supply-chain compromise.** This is a malicious or hijacked crate, crate release, action,
   compiler or CI tool. The dependency, toolchain and advisory rules defend against it:
   - every package is reviewed by name;
   - every version is pinned by the lockfile's checksum;
   - new releases wait seven days;
   - sources are limited to crates.io;
   - actions are pinned by commit, and tools by version.

**Guest-side syscall filtering is in scope.** Whatever denies the workload a kernel object inside
the guest is part of the boundary this standard covers. That includes AF_VSOCK sockets, user
namespaces and io_uring. Reviews of draft spec 024 found two routes that a single-syscall filter
misses on the qualified 6.12 guest kernel:

- io_uring's `IORING_OP_SOCKET` opens a socket without calling `socket(2)`, so a seccomp filter on
  `socket(2)` never sees it.
- A network namespace does not confine AF_VSOCK, so a workload holding such a socket could reach the
  host bridges on ports 4090 and 4091.

Spec 017 already requires that "all alternate syscall/loader paths" meet the same denial. Spec 024
owns the mechanism, and owner item O-7 shapes it. This spec names the surface and does not choose.

**Not covered yet.** These are stated so nobody reads silence as a claim.

- **Mandatory access control inside the guest.** The qualified guest kernel has `CONFIG_SECURITY`
  unset (O-7). Until O-7 is settled, file and exec enforcement inside the guest is not claimed. A
  process in a cell may read whatever the guest's ordinary permissions allow, including files staged
  for another plasmid in the same cell. Network egress is designed to be enforced on the host
  (spec 001 §4.2), and that design is in scope.
- **Credential custody for a foreign harness.** A harness that makes its own TLS connections needs
  the real credential inside the guest, or an intercepting proxy on the host. Neither is designed.
  A credential delivered into a cell is assumed readable by the workload. Detach does not revoke
  what it already read.
- **An orphan after `SIGKILL` of `membraned`** (O-10). The helper and VM keep running. A restarted
  controller notices, and nothing cleans up.
- **The hypervisor and the host OS.** libkrun, Hypervisor.framework and the macOS kernel are
  trusted. They are covered only by version pins and advisories.
- **An agent running on the host as the owner's user**, outside any cell. Plasmosome does not
  confine it, and a same-UID process defeats every host-side check (spec 001 says so for its
  sockets).
- **Developer workstation tools**, such as `gh`, Beads and Python. They are pinned where the repo
  pins them, as `tools/work-state-beads-1.1.2.toml` pins Beads, and are otherwise outside this
  standard.
- **Linux hosts**, deferred with the platform.

## Contract

### 1. Dependency admission

**What counts.** A dependency is every package in every `Cargo.lock` in the tree that is not a
member of that lockfile's workspace. That covers:

- normal, build and dev dependencies;
- everything they bring in;
- lockfiles outside the root workspace, such as a fuzz crate's.

Build and dev dependencies count fully. Their build scripts, proc macros and tests run on every
machine that builds or tests the tree.

**When a dependency may be added.** All of these must hold:

1. It comes from crates.io ([section 4](#4-advisories-licences-sources-and-updates)).
2. Its licence passes the allow-list (section 4).
3. No unignored vulnerability or unsound advisory affects the locked version (section 4).
4. What the workspace uses of it could not be written here in roughly a thousand lines, or an
   exception is approved.
5. It is the narrowest crate, with the fewest features, that does the job. When a smaller crate in
   the same family provides what is used, depend on that one. Default features are off unless the
   record says why they are on.
6. The locked version has been public for at least seven days, unless it fixes an advisory.
7. A review record exists, and the independent reviewer re-derived it.

PR #123 is the model for item 5. Its review found that the membrane's strict JSON reader needs only
serde's deserialization traits, which `serde_core` provides. Switching from `serde`, with its
`derive` feature, to `serde_core` took the membrane's normal dependency tree from 20 crates to 14.
No behavior was lost.

**The thousand-line rule.** The size that matters is what writing the used part here would take,
not the size of the crate. A 40,000-line crate whose two functions we call could be 200 lines of
our own. A 900-line crate we use all of passes.

- The record states the author's estimate and how it was reached.
- Under roughly a thousand lines, the default is to write the code here.
- An exception needs an argument in the record, and an approval. An example argument: our own
  version would need more verification than reuse does.

Whose approval counts is owner question Q2. Until it is answered, only the owner approves an
exception. This interim is the stricter reading. Relaxing it later refuses nothing already
admitted. A looser interim could admit what a hard rule would refuse.

**The review record.** Each direct dependency has one TOML file, `docs/dependencies/<crate>.toml`.
Every field is required.

| Field | What it holds |
| --- | --- |
| `name` | The crate name |
| `used_by` | Each workspace package that names it, with the kind: `normal`, `build` or `dev` |
| `reviewed_version` | The version the numbers below were derived from |
| `reason` | What it does for us, in a sentence or two |
| `write_it_instead` | Estimated lines to write what we use, and how the estimate was reached |
| `own_lines` | Lines of Rust in the crate as published |
| `brings` | Every package name it added to the lockfile when admitted |
| `brought_lines` | Lines of Rust across `brings` |
| `unsafe` | How many `unsafe` blocks, functions and impls it and `brings` hold, and what they do |
| `build_time_code` | Each build script and proc macro in it and in `brings` |
| `native_code` | Any C or other native code it compiles or links, or `none` |
| `maintainers` | The crates.io owners at review time |
| `licence` | The SPDX expression |
| `features` | The features enabled across the workspace, and whether default features are on |
| `exception` | Empty, or the argument and who approved it when `write_it_instead` is under about 1000 |
| `verdict` | `admitted`, `grandfathered` or `replace` |

"Lines of Rust" means non-blank lines in the `.rs` files of the published package. The build
script is counted. The `tests/`, `benches/` and `examples/` directories are not. Two people can
repeat this measure. It is not a quality score.

**Who reviews.** The PR's independent reviewer, under section 2 of the pr-review skill.

- The reviewer re-derives every number and list in the record from the crate's source at the locked
  version, and from crates.io. The review comment says whether each one matches.
- The reviewer reads every build script and proc macro in `build_time_code` in full, and says so.
- The reviewer runs `cargo tree -e normal -p <package>` before and after, and asks whether a
  narrower crate or fewer features would serve.

A review that restates the record without re-deriving it is not a review of the dependency. Nor is
an advisory pass. On PR #123, `cargo audit --deny warnings` found 0 advisories over 120 crates. That
shows nobody has reported a problem, not that anybody read the code.

**Dependencies on main, and dependencies being added now.** The task that lands the guard writes a
record for every direct dependency on main that day, re-deriving every field.

- Today that is ten crates: `serde`, `serde_json`, `uuid`, `toml`, `toml_edit`, `libc`, `cc`,
  `proptest`, `tempfile` and `criterion`.
- It also covers every PR merged before the guard lands, such as PR #123's `sha2` and `serde_core`
  if it merges first. Where such a PR's review already recorded facts, as PR #123's did for the
  `sha2` tree, its licences and its publishers, the task starts from them. It re-derives them and
  does not copy them.
- Each such record has verdict `grandfathered`. A grandfathered crate whose `write_it_instead` comes
  out under about a thousand lines, with no approved exception, gets verdict `replace` instead. The
  guard PR's body then lists it as a follow-up task under this spec. No deadline is set.
- Until the guard lands, a PR that adds a dependency writes the record's facts into its own PR, as
  PR #123 did, so the guard task has something to re-derive.
- A PR still open when the guard lands rebases onto it and carries its own record. So does every
  later PR that adds a dependency. Their verdict is `admitted`.

**Re-review.** A record is re-derived when the locked version of its crate crosses a semver
compatibility boundary, such as `0.8` to `0.9` or `1` to `2`. That is new code the record never
described. A compatible bump needs no re-review. The lockfile's checksum, the cooldown and the
advisory check cover it.

**What the guard refuses.** The guard is `every_dependency_has_a_review_record` in
`crates/plasmosome-guards`. It reads each lockfile and manifest through `cargo metadata`, never by
matching text. It refuses when any of these holds:

- A non-member package name in any lockfile is neither a record's `name` nor listed in some
  record's `brings`.
- A workspace package depends directly on a third-party crate, as a normal, build or dev
  dependency, and no record lists that package and kind under `used_by`.
- The features the workspace manifests enable for a dependency differ from the record's
  `features`, including whether default features are on.
- The locked version of a recorded crate is not semver-compatible with its `reviewed_version`.
- A record lacks a required field.

Its message names the package and the harm: a package nobody reviewed runs its build-time code on
every machine that builds main, including ones that hold publish credentials, and ships inside every
published crate that depends on it. The guard checks that records exist and have the right shape.
It never checks whether a number is right. The numbers are the reviewer's.

**Tools and actions in CI.** Every tool CI installs is pinned to a version and installed with
`--locked`. Every action is pinned to a full commit SHA with its version in a comment, as today. A
floating version lets an upstream release change what CI accepts, with no commit here saying so.
`audit.yml` already gives this reason for pinning `cargo-audit`.

### 2. `unsafe` code

**The register.** The register is the set of packages whose manifest sets `unsafe_code` to `deny`
rather than `forbid`. The manifests are the source of truth. Today two packages need `unsafe`:

- **`plasmosome-membrane`.** It has 68 blocks: the VMM child, readiness probes, broker and exec
  launch, signal handling and its tests. It also has one `unsafe extern` block for
  `darwin_group.c`.
- **`plasmosome-core`.** It has three blocks. One installs the shutdown signal handler in
  `plasmosomed`, and two send signals in its integration tests.

**Everything else forbids it.** Every other package sets `unsafe_code = "forbid"` in its manifest's
`[lints.rust]` table, directly or through `[workspace.lints]`. That covers each target the package
compiles: library, binaries, integration tests, benches and examples. Today that means seven
packages: `plasmosome`, `plasmosome-backend`, `plasmosome-ledger`, `plasmosome-testkit`,
`plasmosome-guards`, `plasmid` and `plasmid-sdk`.

A `#![forbid(unsafe_code)]` line in `lib.rs` alone does not meet this rule. It leaves `tests/`,
`benches/` and binaries unchecked. A new workspace member forbids `unsafe` from its first commit,
unless its PR adds it to the register.

**Inside the register.** A register package denies `unsafe_code` for the whole package. It allows
`unsafe` only on the items or modules that need it, with `#[allow(unsafe_code)]`. That makes every
place `unsafe` lives a line a reviewer can find. Adding `unsafe` to another function changes a
line too. The register uses `deny`, not `forbid`, because an inner `allow` cannot lift `forbid`.

**The safety argument.** Every `unsafe` block and `unsafe impl` in the workspace carries a
`// SAFETY:` comment directly above it. The comment says why the operation is sound at that point:
which invariant holds, and what establishes it. Two clippy lints are set to `deny` in every
package:

- `undocumented_unsafe_blocks`, so a block without a safety comment fails
  `cargo clippy -D warnings`;
- `missing_safety_doc`, so every `unsafe fn` must carry a `# Safety` section in its doc comment.

`multiple_unsafe_ops_per_block` is not required. Splitting existing blocks is churn this standard
does not ask for.

An `unsafe extern` block carries a `///` doc on each declaration, saying what it assumes about the
foreign side. That is a reviewer item, because no lint reads it.

These comments are the one place the code carries an inline `//` comment. Clippy accepts a safety
argument only in that position, so the root `AGENTS.md` needs the amendment below.

**Native code.** C or other native code compiled into a package, such as `darwin_group.c`, is
`unsafe` code. Only register packages compile native code. The `unsafe extern` block that declares
it carries the safety argument for each declaration.

**Gaining `unsafe` later.** A package joins the register through a PR that does all of these:

- changes its lint from `forbid` to `deny`;
- adds the scoped `allow`s and the safety comments;
- updates the crate's `AGENTS.md` to say where `unsafe` lives and why;
- says in its body why no safe route exists: what was tried, and which crate the thousand-line rule
  ruled out or admitted.

Leaving the register is the reverse change, and it is welcome.

### 3. The toolchain

**The pin.** `rust-toolchain.toml` at the root names one exact stable release, with
`components = ["clippy", "rustfmt"]` and `profile = "minimal"`. At the time of writing that is
`channel = "1.99.0"`. It is never `stable` and never `1.99`. Rustup then selects that compiler for
every `cargo` command in the tree, locally and in CI.

**CI installs exactly the pinned toolchain.** No workflow names `stable`. The failure this prevents
is a job that installs one toolchain and runs another. `dtolnay/rust-toolchain` sets only the
default toolchain, with `rustup default`, and rustup ranks a toolchain file above the default. The
action's log still prints the version it installed, not the version that ran.

**The minimum Rust version is a separate promise.** `rust-version = "1.96"` says which compiler a
consumer may build with. The pin says which compiler we develop and lint with. The pin is never
below `rust-version`. Spec 004's `msrv` job proves the minimum. With a toolchain file present, that
job must override the file explicitly, as `cargo +1.96.0 check` or with `RUSTUP_TOOLCHAIN=1.96.0`.
Without the override it silently checks the pinned compiler while its log says 1.96. The amendment
to spec 004 below fixes this.

**Bumping the pin** is its own PR. The PR:

- changes the one line;
- runs the root gate and the locally runnable CI checks on the new compiler;
- fixes what the new compiler finds, in the same PR;
- adds no `allow` to silence a new lint unless it argues for one.

PR #120's rename of `fetch_update` to `try_update` has the right shape. A Rust release that fixes a
security problem in the standard library on a supported platform is adopted by a bump PR without
waiting, because `std` ships inside every binary. Otherwise the pin follows stable at the owner's
pace. Raising `rust-version` is a separate decision, never a side effect of a bump.

**Fuzzing.** If the fuzzer needs a nightly compiler, it gets its own pin: a dated nightly in the
fuzz crate's own `rust-toolchain.toml`. It is bumped the same way.

### 4. Advisories, licences, sources and updates

`deny.toml` at the root configures `cargo-deny`, pinned and installed with `--locked`. It replaces
the separate `cargo-audit` install, because one tool reading the same RustSec database is enough.

| Check | Setting | When it runs | Refuses a merge? |
| --- | --- | --- | --- |
| Sources | crates.io only; other registries and git sources denied | Every PR, in `gates` | Yes |
| Licences | The allow-list below; anything else denied | Every PR, in `gates` | Yes |
| Bans | Wildcard version requirements denied; duplicate versions reported | Every PR, in `gates` | Wildcards only |
| Vulnerability and unsound advisories | Denied unless ignored in `deny.toml` | Every PR that changes a lockfile, in `gates` | Yes |
| Vulnerability and unsound advisories | The same | Weekly schedule and manual dispatch | No: a red run reports |
| Unmaintained, notice and yanked advisories | Warned | Both | No |

**Which advisories block.** A vulnerability or unsound advisory against any package in a lockfile
blocks a PR that changes that lockfile. That holds even when the affected package is not the one the
PR touched, because a PR that changes dependencies is the right place to notice. A PR that changes
no lockfile is never refused for an advisory it did not bring. The weekly run and Dependabot alerts
report those, as `audit.yml` already argues.

An advisory may be ignored only by an entry in `deny.toml` with its ID and a reason. The reason says
why the advisory does not reach us, or what is waiting on a fix. Adding an ignore is a reviewer
item.

**Releases.** Spec 007 is a draft. When it is accepted, its release step runs this advisory check
on every release, whether or not the lockfile changed, because a published version cannot be
withdrawn.

**The licence allow-list** is `MIT`, `Apache-2.0` and `Unicode-3.0`. That is the smallest list that
passes today's lockfile. An expression with `OR` passes when one of its alternatives is on the
list, so today's `MIT OR Apache-2.0 OR LGPL-2.1-or-later` passes on its first two. Adding a licence
is a reviewed one-line change to `deny.toml`. The workspace itself is MIT.

**Dependabot for Cargo.** `.github/dependabot.yml` gains a `cargo` entry with these settings:

- updates run weekly;
- minor and patch updates are grouped into one PR;
- each semver-incompatible bump gets its own PR, because it needs a re-review (section 1);
- a seven-day cooldown means a release younger than a week is not proposed.

Security updates are already on in the repository settings, and the cooldown does not delay them.
Every Dependabot PR passes the same gates as any other. A bump that brings a new package name fails
the dependency guard until someone adds the name to a record.

### 5. Bytes from a less-trusted side

**The rule.** A parser whose input a less-trusted party can write states a bound. The bound is a
maximum size and, where it applies, a maximum nesting depth and a time budget.

- The parser checks the size bound while reading, before it buffers the whole input.
- Past the bound it refuses with an error. It never truncates to a success.
- A parser with a hostile writer also has a fuzz target. Hostile writers are a cell's workload, a
  plasmid author, a broker, and a client of the controller's control socket.
- A new parser lands together with its row in the table below and its bound, in the same PR.

**The inventory today**, verified on main at 379962b:

| Parser | Where | Who writes the bytes | Bound today | Needs |
| --- | --- | --- | --- | --- |
| Controller control line | `plasmosome-core`: `control.rs` `serve_connection`, types in `protocol.rs` | Any control-socket client, including an agent driving the command line | 1 MiB per line (`MAX_LINE_BYTES`), checked while reading; JSON depth 128 (serde_json's default) | Fuzz target |
| Broker readiness reply | `plasmosome-membrane`: `readiness.rs` | A plasmid's broker | 1 MiB frame (`MAX_RESPONSE_BYTES`) within spec 001's readiness budget; JSON depth 128 | Fuzz target |
| Plasmid manifest | `plasmosome-core`: `manifest.rs`, `PlasmidManifest::load` and `parse` | A plasmid author | None on size: `load` reads the whole file. TOML depth 80 (toml_edit's default) | A 1 MiB bound checked before the file is read in full, matching spec 020's limit for a declaration; a fuzz target |
| Membrane control request | `plasmosome-membrane`: `control.rs` | The controller, under spec 001's peer-UID check | 1 MiB (`MAX_REQUEST_BYTES`), checked while reading; JSON depth 128 | Bound only |
| Session log | `plasmosome-core`: `session_log.rs` | The controller's own earlier writes | None: reads the whole file | Bounds per line and per file |
| Ledger journal | `plasmosome-ledger`: `Ledger::open_file` | The controller's own earlier writes | None: reads the whole file | Bounds per record and per file |

Bounds that this table does not fix are stated by the task that adds them. That task records them
here and in the crate's README.

Journals are written by the kernel itself. They get bounds, and property tests over truncated and
corrupted input, rather than fuzzing. The failure they meet is a torn write after a crash, which
spec 008 already says to quarantine rather than repair. The existing `proptest` dependency serves.

**Rows added when the code exists:**

| Parser | Governing spec | Who writes the bytes | Bound | Needs |
| --- | --- | --- | --- | --- |
| Host side of the guest bridges, ports 4090 and 4091 | 001 §4.2 | The guest shim, and the workload if it ever reaches AF_VSOCK | 001 §1's maximum frame size | Fuzz target |
| `cell.exec` output and guest observations | Draft 024 | The workload | Stated by spec 024 | Fuzz target |
| Registry package descriptor and declarations | 020 §2 | Any publisher | Spec 020's limits, such as 1 MiB per descriptor and declaration | Fuzz target |
| Runtime recipe (strict JSON) | 001 §4.2, PR #123 | The operator, after the file's trust checks | Stated by its task | Bound only |
| Recovery journals | 008 | The kernel | Stated by its task | Bounds and property tests |

**The guest's syscall interface is input too.** The workload's syscalls are the input it controls
most directly. The filter that limits them inside the guest is in scope, as the threat model says.
Spec 024 owns its mechanism and its acceptance. This spec does not settle either.

**Fuzz targets.**

- They live in a `fuzz/` crate outside the workspace members. Its own lockfile is covered by the
  dependency guard and its records.
- Each target asserts that the parser does not panic and stays within the fuzzer's memory and time
  limits.
- The control-line target also asserts what `control.rs` promises. A line that fails to parse is
  answered with a well-formed JSON reply, and the connection then answers the next request. A line
  over `MAX_LINE_BYTES` is answered with `-32600`, and then the connection closes.
- Each target keeps its seeds, and every input that ever crashed it, in the tree. A test in the
  owning crate replays them under `cargo test` on the stable pin, so a fixed crash cannot come back
  unnoticed. This replay is an ordinary test, not a guard.
- A scheduled CI job runs each target for ten minutes a week. It reports and never blocks a merge,
  because what a fuzzer finds does not depend on the PR under test. A crash it finds becomes a task
  that carries the input. The fix adds the input to the replayed corpus.

### 6. Reporting a vulnerability

`SECURITY.md` at the root says four things:

- **How to report.** Use GitHub's private vulnerability reporting for this repository, which is
  enabled today, plus whatever additional contact Q1 settles.
- **What is in scope.** The crates and binaries in this repository, the cell boundary they build,
  and their dependency and build supply chain.
- **What is not covered yet.** It links to this spec's [threat model](#threat-model) instead of
  copying the list, so the two cannot drift apart.
- **What a reporter can expect.** An acknowledgement within the time Q1 settles, and a fix or a
  stated decision before public disclosure.

It never names a route nobody receives. If private reporting is ever turned off, `SECURITY.md`
changes in the same act.

### 7. How each rule refuses

| Rule broken | Refused by | Why that is allowed under spec 013 |
| --- | --- | --- |
| A locked package no record names; a direct use, feature set or incompatible version the record does not describe | The guard `every_dependency_has_a_review_record`, in `gates` and the root gate | Its build-time code runs on every machine that builds main, including the owner's, which holds a crates.io publish token. It ships in every published crate that depends on it. Code that ran cannot be un-run, and a published version cannot be unpublished. |
| Record numbers wrong; the thousand-line rule broken without approval; a build script unread; a narrower crate available; a release younger than seven days | Reviewer | Judgement that a guard cannot make |
| `unsafe` in a package outside the register | The compiler, through the package's lint level | Not a guard. The refusal lives in the package's own manifest. A PR that needs `unsafe` changes that line, which puts the decision in front of review. See the spec 013 amendment. |
| An `unsafe` block or `unsafe fn` without its safety argument | Clippy, in `gates` and the root gate | Not a guard: a lint level, as above |
| A false safety argument; a new member without the forbid; a package joining the register | Reviewer | Judgement |
| An unknown source, an unlisted licence or a wildcard version | `cargo-deny` in `gates` | A licence obligation, and a crate from an unvetted source, ship with every published version. Spec 013 puts what ships to a consumer in scope. |
| A known vulnerability in a lockfile the PR changes | `cargo-deny` in `gates` | Main is what the next release publishes and what every agent builds, as above |
| An advisory against an unchanged lockfile; a fuzz crash; an unmaintained crate | Scheduled report | Not the PR's doing, so refusing the PR would refuse work for something it cannot fix |
| CI running a toolchain other than the pin; an `msrv` job without its override; an unpinned tool or action; a parser without its bound or row; a missing fuzz target | Reviewer | Revertible by the next commit, so spec 013 keeps it out of the build |
| A fixed crash returning | The corpus replay test, under `cargo test` | An ordinary regression test, not a guard |
| `SECURITY.md` naming a dead route | Reviewer, in the task that writes it | Judgement |

### 8. Reviewer checklist

This spec governs a PR, whatever task it maps to, when the PR changes any of these:

- a `Cargo.lock`, or a manifest's dependency or `[lints]` tables;
- `unsafe` or native code;
- a parser of bytes from a less-trusted side;
- `rust-toolchain.toml`, `deny.toml` or `.github/dependabot.yml`;
- a workflow's tool or action pin;
- `SECURITY.md`.

Its independent reviewer checks these items:

1. **Records.** Each new or changed record's re-derived fields match. Every build script and proc
   macro was read in full. The thousand-line estimate is plausible. An exception has its approval.
   No narrower crate or smaller feature set would serve. The review comment shows
   `cargo tree -e normal -p <package>` before and after.
2. **Release age.** The locked version is at least seven days old, or fixes an advisory.
3. **`deny.toml`.** A new ignore or licence states its reason.
4. **The register.** A new workspace member forbids `unsafe`. A package joining the register says
   why no safe route exists, and updates its `AGENTS.md`.
5. **Safety arguments.** Each new or changed safety argument is true at that point in the code.
   Each `unsafe extern` declaration is documented.
6. **Parsers.** A new parser of less-trusted bytes has its row and its bound, checked while
   reading. It has a fuzz target when its writer is hostile.
7. **Toolchain.** A bump fixes new lints rather than allowing them, and keeps the pin at or above
   `rust-version`.
8. **CI pins.** Tools are pinned by version and installed with `--locked`. Actions are pinned by
   commit SHA.

## Changes proposed to accepted specs and documents

This PR proposes the exact text below and edits none of these files. Each change lands with the
first task that needs it.

### Spec 013: lint levels and third-party tools

Add after the paragraph that begins "**The guards live in one crate**":

> **Two kinds of build failure are not guards in this sense.** A lint level that a package declares
> in its own manifest, such as `unsafe_code = "forbid"` or a clippy lint set to `deny`, is part of
> that package's code. The change that needs to cross it edits that same line in the same diff. So
> the refusal costs one visible line, and it decides nothing on its own. A supply-chain check run
> by a pinned third-party tool, such as `cargo-deny`, runs as its own CI step rather than in this
> crate. It reads an advisory database, and its tool binary does not belong in this crate. Such a
> check is still held to this spec's bar. It may refuse a merge only for a harm the next commit
> cannot undo, and its configuration states that harm.
> [Spec 026](026-security-standard.md#7-how-each-rule-refuses) argues each one.

Nothing else in spec 013 changes. Its guard inventory counts what the crate held when it landed,
and is not a ceiling. A seventh guard, `every_dependency_has_a_review_record`, must still clear
the bar, and section 7 argues that it does.

### Spec 003: where fuzzing sits

Add after the paragraph that begins "A unit test exercises one crate":

> **Fuzzing belongs to the unit layer, and runs two ways.** A fuzz target feeds arbitrary bytes to
> one parser through its crate's API. Its saved inputs replay as an ordinary test of the owning
> crate, so `cargo test -p <crate>` still answers for them. The open-ended search runs on a
> schedule in CI. It reports rather than gates, because its findings do not depend on the PR under
> test. [Spec 026](026-security-standard.md#5-bytes-from-a-less-trusted-side) says which parsers
> need a target.

### Spec 004: the `msrv` job and the toolchain file

Replace the `msrv` bullet under "Jobs" with:

> - **`msrv`**: installs Rust 1.96.0, then runs one step,
>   `cargo +1.96.0 check --workspace --all-targets`. The explicit `+1.96.0` is required. Once
>   `rust-toolchain.toml` pins the development compiler
>   ([spec 026](026-security-standard.md#3-the-toolchain)), rustup prefers that file over the
>   default toolchain an action sets. A bare `cargo check` would then run the pinned compiler while
>   the action's log still reports 1.96.

Add to its Acceptance:

> - Adding a call to a standard-library API stabilized after 1.96 makes the `msrv` job fail while
>   `gates` passes, shown by mutation in the PR that adds the job.

### Root `AGENTS.md`: the safety comment

In "Style", replace the sentence "No inline `//` comments." with:

> No inline `//` comments, with one exception: a `// SAFETY:` comment directly above an `unsafe`
> block or `unsafe impl`, saying why it is sound there. Clippy accepts it only in that position
> ([spec 026](docs/specs/026-security-standard.md#2-unsafe-code)).

This relaxes an existing rule. It does not add an instruction whose effect needs measuring under
decision 001. A missing comment fails `cargo clippy`, so the build enforces it whether or not an
agent reads the sentence.

### The pr-review skill: which PRs this spec governs

In `.agents/skills/pr-review/SKILL.md` section 2, after "checks every applicable acceptance item in
every governing spec.", add:

> A PR that changes a lockfile, a manifest's dependency or `[lints]` tables, `unsafe` or native
> code, a parser of less-trusted bytes, the toolchain pin, `deny.toml`, Dependabot, a CI tool or
> action pin, or `SECURITY.md` is also governed by
> [spec 026](../../../docs/specs/026-security-standard.md#8-reviewer-checklist), whatever its task
> maps to. Check its reviewer checklist.

This is a rule about who does what, so it lands on its reasoning. Here is the failure it prevents.
A PR that adds a dependency under a task mapped to spec 017 is reviewed against spec 017 only, and
nobody opens the record.

## Open questions for the owner

1. **Q1, the security contact.** Private vulnerability reporting is on. Should `SECURITY.md` add
   another contact, such as an email address? What acknowledgement time should it promise?
2. **Q2, whether the thousand-line rule is hard or advisory.** If it is hard, only the owner
   approves an exception. If it is advisory, the independent reviewer may accept a stated argument.
   Until Q2 is answered, the owner approves. Nothing else in this spec changes with the answer.
3. **Q3, owner item O-7, the guest kernel policy.** The options are:
   - (a) a BPF LSM, through a second kernel configuration deviation;
   - (b) seccomp user-notify, accepted by a spec 017 amendment;
   - (c) a small LSM patch.

   This spec does not choose. Whichever is chosen is in scope:
   - (a) and (c) make a kernel build part of what this standard pins and tracks advisories for.
     Intent 006 names "the kernels it runs on".
   - The reviews of draft 024 show that (b) must also close the io_uring and AF_VSOCK routes.

   Until O-7 is answered, enforcement inside the guest stays under "Not covered yet".
4. **Q4, the dependency in owner item O-11.** If guest TCP uses a userspace stack such as smoltcp,
   that stack is a dependency under section 1. A TCP/IP stack is far over a thousand lines, so the
   size rule does not refuse it. It still needs a record like any other, and it would be among the
   largest additions to what ships. Should its record be written before O-11 is decided, as input
   to the decision, or only if smoltcp is chosen? The nftables alternative adds no crate, but it
   amends spec 001.
5. **Q5, secret scanning push protection.** It is off. A secret pushed to a public repository
   cannot be withdrawn. Should it be turned on? It is an admin setting, not a guard.

## Acceptance

Each line names the broken implementation it catches.

**Dependency admission**

1. A PR that adds a crate no record names fails `gates`, whether the crate is a direct dependency of
   any workspace package or arrives through another crate. The message names the package and the
   build-time-code harm. Catches a guard that reads only direct dependencies.
2. A PR that makes an already-locked crate a direct normal dependency of a workspace package, with
   no record naming that package and kind, fails. An example is `rand`, today locked only through
   `proptest` as a dev dependency. Catches a guard that checks only lockfile names.
3. A PR that enables one more feature of a recorded crate, or turns its default features on,
   without changing the record's `features`, fails. Catches a guard that ignores features.
4. A lockfile bump of a recorded crate across a compatibility boundary without a new
   `reviewed_version` fails. An example is `toml` 0.8 to 0.9. A compatible bump, such as 0.8.23 to
   0.8.24, passes. Catches a guard that ignores versions, and one that refuses every bump.
5. A package added only to the fuzz crate's own lockfile, with no record, fails. Catches a guard
   that reads only the root `Cargo.lock`.
6. The guard's PR mutation-tests each of its five clauses, including a record with a required field
   removed. It adds the violation, observes the failure and reverts the violation, as
   `crates/plasmosome-guards/AGENTS.md` requires. Catches a clause that cannot fail.
7. On the day the guard lands, `docs/dependencies/` holds a record for every direct dependency on
   main, with every field filled, and that PR's review re-derives the numbers. Any record whose
   `write_it_instead` is under about a thousand lines, without an approved exception, has verdict
   `replace`, and the PR body lists its follow-up task. Catches blanket grandfathering with empty or
   copied fields.
8. The independent review of a PR that adds a dependency states the re-derived numbers and says
   that every build script and proc macro was read. A comment that only restates the record does
   not meet the merge condition. Catches a review that trusted the author's record or an advisory
   pass.

**`unsafe` code**

9. Adding an `unsafe` block to a package outside the register fails to compile. The PR shows this
   for its library, a binary, an integration test and a bench. Catches `#![forbid(unsafe_code)]`
   placed only in `lib.rs`, which leaves the other targets open.
10. Adding `#[allow(unsafe_code)]` beside new `unsafe` in a package outside the register still
    fails. Catches `deny` used where `forbid` is required.
11. In a register package, adding `unsafe` to a function with no `#[allow(unsafe_code)]` on it or
    its module fails to compile. Catches a package-wide `allow`.
12. Removing one `// SAFETY:` comment in either register package makes
    `cargo clippy --workspace --all-targets -- -D warnings` fail. Catches the lint left at its
    default level, or denied in only some packages.
13. When the task lands, every `unsafe` block on main carries a safety comment. The root gate on a
    Mac and `gates` on Linux both pass with the lint denied. Catches comments added only to code
    that Linux CI compiles.

**The toolchain**

14. `rust-toolchain.toml` names an exact `x.y.z` release with clippy and rustfmt, and no workflow
    names `stable`. Every CI job's log prints a `rustc` version equal to the pin, from a command
    that ran under the job's own toolchain selection. Catches CI that installs `stable` and passes
    only because rustup happened to choose the file.
15. Adding a call to a standard-library API stabilized after 1.96 makes the `msrv` job fail while
    `gates` passes. Catches spec 004's job as written today. With a toolchain file present, that job
    would check the pinned compiler and log 1.96.

**Advisories, licences, sources and updates**

16. Each of these fails `gates`: a PR that adds a git dependency; a PR that adds a crate whose
    licence expression has no alternative on the allow-list, such as `GPL-3.0-only`; a PR that adds
    a `*` version requirement. Catches `cargo-deny` run in a job that is not required, which is
    where today's `audit` job runs.
17. A PR that changes `Cargo.lock` while a locked package has an unignored vulnerability advisory
    fails `gates`. The same advisory does not fail a PR that leaves every lockfile unchanged.
    Catches advisories checked only on the schedule, and advisories that turn every PR red.
18. `.github/dependabot.yml` has a weekly `cargo` entry with minor and patch updates grouped and a
    seven-day cooldown. The first Dependabot Cargo PR that brings a new package fails the dependency
    guard until its record is written. Catches a Dependabot path that bypasses the records.
19. No workflow installs a tool without a version and `--locked`. No `uses:` names a tag or branch
    instead of a commit SHA. Catches a floating `cargo install cargo-deny`.

**Bytes from a less-trusted side**

20. A plasmid manifest file one byte over 1 MiB is refused with a size error before its contents
    are read in full. A file of exactly 1 MiB is not refused for size. Catches a bound checked after
    `read_to_string`, which allocates whatever the file holds, and a missing bound.
21. The session log and ledger readers refuse a record or file past their stated bounds. A property
    test over truncations and corruptions of valid journals finds no panic, and every refusal is an
    error value. Catches a reader that panics on a torn or oversized record.
22. Fuzz targets exist for the controller control line, the broker readiness reply and the plasmid
    manifest. Each asserts no panic within the fuzzer's memory and time limits. The control-line
    target also asserts that every reply is a well-formed JSON response, and that the connection
    answers the next request after a malformed line. Catches a target that calls the parser and
    discards the result, which passes even when the reply path writes garbage.
23. A crashing input, once fixed, is in the owning crate's replayed corpus, and reverting the fix
    makes `cargo test` fail. Catches a corpus kept only in a CI cache.
24. The scheduled fuzz job runs each target for ten minutes a week and is not a required check.
    Catches a nondeterministic job that blocks unrelated PRs.

**Disclosure**

25. `SECURITY.md` names GitHub private vulnerability reporting and the contact Q1 settles. It links
    this spec's threat model for what is not covered, and states the acknowledgement time Q1
    settles. `gh api repos/teonimesic/plasmosome/private-vulnerability-reporting` returns
    `{"enabled":true}` when it merges. Catches a file that names a route nobody receives.

**The standard as a whole**

26. Every rule in sections 1 to 6 appears in section 7 with something that refuses it. Catches a
    rule that nothing checks.
27. The root gate in `AGENTS.md` is green.

## Out of scope

- **The guest syscall-filter mechanism.** Spec 024 and O-7 own it.
- **Credential custody** for a foreign harness.
- **Signing release binaries, build provenance and reproducible builds.** These belong to spec 007
  once it is accepted.
- **Developer workstation tools** beyond the pins the repo already holds.
- **Per-version audits of every package**, in the manner of `cargo-vet`. This was considered and not
  chosen. Auditing each version of 104 packages is a cost this project cannot pay yet. The named
  records, lockfile checksums, the cooldown and the advisory check cover the route by which a new
  crate arrives. The re-review rule covers new major versions. If a compromised compatible release
  ever reaches the lockfile, that is the evidence to reopen this choice.
- **Linux hosts**, deferred with the platform.
