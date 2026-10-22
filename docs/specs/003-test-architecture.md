---
id: 003
title: Hexagonal test architecture — four layers, one seam rule
status: accepted
intents: [002]
---

## Behavior

Every piece of kernel logic is testable without the outside world, and every contact with the
outside world goes through a seam a test can replace — a trait where the behavior varies, an
injected path or socket where only the resource varies. Tests are organized into four layers —
unit, integration, end-to-end, performance — and each layer is answerable by one command. This
spec builds the first two layers and the seam inventory; it defines the other two and names what
they wait on.

The shape is hexagonal. The core crates (`plasmosome-core`, `plasmosome-ledger`) never touch the
OS directly; enforcement, process spawning, and sockets are reached through traits whose fake
implementations behave like the real thing. The fakes are models, not stubs: a test that passes
against a fake and fails against the real adapter means the fake is wrong and gets fixed —
never the test.

What lands here: a new workspace crate `plasmosome-testkit` holding shared builders, a backend
conformance suite that every `EnforcementBackend` implementation must pass, and the first
cross-crate integration tests. What does not land here: end-to-end tests of a whole cell — there
is no runnable cell yet (no VMM integration, no controller daemon), so that layer is defined
below and stays empty until one exists.

## Design

### The four layers

| Layer | Lives in | Command | Exists after this spec |
| --- | --- | --- | --- |
| Unit | each crate: `#[cfg(test)]` modules and its `tests/` | `cargo test -p <crate>` | yes (already does) |
| Integration | `crates/plasmosome-testkit/tests/` | `cargo test -p plasmosome-testkit` | yes |
| End-to-end | `crates/plasmosome-testkit/tests/e2e_*.rs`, `#[ignore = "needs a booted cell"]` until a cell boots | `cargo test -p plasmosome-testkit -- --ignored` | defined only |
| Performance | `benches/` per crate and in testkit | `cargo bench` | no — specs 005 and 006 |

A unit test exercises one crate through its own API. An integration test exercises two or more
crates together through public APIs only, with the outside world replaced at the seams below.
An end-to-end test drives the real binaries as black boxes over their control sockets, with
nothing replaced. Performance tests are their own specs because they need their own harnesses
and their own honesty rules.

### The seam inventory

Every place the kernel touches the world outside the process, and the trait that guards it:

| Outside world | Seam | Fake |
| --- | --- | --- |
| OS enforcement (grants, revocation, residue) | `EnforcementBackend` (`plasmosome-backend`) | `FakeBackend`, `CompositeBackend` over fake leaves |
| Process spawning (the VMM child) | `Launch` (`plasmosome-membrane::vmm`) | test launchers already in the crate's tests |
| Filesystem (session log, state) | paths injected as arguments | `tempfile::TempDir` |
| Durable appends that can fail (session log) | `LogStore` and `LogFile` (`plasmosome-core::session_log`) | `FaultLogStore` in core's unit tests |
| Directory syncs that can fail (instance root) | the private `SyncDirectory` function that `InstanceRoot` and `CellDir` carry (`plasmosome-core::instance`) | `recording_sync` in core's unit tests |
| Directory reads that can fail (cell discovery, the trusted-directory listing) | the private name iterator that `collect_names` (`plasmosome-core::instance`) and `listed_from` (`plasmosome-core::private_socket`) take | a `Vec` of names and read errors in core's unit tests |
| File writes that can fail (the differential's results file) | the write function that the private `write_with` takes (`plasmosome-testkit::differential::results`); `write_results` passes `File::write_all` | a write that writes 512 bytes and then fails, and one that writes the whole text and then hands back `/dev/null`, whose sync fails; both in the differential's results tests |
| Sockets (readiness probes) | socket path injected as argument | a test-owned socket in a `TempDir` |
| Standard output and error (the command line's envelopes) | writers injected as arguments to `plasmosome_cli::emit` | a `Vec<u8>`; writers that refuse every write, fail only on flush, or fail only their first write; one that records the largest piece; all in plasmosome-cli's tests |
| Time (lifetime bounds, the fence, expiry scheduling) | `Clock` (`plasmosome-backend::clock`), `SystemClock` in `plasmosome-core::clock` | `ManualClock` |
| The command line's deadline (`--deadline-ms`) | `Clock` and `Deadline` (`plasmosome-cli::clock`), passed in `plasmosome_cli::Context`; the binary passes `SystemClock` | a scripted clock, with an offset for each reading and an action at a chosen reading, in plasmosome-cli's tests |
| The command line's environment (`HOME`, `PLASMOSOME_KERNEL`) | `Env` (`plasmosome-cli::env`), passed in `plasmosome_cli::Context`; the binary passes `ProcessEnv` | `Vars`, which holds each variable's value, an `Env` that panics when it is read, and one that reads nothing; in plasmosome-cli's tests |
| A daemon's control socket (the command line's request and reply) | the socket path and the trusted UID, injected as arguments to `plasmosome_cli::handle` and `exchange` | `FakeDaemon` on a private socket in a `TempDir`, socket pairs, and `plasmosome-core`'s daemon loop on a thread, under a short test `HOME` for selection; in plasmosome-cli's and plasmosome's tests |
| A daemon's reply line (the command line's socket read) | the reader injected as an argument to `plasmosome_cli::reply::read_reply` | a `&[u8]`; readers that give a script of chunks and errors, serve a fixed chunk size, or send no newline, stop after a set length and count what was taken; all in plasmosome-cli's tests |
| The pinned guest (`plasmosome-lanes`) | the bundle directory and the e2fsprogs tool directory injected as paths, and the VMM launcher path given to `run_lane` | a `TempDir` bundle, and launcher scripts that hang or write a console |

