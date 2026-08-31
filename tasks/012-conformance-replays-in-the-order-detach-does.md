---
id: 012
title: A backend can pass all eight clauses and still leak three capabilities on detach
status: todo
priority: 2
specs: [003]
intents: []
refs:
  [
    crates/plasmosome-testkit/src/conformance.rs,
    crates/plasmosome-testkit/tests/clauses_discriminate.rs,
    crates/plasmosome-ledger/src/lib.rs,
    crates/plasmosome-backend/src/backend.rs,
  ]
done_when: >-
  a backend that only accepts revokes in grant order fails at least one clause;
  the revoke-failure arm and the two assertions added by task 011 each have a
  backend that fails them; and FakeBackend and CompositeBackend still pass
  every clause.
pr:
evidence:
---

## Why

**Demoted to 2 on 2026-08-31.** It was filed at 1 while the conformance suite was the active
work. Nothing in the product has this defect: `FakeBackend` and `CompositeBackend` both revoke by
handle and both pass. It becomes urgent the moment a second real backend is written, because that
is when the suite would certify a leaking one — so it is the first thing to do before any new
backend lands, and not before.


`RevokesOnlyInGrantOrder` — a backend answering `UnknownHandle` for any live handle that is not
the oldest live one — passes all eight clauses and then breaks a real detach. Driven through
`SealedLedger::detach` with three live grants:

```text
detach -> Err("ledger replay failed: unknown handle h3")
universe left = [uds-path `/run/victim/a.uds`, proxy-map `victim.test`, mount `/victim`]
```

Three capabilities still granted after a detach that reported failure. That is the bug class the
first invariant of the root `AGENTS.md` names.

The cause is structural, not a missing assertion. `live_grants_hold_distinct_handles` revokes live
grants in **grant order**; `plasmosome-ledger`'s `replay` walks them in **reverse push order**
(`lib.rs:355`, pinned by `detach_replays_effects_in_reverse_push_order`). **No clause revokes live
grants in the order a detach actually replays them.** Closing this means revoking in reverse, or
in both directions, wherever the suite revokes a set.

Two smaller holes, both introduced or exposed by task 011 and worth closing in the same pass:

- **The revoke-failure arm lost its only witness.** Moving `ALedgerKeyedByClass` onto the new
  between-revokes assertion left `conformance.rs`'s revoke-failure arm with no backend reaching
  it, and left two tests producing a byte-for-byte identical panic. A backend that revokes the
  first live grant honestly and then errors on the second reaches it.
- **Two assertions added by task 011 are reached but never failed.** They have no witness, which
  is the standard this repo holds every other clause to.

Five further backends walk through all eight. They are thinner and are recorded so nobody
re-derives them: `PlantStealsOwnershipOnAnEmptyUniverse`, `SameCapabilityReusesTheHandle`,
`ApplyKeepsTheExistingOwner`, `DeadlineOtherThanFiftyMillisFaults` and
`OnlyKnowsTheSampleCapabilities`. The last two exploit the suite being a fixed fixture — one
hardcoded drain deadline, one fixed capability set — rather than a property test, which is a
different and larger decision than adding a clause. `SameCapabilityReusesTheHandle` may be
underspecified rather than untested: the honest backend also errors on the second revoke of a
duplicate capability, so what should happen there is not settled.

## Plan

## Notes
