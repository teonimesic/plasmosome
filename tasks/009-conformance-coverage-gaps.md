---
id: 009
title: Close the three gaps a broken backend can walk through the conformance suite
status: todo
priority: 2
specs: [003]
intents: []
refs:
  [
    docs/specs/003-test-architecture.md,
    crates/plasmosome-testkit/src/conformance.rs,
    crates/plasmosome-testkit/tests/fake_backend_conformance.rs,
    crates/plasmosome-testkit/tests/composite_backend_conformance.rs,
    crates/plasmosome-backend/src/backend.rs,
    crates/plasmosome-backend/src/fake.rs,
    crates/plasmosome-ledger/src/lib.rs,
  ]
done_when: >-
  each of the three gaps below has a clause in
  crates/plasmosome-testkit/src/conformance.rs, and each clause is shown failing
  against a backend carrying that defect before it is shown passing against
  FakeBackend.
pr:
evidence:
---

## Why

Task 004's review built backends that pass all five clauses while being broken. Three gaps let
them through.

**Handle uniqueness is not a clause.** A backend that returns the same `Handle(7)` for every
grant passes all five. Clauses 1, 3 and 4 grant one capability and revoke it before granting the
next, so no two live grants ever share a handle inside a clause, and clause 5 never revokes at
all. Two live grants sharing a handle break `plasmosome-ledger`'s replay: the first revoke consumes
the handle, the second gets `UnknownHandle`, and `replay` propagates it with `?` — so detach
aborts and every effect below the failure stays granted and pending, not just the duplicate.

**`apply` and `apply_removal` are never called.** Two of the six trait methods have no clause. A
backend that returns `Unimplemented` from both is certified conformant. `apply_removal` is the
path the ledger takes for every compensating effect and for `InverseVia::Universe`, and `replay`
propagates its error too — so such a backend passes the suite and then fails every detach that
reaches one of those effects, leaving the rest of the ledger unreverted.

**Revoking an already-revoked handle is not a clause.** Clause 2 probes only
`live.handle + 1_000_000` — a handle no grant ever issued. It never probes a handle that was
granted and then killed, which is a different code path in any backend that keeps a table. A
partially replayed ledger does exactly that: it resumes and revokes handles some earlier pass
already withdrew.

## Plan

Written by the planner; blank while the task is `todo`. See `.agents/skills/tasks`.

## Notes

Blank until there is something to add.