Both forms in that table are seams. Which one a contact gets is decided by what varies between
the real thing and the test. When the *behavior* varies — enforcement that really grants and
revokes versus one that records what it was asked to do — the seam is a trait with a fake that
models the contract. When only the *resource* varies — the same filesystem calls against a
different directory, the same socket calls against a different path — the seam is the path,
injected as an argument. A trait over a file path would be ceremony: it would add an indirection
without letting a test observe anything a `TempDir` does not already give it.

The rule for new code: the first time a change touches something not in this table — a network
call, a clock read that affects behavior, a new daemon — it adds the seam in the crate that owns
the contact, in whichever of the two forms fits, and the same row here and in the copy in
`crates/plasmosome-testkit/AGENTS.md`. It never calls the OS inline and mocks it in the test.

There is deliberately no mocking-framework dependency. Expectation-style mocks couple tests to
call sequences; a fake that models the contract couples tests to behavior. Hand-built fakes
only.

Spec 028 requires one injected clock for every lifetime decision, with three readings that tests
set independently: the wall clock, a continuous clock with its boot session, and the clock waits
use. That clock is `Clock` in `plasmosome-backend::clock`. `SystemClock` in
`plasmosome-core::clock` is the production clock, for production code to pass once a caller
exists, and tests pass `ManualClock`. Five decisions read elapsed time
outside it, and stay that way until a task moves them. Durations are passed in as arguments.
`VmmChild::wait_terminal` spends a fixed child-termination allowance with no seam. Two serve one
shared readiness budget (spec001 §4): `BrokerSet::status_with` runs the set-level `ProbeBudget`
checks and `readiness::probe_with` the socket-operation checks, and each takes its own injected
clock seam — production supplies `Instant::now` at both, and the 9z0 deterministic boundary tests
inject a fake at each. The fourth is the capture deadline of a paged observation (spec001 §4.1).
`CaptureSlot` in `plasmosome-backend::paging` decides it in `capture`, `page` and `expire`:
`capture` takes an injected clock and reads it before and after collection, and `page` and `expire`
take the current instant. Each server that pages a result passes its clock in and counts no decision
of its own. The fifth is the budget of the runtime handshake (spec001 §4.2), which uses the
readiness probe's `ProbeBudget` type. `runtime::handshake` in `plasmosome-membrane` checks it
before every wait and after every step, and `handshake_with` takes an injected clock and waiter
in the shape of `readiness::probe_with`: production supplies `Instant::now` and the real wait, and
its tests inject a fake clock that moves only when the test moves it. Amended on 2026-10-21: this
read "Four earlier decisions read elapsed time", before the handshake landed.
One test fixture outside the kernel also reads elapsed time: `run_lane` in
`plasmosome-lanes` kills a lane at a timeout passed in. It reads `Instant` itself, not `Clock`,
because it makes no lifetime decision and its test needs no fake clock: a launcher that never exits
times out whatever the clock reads. The command line's deadline is not a lifetime decision either.
`plasmosome-cli` counts it on its own `Clock` in `plasmosome-cli::clock`, which gives the one
monotonic reading a client needs. Its tests pass a scripted clock: an offset for each reading,
and an action at a chosen reading. An offset can jump past the deadline or leave 10 ms of it,
and the action can hold a call until the peer has closed.

