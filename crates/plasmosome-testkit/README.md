# plasmosome-testkit

Test support shared by the kernel crates: builders, the backend conformance suite, and the
scenarios that cross crate boundaries.

Two things live here that cannot live anywhere else. The first is the **conformance suite** — the
behavioral clauses of the `EnforcementBackend` contract, written once as functions generic over
the trait. Every backend is held to the same functions, unchanged; a backend that fails one is
the thing that is wrong. That is what makes the fake a model of enforcement rather than a hope
about it.

`FakeBackend` and `CompositeBackend` over three fake leaves both pass all eight. The composite
failed three of them when it was first wired in, because it lost the handle its leaf issued;
task 008 fixed the backend rather than the clauses, which is the point of holding every backend
to the same functions.

A clause earns its place by being watched failing against a backend built to carry the defect it
names. The suite started at five, and the three added by task 009 came from asking what a broken
backend could still walk through: handles reused between live grants, `apply` and `apply_removal`
never called at all, and a handle revoked twice.

The second is the **integration layer**. A unit test exercises one crate. These exercise core,
backend and ledger together through their public APIs, with the outside world replaced only at
the seams — the first of them attaches a plasmid's capabilities, replays its ledger on detach,
and verifies the backend snapshot shows no residue.

## What's inside

| Module | Holds |
| --- | --- |
| `builders` | `PlasmidManifest`, `Grant` sequences, `Effect`s and `DesiredState` — a test states only what it is about |
| `conformance` | Eight clauses of the backend contract, each generic over `EnforcementBackend` |
| `tests/` | The cross-crate scenarios, and where end-to-end tests will go once a cell boots |

Nothing here ships: the crate is `publish = false`, and a freeze rule keeps it out of every other
crate's non-dev dependencies.

Tests: `cargo test -p plasmosome-testkit`
