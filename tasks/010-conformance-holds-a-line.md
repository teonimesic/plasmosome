---
id: 010
title: Close the six backends that still walk through all eight clauses
status: todo
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
  defect, and the six backends named below no longer pass all eight clauses.
pr:
evidence:
---

## Why

Task 009 took the suite from five clauses to eight. An independent reviewer then built eight
backends broken in ways the suite should catch, and **six of them passed all eight clauses**. Each
is small and each is already written, in the review on PR #10.

**Forced revoke has no coverage at all.** Every clause uses `DrainSpec::graceful`;
`DrainSpec::forcing()` appears nowhere in `conformance.rs`. A backend that returns `Ok` for a
forced revoke and removes nothing is conformant. That is the failure named in the first invariant
of the root `AGENTS.md`, and `ResidueReport` even has a branch for it with no clause behind it.

**No clause holds two capabilities of one class live.** `sample_grants` and `universe_pairs` are
one instance per class. A backend keying its ledger by class rather than by handle passes all
eight, while revoking the first handle reports the second grant's capability and strands the
first. Two plugins each holding a session file is the ordinary case for this kernel.

**A revoked handle may be reissued.** Clause A scopes itself to live grants and clause C grants
nothing in between, so a backend that recycles a freed handle passes — and a partially replayed
ledger then revokes an unrelated live grant instead of erroring. That is the exact scenario clause
C's own documentation invokes.

**Residue is guarded against `revoke` but not against `apply_removal`.** Clause 4 plants residue
and checks an unrelated revoke leaves it alone. Clause B never plants anything, so an
`apply_removal` that clears every object of its class is invisible — and `apply_removal` is the
path replay takes for `InverseVia::Universe` and every compensating effect.

**Clause B ignores the owner.** It uses `OsState::contains(class, key)`, which drops `owner`,
where clause 5 compares whole objects. A backend applying every operation under an impostor owner
passes. Owner attribution is what the residue report is built on.

**The failing-first evidence is not in the repo.** Every clause here was watched failing against a
scratch backend that was then deleted, so the discipline lives in prose and the repo cannot check
it. Committing those backends as a test that asserts each clause panics against its own defect
turns the claim into a guard — and it would have caught the false claim task 009 shipped, that
clause C covered the composite defect, while that clause was still being written.

## Plan

## Notes
