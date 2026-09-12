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
| End-to-end | `crates/plasmosome-testkit/tests/e2e_*.rs`, `#[ignore]` until a cell boots | `cargo test -p plasmosome-testkit -- --ignored` | defined only |
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
| Sockets (readiness probes) | socket path injected as argument | a test-owned socket in a `TempDir` |

Both forms in that table are seams. Which one a contact gets is decided by what varies between
the real thing and the test. When the *behavior* varies — enforcement that really grants and
revokes versus one that records what it was asked to do — the seam is a trait with a fake that
models the contract. When only the *resource* varies — the same filesystem calls against a
different directory, the same socket calls against a different path — the seam is the path,
injected as an argument. A trait over a file path would be ceremony: it would add an indirection
without letting a test observe anything a `TempDir` does not already give it.

The rule for new code: the first time a change touches something not in this table — a network
call, a clock read that affects behavior, a new daemon — it adds the seam in the crate that owns
the contact, in whichever of the two forms fits, and a row here. It never calls the OS inline and
mocks it in the test.

There is deliberately no mocking-framework dependency. Expectation-style mocks couple tests to
call sequences; a fake that models the contract couples tests to behavior. Hand-built fakes
only.

There is no clock seam today. Durations are passed in as arguments and nothing reads wall time
to make a decision. Open question, left open on purpose: if a flaky time-dependent test ever
appears, that is the moment to add a clock trait — not before.

### Workspace files belong to the invocation

Repository-reading tests use one checked boundary in `plasmosome-guards`. It supplies the
runtime workspace root to the check as a path and owns the final result, including
[spec013's stale-target refusal](013-what-earns-a-guard.md#workspace-check-validity).
All four guard test files use it; membrane's readiness-verb test uses the same boundary through
a dev-dependency. Production readiness does not discover a checkout or acquire this dependency.
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
- The copied-checkout regression exercises the actual publication guard and membrane
  readiness-verb test through the shared boundary, with the original checkout still present.
  Binaries compiled only in the original must observe copy-only publication and spec-verb
  violations, not merely return a different root string or fail before either read. Unset and
  misleading runtime `CARGO_MANIFEST_DIR` values do not change the observations.
  The copy, move and outside-workspace outcomes, including stale-target failure rather than
  missing-original-file errors, meet [spec013](013-what-earns-a-guard.md#workspace-check-validity).
- The gate in the root `AGENTS.md` is green.
