---
id: 003
title: Hexagonal test architecture — four layers, one seam rule
status: draft
intents: [002]
---

## Behavior

Every piece of kernel logic is testable without the outside world, and every contact with the
outside world sits behind a trait that a test can replace. Tests are organized into four layers —
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

The rule for new code: the first time a change touches something not in this table — a network
call, a clock read that affects behavior, a new daemon — it adds a trait in the crate that owns
the contact, a fake that models it, and a row here. It never calls the OS inline and mocks it
in the test.

There is deliberately no mocking-framework dependency. Expectation-style mocks couple tests to
call sequences; a fake that models the contract couples tests to behavior. Hand-built fakes
only.

There is no clock seam today. Durations are passed in as arguments and nothing reads wall time
to make a decision. Open question, left open on purpose: if a flaky time-dependent test ever
appears, that is the moment to add a clock trait — not before.

### The testkit crate

`crates/plasmosome-testkit`, a workspace member with `publish = false`. It depends on the kernel
crates; no kernel crate may depend on it outside `dev-dependencies`, and a freeze-checks rule
enforces that. Layout:

- `src/builders.rs` — construction helpers for the noisy types: a `PlasmidManifest` builder, a
  `Grant`/`Effect` sequence builder, a `DesiredState` builder. Tests state only what they are
  about; the builder supplies the rest.
- `src/conformance.rs` — the backend conformance suite: public functions generic over
  `EnforcementBackend`, each one behavioral clause of the backend contract (a grant returns a
  replayable entry; revoke of an unknown handle is `UnknownHandle`; a drained revoke removes
  the object from the snapshot; planted residue survives an unrelated revoke; snapshots never
  invent objects). `FakeBackend` passes it now. Every future real backend passes the same
  functions unchanged — that is what makes the fake a model rather than a hope.
- `tests/` — the cross-crate scenarios. The first one: build a manifest, register it, grant its
  capabilities through `FakeBackend`, record effects in a `Ledger`, detach, replay LIFO, and
  verify the backend snapshot shows no residue. That path crosses core, backend, and ledger
  and is the kernel's whole reason to exist.

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
  `FakeBackend` passes all of them.
- At least one integration test in `crates/plasmosome-testkit/tests/` exercises core + backend +
  ledger together through public APIs and asserts an empty residue after replay.
- A freeze-checks rule fails the build if any kernel crate depends on `plasmosome-testkit`
  outside dev-dependencies, and the rule is mutation-tested: the violation was added, seen to
  fail, and reverted.
- `crates/plasmosome-testkit/AGENTS.md` carries the layer table and the seam rule.
- The gate is green: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo fmt --all -- --check`, `./.githooks/provenance-guard`.
