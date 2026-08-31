# plasmosome-testkit

Test support shared by the kernel crates: builders, the backend conformance suite, and the
scenarios that cross crate boundaries.

Two things live here that cannot live anywhere else. The first is the **conformance suite** — the
behavioral clauses of the `EnforcementBackend` contract, written once as functions generic over
the trait. Every backend is held to the same functions, unchanged; a backend that fails one is
the thing that is wrong. That is what makes the fake a model of enforcement rather than a hope
about it.

`FakeBackend` passes all five. `CompositeBackend` over three fake leaves passes two and fails
three, because it loses the handle its leaf issued — the failing three are `#[ignore]`d in
`tests/composite_backend_conformance.rs` and task 008 fixes the backend, not the clauses.

The second is the **integration layer**. A unit test exercises one crate. These exercise core,
backend and ledger together through their public APIs, with the outside world replaced only at
the seams — the first of them attaches a plasmid's capabilities, replays its ledger on detach,
and verifies the backend snapshot shows no residue.

## What's inside

| Module | Holds |
| --- | --- |
| `builders` | `PlasmidManifest`, `Grant` sequences, `Effect`s and `DesiredState` — a test states only what it is about |
| `conformance` | Five clauses of the backend contract, each generic over `EnforcementBackend` |
| `tests/` | The cross-crate scenarios, and where end-to-end tests will go once a cell boots |

Nothing here ships: the crate is `publish = false`, and a freeze rule keeps it out of every other
crate's non-dev dependencies.

Tests: `cargo test -p plasmosome-testkit`
