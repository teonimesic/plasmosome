---
id: 011
title: Close the seven backends that still walk through all eight clauses
status: done
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
pr: https://github.com/teonimesic/plasmosome/pull/12
evidence: squash commit 65e47b4 on main; four clauses now run under both drain policies and the seven backends that walked through all eight each fail one
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

**Deliverable:** the seven backends named in `## Why` each fail at least one clause, each is in
`tests/clauses_discriminate.rs`, and the unreached live-grant assertion is reached. Out of scope:
changing `EnforcementBackend`, changing either shipped backend, adding a ninth clause unless the
table below says so, and tasks 005 and 006.

**Add every defective backend to the discriminator before touching a clause**, and run it. All
seven must report `test did not panic as expected`. That is the gap reproduced rather than assumed,
and it is the order task 010 used.

**Prefer widening an existing clause to writing a new one.** Four of the seven are the same defect
under a policy nobody checks, so the work is mostly a loop, not new prose.

| Backend | Close it by | The clause must then fail against |
| --- | --- | --- |
| `ForcedRevokeNukesUniverse` | run `planted_residue_survives_unrelated_revoke` under both policies | a forced revoke that clears another plugin's objects |
| `ForcedRevokeReturnsStranger` | run `grant_is_replayable` under both policies | a forced revoke returning an entry the grant never issued |
| `ForcedRevokeOfUnknownHandleOk` | run `revoke_unknown_handle_is_error` under both policies | a forced revoke reporting success on a handle nobody holds |
| `ForcedRevokeKeepsHandleAlive` | run `revoke_of_a_revoked_handle_is_error` under both policies | a handle that stays revocable after a forced revoke |
| `RevokeTakesLastOfClass` | assert between the revokes in `live_grants_hold_distinct_handles`, not only after | a revoke that withdraws an arbitrary object of the right class |
| `ClassNukeSparingSessionFiles` | plant residue of a class other than `SessionFile` in `apply_and_removal_reach_the_universe`, or plant one per class | an `apply_removal` that spares session files and clears the rest |
| `HandleRecyclerDepthTwo` | free two handles before the recycling grant in `revoke_of_a_revoked_handle_is_error` | a FIFO free list that reuses only at depth two |

**Reach the unreached assertion.** The live-grant assertion for a recycled handle needs a backend
that errors on the spent handle correctly, then hands that number to a live grant — otherwise it
dies at the earlier assertion, which is why `ARevokedHandleIsReissued` never gets there.

**A note on the policy loop.** `drained_revoke_removes_object` already loops over both policies;
copy that shape rather than inventing another. Where a clause loops, make sure the failure message
still says which policy failed, or a red test will not say what broke.

**Definition of done:** all seven fail at least one clause with the defect committed to the
discriminator; the live-grant assertion is reached; `FakeBackend` and `CompositeBackend` still pass
every clause; the gate in root `AGENTS.md` is green. If either shipped backend fails a widened
clause, that is a real bug — stop and report, do not weaken the clause and do not fix the backend
here.

STOP when done. Do not start task 005 or 006.

## Notes