### Workspace files belong to the invocation

Repository-reading tests use one checked boundary in `plasmosome-guards`. It supplies the
runtime workspace root to the check as a path and owns the final result, including
[spec013's stale-target refusal](013-what-earns-a-guard.md#workspace-check-validity).
All five guard test files use it. Membrane's readiness-verb test also used this boundary through
a dev-dependency, until the 9z0 readiness repair removed that spec-text comparison as a
wording-pinned test; the dev-dependency went with it, and production readiness never discovered
a checkout or acquired the dependency.
Fixture paths remain explicit: selecting a hook script in the invocation tree does not change
the scratch repository in which that script runs.

The authority is the test process's working directory, captured once before its workspace
reads. [Cargo guarantees](https://doc.rust-lang.org/cargo/commands/cargo-test.html#working-directory-of-tests)
the package root as each unit/integration test's working directory, including when the shell
is elsewhere and selects a manifest with `--manifest-path`. Direct test-binary execution instead
selects the workspace through its actual working directory. Executable location, `target/`,
Git discovery and either form of `CARGO_MANIFEST_DIR` do not select the runtime tree; in
particular, an absent or misleading runtime manifest variable cannot cause a fallback.

From that captured directory, use
[`cargo locate-project --workspace --message-format plain`](https://doc.rust-lang.org/cargo/commands/cargo-locate-project.html)
and canonicalize the returned manifest's parent. This follows Cargo's workspace membership,
including `package.workspace`, rather than guessing an ancestor depth or accepting any file
named `Cargo.toml`. Cargo is a runtime prerequisite: use `CARGO` when supplied, otherwise
`cargo` on `PATH`, as the existing workspace checks do. An unreadable working directory,
failed locator, malformed output or unavailable manifest/root refuses as
`WorkspaceRootUnavailable`, with the attempted directory and underlying cause. It runs no
workspace check and does not claim that unavailable context proves a stale binary.

Resolve once per checked body and pass that same root to its helpers and repository child
commands. Never mutate process-global cwd or retain a root across invocations. The boundary
must execute the real consumer against the selected tree even when spec013 already identifies
a stale target; it must not replace that observation with an early stale-root panic.
Spec013 defines when those observations can support a passing result.

### The testkit crate

`crates/plasmosome-testkit`, a workspace member with `publish = false`. It depends on the kernel
crates; no kernel crate may depend on it outside `dev-dependencies`, and a guard in
`plasmosome-guards` enforces that. Layout:

- `src/builders.rs` — construction helpers for the noisy types: a `PlasmidManifest` builder, a
  predeclared UniverseOp/Effect sequence builder and a DesiredState builder. Under spec017's
  cutover there is no Grant request builder or backend-minted identity: the caller chooses the
  fresh ID, full operation and inverse before preparation and calls fallible grant(op, kind).
- `src/conformance.rs` — the backend conformance suite: public functions generic over
  `EnforcementBackend`, each one behavioral clause of the backend contract (a grant returns a
  replayable entry; revoke of an unknown handle is `UnknownHandle`; a drained revoke removes
  the object from the snapshot; planted residue survives an unrelated revoke; snapshots never
  invent objects). `FakeBackend` passes it now. Every future real backend passes the same
  functions unchanged — that is what makes the fake a model rather than a hope.
  Spec017's API migration updates these shared bodies once, without renaming their clauses or
  changing factory signatures. Fixtures supply caller-owned durable preparation before actual
  resource creation, never grant-then-record or a hidden backend WAL. Exercise failed grants
  without issued entries and complete standing/issued/incomplete snapshots; empty OsState alone
  is not cleanup. The same clauses run against real implementations with all five classes
  covered, not a model-only or success-only substitute. A real factory may compose assigned-class
  adapters; all five participating adapters must be real for real-enforcement evidence.
- `tests/` — the cross-crate scenario still builds/registers a manifest, grants through FakeBackend,
  records effects in the single-plugin Ledger and detaches LIFO across core, backend and ledger.
  At spec017's cutover its fixture first predeclares and durably prepares the complete operation
  and inverse before fallible grant. Recording a returned entry is not that preparation.
  Verify the complete standing/incomplete residue account after cleanup. The fixture's
  caller-owned preparation is not a backend WAL or a replacement for spec008's per-cell
  decision/cleanup/finish/publication protocol. Actual cell recovery remains separate acceptance.

### Real runtime evidence and its limits

Spec001 §4.2 selects the hardware Linux guest, private bridge, artifact/configuration and
host/guest confinement contract. Spec017's five-adapter realization and managed-file boundary
are the concrete obligations behind real conformance, not new fake-only factory entrypoints.
The actual controller-restart scenario remains spec008R1–R12. Core's unit/integration layers
remain VM-independent; only the actual end-to-end path uses the selected runtime.

Report three different kinds of proof separately: model/API behavior; bounded OS mechanisms;
and complete pinned-runtime enforcement. A deny-default Darwin helper that denies new file or
socket access is not proof that inherited FDs are absent or that HVF/libkrun still works under
its profile. Linux chmod/unlink with a surviving descriptor/private mapping disproves an
overbroad revocation claim; copying and executing acquired bytes proves only that data-retention
boundary. Neither experiment proves the managed FUSE/LSM denial rule. Kernel configuration,
a Docker CLI/container or an unavailable securityfs path cannot be reported as effective policy.

The real-runtime witness must identify the exact publisher-supplied kernel/initramfs/image,
VMM/helper libraries, host confinement and guest policy; actual effective hooks, protected
policy lifetime and denied bypasses matter, not names in configuration. Run the meaningful
strict-mmap/direct-exec refusal, private-copy execution, inherited-FD and omitted-confinement
mutants through those actual boundaries. If FUSE access, a privileged trusted policy loader,
signed helper or supported host is unavailable, name the prerequisite and supplier and preserve
the unproved acceptance. A different VM, privilege boundary or cooperative check is evidence
about that different setup, not a passing witness for the specified boundary.

### Conventions, written where agents look

The layer table and the seam rule go into `crates/plasmosome-testkit/AGENTS.md`. Each existing
crate's `AGENTS.md` Testing section stays authoritative for that crate; the testkit's covers
only what spans crates.

## Contract

- Crate `plasmosome-testkit`, `publish = false`, modules `builders` and `conformance`, both
  public.
- Conformance functions take a factory, not an instance:
  `pub fn grant_is_replayable<B: EnforcementBackend>(make: impl Fn() -> B)` — each function
  owns its backend and can be called with any implementation.
- Conformance function names state the clause they prove; renaming one is a contract change to
  every backend that cites it.
- No kernel crate lists `plasmosome-testkit` outside `[dev-dependencies]`.

## Acceptance

- `crates/plasmosome-testkit` exists, is a workspace member, `publish = false`.
- The conformance suite has at least five clauses, each generic over `EnforcementBackend`, and
  `FakeBackend` passes all of them. At spec017's cutover, this gate also requires its
  [Backend conformance](017-exact-capability-grants.md#backend-conformance) contract and
  [A12 integration acceptance](017-exact-capability-grants.md#acceptance), including the same
  shared clauses across all five real adapter classes; model-only evidence cannot satisfy that
  real-enforcement requirement.
- At least one integration test in `crates/plasmosome-testkit/tests/` exercises core + backend +
  ledger together through public APIs and asserts an empty residue after replay. At spec017's
  cutover, it exercises the caller-owned preparation and fallible outcomes required by those
  same canonical clauses and verifies their complete standing/issued/incomplete cleanup account,
  not merely an empty `OsState`.
  Real-enforcement evidence additionally meets spec017A13/A14 and spec001 §4.2 on the supported
  Darwin and Linux platforms; the mechanism/runtime distinction above is part of that gate.
  Unavailable strict managed-mapping or host confinement proof is recorded as unproved, not an
  ignored/skipped success or a reason to narrow spec008's actual recovery acceptance.
- A guard in `plasmosome-guards` fails the build if any kernel crate depends on
  `plasmosome-testkit` outside dev-dependencies, and the guard is mutation-tested: the violation
  was added, seen to fail, and reverted.
- `crates/plasmosome-testkit/AGENTS.md` carries the layer table and the seam rule.
- The copied-checkout regression exercises the actual publication guard through the shared
  boundary, with the original checkout still present.
  Binaries compiled only in the original must observe the copy-only publication
  violation, not merely return a different root string or fail before the read. Unset and
  misleading runtime `CARGO_MANIFEST_DIR` values do not change the observations.
  The copy, move and outside-workspace outcomes, including stale-target failure rather than
  missing-original-file errors, meet [spec013](013-what-earns-a-guard.md#workspace-check-validity).
  Corrected on 2026-10-09: this item named a second consumer, a membrane test that PR #116
  removed, as [Workspace files belong to the invocation](#workspace-files-belong-to-the-invocation)
  records.
- The gate in the root `AGENTS.md` is green.
