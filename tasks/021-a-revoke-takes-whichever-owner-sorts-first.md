---
id: 021
title: A revoke takes whichever owner sorts first, not the one it was for
status: in_review
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
pr: https://github.com/teonimesic/plasmosome/pull/34
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
`SpawnBroker` drops `name`. `AddMount` collides in exactly the way `SetProxyMap` does, since two
sources may be mounted at one target and the OS keeps both.

The first answer written here said `SpawnBroker` was immune, because a pid names one live process
and two brokers never hold it at once. That was wrong, and the independent review found it. The
universe keeps abandoned objects on purpose — the conformance suite's own fixture plants
`broker/31337` owned by `abandoned` — and pids are reused, so a residue object and a live grant sit
on one key with different owners. Run against the unfixed removal, revoking the live broker takes
the abandoned one and leaves the live one standing. No class is immune; the model imposes no
uniqueness on any key.

The discarded fields turned out not to be the cause. Two plasmids granted the *same* session file
path — a key that discards nothing — also produce two objects, because nothing in `OsState` limits
a key to one holder. Measured, not reasoned: a probe granting each capability twice to one plugin
and once to each of two plugins is what settled it.

That probe also found the second half, which is **not fixed here** and is filed as task 022. One
plasmid granting the same capability twice materializes one object for every class, including the
ones that discard nothing. The second revoke then fails with `UnknownObject`, and residue
verification cannot see what the first revoke left behind. It is loud rather than silent, which is
why it was left out of this change rather than folded into it.

The first composite test written for this did not test what its own commit message claimed.
`CompositeBackend::revoke` routes by handle to the leaf's `revoke`, so it never reaches
`CompositeBackend::apply_removal` — the function the change actually modified. The independent
review demonstrated it: the original first-match bug can be reinstated inside that function and
the whole workspace suite stays green. A test now drives `apply_removal` directly.

That gap led to the other half. `apply_removal` tried each leaf in turn and returned the last
error, while `apply` and `plant` both routed by class. The fallback could never fire — objects only
enter a leaf through those two, both class-routed — and it made every removal miss at two leaves
that cannot hold it, then report the third leaf's answer. A leaf reporting a real fault had that
fault overwritten by "not held". It now routes by class like its neighbours, which deletes the
fallback and the wrong error together.

`OsState::owner_of` was deliberately left alone. It keeps its first-match answer, and after this
change no removal goes through it — it is a diagnostic read, used in a conformance failure message
and in tests. A reviewer who reads it as authoritative will be misled; that is worth watching, not
worth widening here.

No conformance clause was added. `crates/plasmosome-testkit/src/conformance.rs` and
`tests/clauses_discriminate.rs` are being rewritten on task 012's branch, and a new clause plus its
discriminating defect would have collided with that work. Filed as part of task 022 instead. The
signature change still touches both files mechanically, which is unavoidable.
