---
id: 012
title: A backend can pass all eight clauses and still leak three capabilities on detach
status: in_progress
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

**Deliverable:** `live_grants_hold_distinct_handles` revokes its live set twice — once in the
reverse push order a detach replays in, once in grant order — so a backend that accepts revokes
only in grant order fails it; and the three assertions in the suite that no backend has ever
failed each gain a witness in `tests/clauses_discriminate.rs`.

**Out of scope:** changing `EnforcementBackend`; changing `FakeBackend` or `CompositeBackend`;
changing `plasmosome-ledger`'s replay; a ninth clause; the five thinner backends `## Why` records
(`PlantStealsOwnershipOnAnEmptyUniverse`, `SameCapabilityReusesTheHandle`, `ApplyKeepsTheExistingOwner`,
`DeadlineOtherThanFiftyMillisFaults`, `OnlyKnowsTheSampleCapabilities`) — the last two need the suite
to stop being a fixed fixture, which is a larger decision than a clause; and any other task.

**Read only these, and do not explore beyond them:** `crates/plasmosome-testkit/src/conformance.rs`,
`crates/plasmosome-testkit/tests/clauses_discriminate.rs`, `crates/plasmosome-ledger/src/lib.rs`
(the `replay` function alone), `crates/plasmosome-backend/src/backend.rs`,
`crates/plasmosome-testkit/README.md`, `crates/plasmosome-testkit/AGENTS.md`, and root `AGENTS.md`.

### The one clause that changes

`live_grants_hold_distinct_handles` is the only clause holding more than one live grant when it
starts revoking, so it is the only place the order can be wrong. Wrap its body in a loop over two
orders, **reverse push order first** — the order `plasmosome-ledger`'s `replay` walks, `for index in
(0..*pending).rev()` — then grant order. Each pass builds its own backend from `make`.

Every message inside the revoke phase names which pass it came from, the way task 011's messages
name the drain policy: `did not revoke through {handle} on the {order} pass`, `revoking {handle} on
the {order} pass left {object} standing`, and `revoking every live grant in {order} must empty the
universe`. The grant phase does not need the order — it is identical in both passes. Keep the
existing substrings `is already holding` and `must withdraw the object its own grant materialized`
intact so the two tests already pinned to them stay pinned.

Reverse-first is not cosmetic. Under it `ALedgerKeyedByClass` revokes the newest session file
honestly and then answers `UnknownHandle` for the older one, which is the revoke-failure arm's
witness `## Why` asks for — and it moves that backend off the assertion it currently shares
byte-for-byte with `RevokeTakesLastOfClass`, which keeps that assertion under the grant-order pass.
Two tests stop printing the same panic.

### Three new defective backends

Add each to `Defect` in `tests/clauses_discriminate.rs` with a `///` line saying what it does, and
one `#[should_panic]` test each.

| Backend | The defect | Fails |
| --- | --- | --- |
| `RevokesOnlyInGrantOrder` | answers `UnknownHandle` for any handle with an older live grant still held | `live_grants_hold_distinct_handles`, on the reverse pass, at its first revoke |
| `ARefusedRevokeRestoresWhatItWithdrew` | a refused revoke puts back every object its earlier revokes withdrew | `revoke_of_a_revoked_handle_is_error`, at `must leave` |
| `ARefusedRevokeClearsTheUniverse` | a refused revoke empties the universe, a fail-safe teardown on a ledger it decides is inconsistent | `revoke_of_a_revoked_handle_is_error`, at `from the live grant holding` |

`RevokesOnlyInGrantOrder` goes in as a guard at the top of `revoke`: refuse when
`self.ledger.keys().any(|live| *live < handle.raw())`. That answers correctly for a handle nobody
holds, so it stays past the other seven clauses.

### Order of work, and the evidence each step owes

1. Commit the three backends and their three tests **before** touching the clause. Run them.
   `RevokesOnlyInGrantOrder` must report `test did not panic as expected` — that is the hole in
   `## Why` reproduced rather than assumed. The other two must pass immediately: their assertions
   already exist and were only ever missing a witness.
2. Then change the clause, and run every one of the three with `-- --exact --nocapture`.
3. Paste the observed panic line for each of the three into `## Notes`, and the line proving step 1
   found no panic. **A clause that passes against a backend that violates it is a finding — stop and
   report.**
4. Add one bullet to `crates/plasmosome-testkit/AGENTS.md` under "Hard rules": a clause that revokes
   a set of live grants walks both orders, because the ledger replays in reverse and grant order
   alone certified a backend that leaks. Add a sentence to `crates/plasmosome-testkit/README.md`
   where it explains how a clause earns its place. The suite still has eight clauses, so the counts
   in both files stay right.

### Tests

| Test | Proves |
| --- | --- |
| `live_grants_hold_distinct_handles_catches_a_backend_that_only_revokes_in_grant_order` | a backend refusing any but the oldest live handle fails the clause on the reverse pass |
| `live_grants_hold_distinct_handles_catches_a_ledger_keyed_by_class` (moves) | the revoke-failure arm has a witness again: an honest first revoke, an error on the second |
| `live_grants_hold_distinct_handles_catches_a_revoke_that_takes_another_object_of_its_class` | the between-revokes assertion still bites, now under the grant-order pass |
| `revoke_of_a_revoked_handle_is_error_catches_a_refused_revoke_that_restores_the_object` | the first assertion task 011 left without a witness fails against one |
| `revoke_of_a_revoked_handle_is_error_catches_a_refused_revoke_that_clears_the_universe` | the second one does too |
| `a_backend_with_no_defect_passes_every_clause`, `snapshot_os_state_is_the_only_oracle_a_clause_has` | the widened clause did not become unpassable |
| `fake_backend_*`, `composite_backend_*` | both shipped backends pass the widened clause |

**If `FakeBackend` or `CompositeBackend` fails the widened clause, that is a real bug — stop and
report. Do not weaken the clause and do not fix the backend here.**

**Definition of done:** the five tests above are green as `#[should_panic]` tests, both shipped
backends still pass all eight clauses, `## Notes` carries the observed output for each new witness,
and the gate in root `AGENTS.md` is green — `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`,
`./.githooks/provenance-guard`, `./.githooks/attribution-guard`. Then `status: in_review`, `pr:`
filled, and a draft PR.

STOP when done. Do not start another task.

## Notes
