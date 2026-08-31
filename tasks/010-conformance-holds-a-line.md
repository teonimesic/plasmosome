---
id: 010
title: Close the six backends that still walk through all eight clauses
status: in_progress
priority: 1
specs: [003]
intents: []
refs:
  [
    crates/plasmosome-testkit/src/conformance.rs,
    crates/plasmosome-testkit/tests/fake_backend_conformance.rs,
    crates/plasmosome-testkit/tests/composite_backend_conformance.rs,
    crates/plasmosome-backend/src/backend.rs,
    crates/plasmosome-backend/src/universe.rs,
    crates/plasmosome-ledger/src/lib.rs,
  ]
done_when: >-
  a committed test asserts each clause panics against a backend carrying its
  defect; and of the six backends named below, the first five no longer pass all
  eight clauses while LedgerMirror is documented as a limit of the seam rather
  than closed.
pr:
evidence:
---

## Why

Task 009 took the suite from five clauses to eight. An independent reviewer then built eight
backends broken in ways the suite should catch, and **six of them passed all eight clauses**. Each
is small and each is already written, in the review on PR #10.

**`ForceIsALie` — forced revoke has no coverage at all.** Every clause uses `DrainSpec::graceful`;
`DrainSpec::forcing()` appears nowhere in `conformance.rs`. A backend that returns `Ok` for a
forced revoke and removes nothing is conformant. That is the failure named in the first invariant
of the root `AGENTS.md`, and `ResidueReport` even has a branch for it with no clause behind it.

**`ClassKeyedLedger` — no clause holds two capabilities of one class live.** `sample_grants` and `universe_pairs` are
one instance per class. A backend keying its ledger by class rather than by handle passes all
eight, while revoking the first handle reports the second grant's capability and strands the
first. Two plugins each holding a session file is the ordinary case for this kernel.

**`HandleRecycler` — a revoked handle may be reissued.** Clause A scopes itself to live grants and clause C grants
nothing in between, so a backend that recycles a freed handle passes — and a partially replayed
ledger then revokes an unrelated live grant instead of erroring. That is the exact scenario clause
C's own documentation invokes.

**`ClassNukingRemoval` — residue is guarded against `revoke` but not against `apply_removal`.** Clause 4 plants residue
and checks an unrelated revoke leaves it alone. Clause B never plants anything, so an
`apply_removal` that clears every object of its class is invisible — and `apply_removal` is the
path replay takes for `InverseVia::Universe` and every compensating effect.

**`ImpostorOwner` — clause B ignores the owner.** It uses `OsState::contains(class, key)`, which drops `owner`,
where clause 5 compares whole objects. A backend applying every operation under an impostor owner
passes. Owner attribution is what the residue report is built on.

**`LedgerMirror` — enforcement is unobservable at this seam.** A backend holding no state at all,
deriving `snapshot_os_state` from its own live ledger, passes all eight. `snapshot_os_state` is the
only oracle any clause has, so no clause can tell a backend that enforces from one that reports its
intent. This one is not fixable inside `EnforcementBackend` as it stands; the deliverable is a
sentence in the testkit README saying so, not a clause.

**The failing-first evidence is not in the repo.** Every clause here was watched failing against a
scratch backend that was then deleted, so the discipline lives in prose and the repo cannot check
it. Committing those backends as a test that asserts each clause panics against its own defect
turns the claim into a guard — and it would have caught the false claim task 009 shipped, that
clause C covered the composite defect, while that clause was still being written.

## Plan

**Deliverable:** the five closable defects below can no longer pass the suite, and a committed
test proves every clause discriminates. Out of scope: changing `EnforcementBackend`, changing
either shipped backend, and the benchmark work in tasks 005 and 006.

**Do the discriminator test first.** Add `crates/plasmosome-testkit/tests/clauses_discriminate.rs`
holding one defective backend per clause and asserting each clause panics against its own defect,
with `#[should_panic(expected = ...)]` matching a distinctive fragment of the message. Write it for
the eight clauses that exist today, before adding anything. It will fail to compile or fail its
assertions where a clause does not discriminate — that is the point, and where it does, stop and
report rather than adjusting the expectation.

This comes first because every clause in this crate so far was watched failing against a scratch
backend that was then deleted, so the discipline lived in prose. Task 009 shipped a false claim
about what clause C covered, and a committed discriminator would have caught it while the clause
was being written.

**Then close five defects.** Prefer extending an existing clause over adding a new one; a suite of
twenty clauses nobody reads is worse than eight that hold.

| Defect | Where it is closed | The clause must now fail against |
| --- | --- | --- |
| `ForceIsALie` | new clause, or extend `drained_revoke_removes_object` | a backend that returns `Ok` for a `DrainSpec::forcing()` revoke and removes nothing |
| `ClassKeyedLedger` | extend `live_grants_hold_distinct_handles` to hold two grants of one class | a backend keying its ledger by capability class rather than handle |
| `HandleRecycler` | extend `revoke_of_a_revoked_handle_is_error` with a grant between the two revokes | a backend that reissues a freed handle number |
| `ClassNukingRemoval` | plant residue at the top of `apply_and_removal_reach_the_universe` | an `apply_removal` that clears every object of the removal's class |
| `ImpostorOwner` | compare `op.object()` rather than `(class, key)` in that same clause | a backend applying every operation under a different owner |

Each new or changed clause goes into `clauses_discriminate.rs` too, so the guard grows with the
suite rather than lagging it.

**`LedgerMirror` is not closed.** Add a paragraph to `crates/plasmosome-testkit/README.md` saying
`snapshot_os_state` is the only oracle any clause has, so the suite cannot tell a backend that
enforces from one that reports its intent, and "conformant" must never be read as "enforcing".

**Definition of done:** the discriminator test is committed and green; the five defects above each
fail at least one clause; `FakeBackend` and `CompositeBackend` still pass every clause; and the
gate in root `AGENTS.md` is green. If either shipped backend fails a new clause, that is a real
bug — stop and report, do not weaken the clause and do not fix the backend here.

STOP when done. Do not start task 005 or 006.

## Notes
