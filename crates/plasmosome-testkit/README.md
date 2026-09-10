# plasmosome-testkit

Test support shared by the kernel crates: builders, the backend conformance suite, and the
scenarios that cross crate boundaries.

Two things live here that cannot live anywhere else. The first is the **conformance suite** — the
behavioral clauses of the `EnforcementBackend` contract, written once as functions generic over
the trait. Every backend is held to the same functions, unchanged; a backend that fails one is
the thing that is wrong. That is what makes the fake a model of enforcement rather than a hope
about it.

`FakeBackend` and `CompositeBackend` over three fake leaves both pass all ten clauses. The
composite failed three of the original clauses when it was first wired in, because it lost the
handle its leaf issued; task 008 fixed the backend rather than the clauses, which is the point of
holding every backend to the same functions.

The exact-grant clauses build expected objects from each request plus the issued identity, never
from a backend's returned owner or capability. Applied-object withdrawal keeps observations from
every other universe class standing. Dedicated defective backends alter each colliding resource
field, substitute an owner, remove cross-class state, and choose the wrong exact instance.

A clause earns its place by being watched failing against a backend built to carry the defect it
names. The suite started at five, and the three added by task 009 came from asking what a broken
backend could still walk through: handles reused between live grants, `apply` and `apply_removal`
never called at all, and a handle revoked twice. That watching is now committed rather than
remembered: the private unit-test module `src/conformance/clauses_discriminate.rs` holds one
defective backend per defect and accepts only failures recorded by the clause's semantic checks.
A clause that stops discriminating fails there. Run those witnesses directly with
`cargo test -p plasmosome-testkit --lib conformance::clauses_discriminate`.

The order a clause revokes in is part of what it proves. A detach replays a ledger in reverse push
order, so a backend that accepts revokes only in grant order was conformant right up until task 012
had `live_grants_hold_distinct_handles` walk both orders — it revokes its live set in reverse
first, then in grant order, and says in every failure which pass it came from.

**Passing the suite is not evidence of enforcement.** Clauses compare requested changes with
the backend's own account of the world. A backend can maintain a consistent in-memory inventory
without performing an operating-system operation; this interface provides no independent OS
oracle. Read "conformant" as "keeps its own books consistently", not "enforcing". Evidence of
enforcement must come from an end-to-end test that drives the real adapter and observes the
real world.

The second is the **integration layer**. A unit test exercises one crate. These exercise core,
backend and ledger together through their public APIs, with the outside world replaced only at
the seams — the first of them attaches a plasmid's capabilities, replays its ledger on detach,
and verifies the backend snapshot shows no residue.

## What's inside

| Module | Holds |
| --- | --- |
| `builders` | `PlasmidManifest`, `Grant` sequences, `Effect`s and `DesiredState` — a test states only what it is about |
| `conformance` | Ten clauses of the backend contract, each generic over `EnforcementBackend` |
| `src/conformance/clauses_discriminate.rs` | Private defective backends shown failing the clause that names each fault |
| `tests/` | The cross-crate scenarios, and where end-to-end tests will go once a cell boots |

Nothing here ships: the crate is `publish = false`, and a guard keeps it out of every other
crate's non-dev dependencies.

`ManifestBuilder::new(id, description)` requires the test's purpose text, and
`.tool(name, description)` adds a named tool with its sentence. The builder constructs trusted
test data directly; it is not a second TOML validator. The description scenario in
`tests/attach_detach_residue.rs` instead parses a real declaration and reads the resulting
descriptions through `ToolRegistry::lookup`, then verifies withdrawal removes them.

Tests: `cargo test -p plasmosome-testkit`
