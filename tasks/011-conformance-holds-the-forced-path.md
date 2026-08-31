---
id: 011
title: Close the seven backends that still walk through all eight clauses
status: todo
priority: 1
specs: [003]
intents: []
refs:
  [
    crates/plasmosome-testkit/src/conformance.rs,
    crates/plasmosome-testkit/tests/clauses_discriminate.rs,
    crates/plasmosome-backend/src/backend.rs,
    crates/plasmosome-backend/src/universe.rs,
  ]
done_when: >-
  the seven backends named below each fail at least one clause, each is added to
  tests/clauses_discriminate.rs, the live-grant assertion for a recycled handle is
  reached by a backend that gets past the earlier one, and FakeBackend and
  CompositeBackend still pass every clause.
pr:
evidence:
---

## Why

Task 010 closed five holes and committed the discriminator that proves every clause still bites.
An independent reviewer then wrote seven more backends and every one passed all eight clauses.
Four of them exploit the same thing.

**The forced path is checked for one property.** `DrainSpec::forcing()` appears once in the whole
suite, inside `drained_revoke_removes_object`, which asserts only that the granted object is gone.
Everything else about a forced revoke is unspecified, so four backends pass:
`ForcedRevokeNukesUniverse` wipes another plugin's objects, `ForcedRevokeReturnsStranger` returns
the wrong ledger entry, `ForcedRevokeOfUnknownHandleOk` reports success on a handle nobody holds,
and `ForcedRevokeKeepsHandleAlive` never retires the handle. Each is a defect the suite already
catches under a graceful drain. The cheapest fix is to run the clauses that assert those
properties under both policies rather than to write four new clauses.

**A revoke may remove the wrong object of the right class.** `RevokeTakesLastOfClass` keys its
ledger honestly by handle, then withdraws an arbitrary object of the matching class.
`live_grants_hold_distinct_handles` now holds two session files but asserts only distinct handles,
every revoke succeeding, and an empty universe at the end — so taking them in the wrong order
passes. This is the `ClassKeyedLedger` defect moved from the ledger into the universe, surviving
in the clause extended to catch it. It needs an assertion between the revokes, not just after.

**The class-nuke check covers one class in five.** The residue planted in
`apply_and_removal_reach_the_universe` is a `SessionFile`, so `ClassNukeSparingSessionFiles` —
which clears the whole class for the other four and behaves for session files — passes.

**Handle recycling is caught at depth one only.** The clause frees one handle and grants once.
`HandleRecyclerDepthTwo` uses a FIFO free list that reuses only after two handles are free, which
is an ordinary allocator shape, and passes.

One further gap, now part of `done_when` rather than a note: the assertion for a recycled handle
taking a live grant is never reached. `ARevokedHandleIsReissued` returns `Ok` from the second
revoke and panics at the earlier assertion, so the later one is unexercised. Closing it needs a
backend that errors on the spent handle as it should, and only then hands the recycled number to a
live grant.

## Plan

## Notes
