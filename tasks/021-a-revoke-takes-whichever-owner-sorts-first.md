---
id: 021
title: A revoke takes whichever owner sorts first, not the one it was for
status: in_progress
priority: 2
specs: []
intents: []
refs:
  [
    crates/plasmosome-backend/src/universe.rs,
    crates/plasmosome-backend/src/backend.rs,
    crates/plasmosome-backend/src/fake.rs,
    crates/plasmosome-backend/src/composite.rs,
    crates/plasmosome-ledger/src/lib.rs,
    docs/decisions/006-a-removal-names-its-owner.md,
  ]
done_when: >-
  revoking one plasmid's grant takes that plasmid's object and leaves every other holder of the
  same key standing, proven by a test that fails against the unfixed removal; and the removal path
  from a backend revoke and from a ledger replay both carry the owner rather than resolving it.
pr:
evidence:
---

## Why

`OsState::remove(class, key)` did not know whose object it was taking. It scanned for whichever
object matched the class and key, read that one's owner, and took it. Two plasmids can hold the
same key at once — `UniverseOp::SetProxyMap` drops its `route` when it forms the key, so two
plasmids proxying one host down different routes are two objects that differ only by owner.

Revoking one took the other. The set is ordered, so the object taken was the one whose owner sorts
first, regardless of which plasmid the caller was revoking for. That is the failure in both
directions at once: one plasmid loses a capability it still holds, another keeps one that was
meant to be withdrawn.

The owner was known at every call site and thrown away. `FakeBackend::revoke` resolves the handle
to a `LedgerEntry` that carries `plugin`, then builds a `UniverseRemoval` that drops it. A detach
replay knows the ledger's `plugin` and drops it the same way.

## Plan

Written and executed on one branch; see `docs/decisions/006-a-removal-names-its-owner.md` for the
choice between widening the key and naming the owner, and why the key lost.

## Notes

**2026-08-31.** The variant audit asked for by the brief. Three `UniverseOp` variants discard a
field when they form their key: `SetProxyMap` drops `route`, `AddMount` drops `source`,
`SpawnBroker` drops `name`. Only the first two can make two live capabilities collide —
`AddMount` in exactly the way `SetProxyMap` does, since two sources may be mounted at one target
and the OS keeps both. `SpawnBroker` cannot: a pid names one live process, so two brokers never
hold it at the same time, and `KillBroker` keys on the pid to match.

The discarded fields turned out not to be the cause. Two plasmids granted the *same* session file
path — a key that discards nothing — also produce two objects, because nothing in `OsState` limits
a key to one holder. Measured, not reasoned: a probe granting each capability twice to one plugin
and once to each of two plugins is what settled it.

That probe also found the second half, which is **not fixed here** and is filed as task 022. One
plasmid granting the same capability twice materializes one object for every class, including the
ones that discard nothing. The second revoke then fails with `UnknownObject`, and residue
verification cannot see what the first revoke left behind. It is loud rather than silent, which is
why it was left out of this change rather than folded into it.

Four tests hold the fix and all four fail against the unfixed removal: two on `OsState` itself,
one over `FakeBackend`, one over `CompositeBackend`. The composite one is what makes the claim
"both backends in the repository" true rather than assumed — its `apply_removal` tries each leaf
in turn, which is a second place an owner could have been lost.

`OsState::owner_of` was deliberately left alone. It keeps its first-match answer, and after this
change no removal goes through it — it is a diagnostic read, used in a conformance failure message
and in tests. A reviewer who reads it as authoritative will be misled; that is worth watching, not
worth widening here.

No conformance clause was added. `crates/plasmosome-testkit/src/conformance.rs` and
`tests/clauses_discriminate.rs` are being rewritten on task 012's branch, and a new clause plus its
discriminating defect would have collided with that work. Filed as part of task 022 instead. The
signature change still touches both files mechanically, which is unavoidable.
