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
never called at all, and a handle revoked twice. That watching is now committed rather than
remembered: `tests/clauses_discriminate.rs` holds one defective backend per defect and asserts the
clause panics against it, alongside a defect-free backend that passes all eight. A clause that
stops discriminating fails there.

**Passing the suite is not evidence of enforcement.** `snapshot_os_state` is the only oracle any
clause has, and it is the backend's own account of the world. A backend that holds no operating
system state at all — one that answers every snapshot from its live ledger, so the answer is what
it was asked to do rather than what happened — passes all eight clauses;
`a_backend_that_only_mirrors_its_ledger_passes_every_clause` is that backend, passing. Nothing at
this seam can
separate a backend that enforces from one that reports its intent, because the seam never reads
the operating system. Read "conformant" as "keeps its own books consistently", never as
"enforcing"; the evidence for enforcement has to come from an end-to-end test that drives the real
thing and looks at the real world.

The second is the **integration layer**. A unit test exercises one crate. These exercise core,
backend and ledger together through their public APIs, with the outside world replaced only at
the seams — the first of them attaches a plasmid's capabilities, replays its ledger on detach,
and verifies the backend snapshot shows no residue.

## What's inside

| Module | Holds |
| --- | --- |
| `builders` | `PlasmidManifest`, `Grant` sequences, `Effect`s and `DesiredState` — a test states only what it is about |
| `conformance` | Eight clauses of the backend contract, each generic over `EnforcementBackend` |
| `tests/clauses_discriminate.rs` | One defective backend per defect, each shown failing the clause that names it |
| `tests/` | The cross-crate scenarios, and where end-to-end tests will go once a cell boots |

Nothing here ships: the crate is `publish = false`, and a freeze rule keeps it out of every other
crate's non-dev dependencies.

Tests: `cargo test -p plasmosome-testkit`
