# plasmosome-testkit

Test support shared by the kernel crates: builders, the backend conformance suite, and the
scenarios that cross crate boundaries.

Two things live here that cannot live anywhere else. The first is the **conformance suite** — the
behavioral clauses of the `EnforcementBackend` contract, written once as functions generic over
the trait. `FakeBackend` passes them today; every real backend passes the same functions
unchanged. That is what makes the fake a model of enforcement rather than a hope about it.

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
