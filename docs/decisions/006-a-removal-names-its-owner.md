---
id: 006
title: A removal names the owner whose object it takes
date: 2026-08-31
status: accepted
---

## Context

`OsState` holds `OsObject { class, key, owner }` in a set. `OsState::remove` took two of those
three fields and resolved the third itself: it scanned for whichever object matched the class and
key, read that object's owner, and took it. The caller said what to remove but never said whose.

Two plasmids can hold the same key at once. `UniverseOp::SetProxyMap` drops its `route` when it
forms the key, so two plasmids proxying `api.github.com` down different routes produce two objects
that differ only by owner. Revoking one of them took the other. The set is ordered, so what came
back was the object whose owner sorts first — `audit` before `deploy`, whichever one the caller
meant.

The discarded `route` is what made the collision easy to reach. It is not what causes it. Grant
the same session file path to two plasmids — a key that discards nothing — and the universe holds
two objects there just the same, because nothing in `OsState` limits a key to one holder. The
missing field was on the removal, not on the key.

## Decision

A removal names its owner. `OsState::remove` takes `(class, key, owner)` and takes that owner's
object or none; `EnforcementBackend::apply_removal` takes the owner alongside the removal.

No wire type changes. The owner is already written down beside every removal — `LedgerEntry.plugin`
where a backend revokes a handle, the ledger's own `plugin` where a detach replays an inverse — and
was simply being dropped on the way to `OsState`. Carrying it the rest of the way costs a method
signature, not a serialized field.

## Rejected

**Put the route in the proxy-map key.** It closes the one case that was found and leaves the
mechanism intact: two plasmids holding one session file path, or one host down one route, still
collide, and `remove` still guesses between them. It also costs more than the chosen fix rather
than less — `UniverseRemoval::RemoveProxyMap` carries only `host`, so it could no longer build the
key it must remove, and a wire enum would have to grow a field. Changing what a key contains is a
contract change; changing who a removal names is not.

**Add the owner to `UniverseRemoval`.** Five wire variants change to record a plugin name that the
ledger already stores twice over — once on the `LogRecord`, once on the `LedgerEntry` the handle
resolves to. A removal is an instruction, not a record of who issued it.

**Make `remove` refuse when a key has several holders.** A safety net over the guess rather than a
replacement for it, and it turns a state two plasmids can legitimately reach into an error.

## Consequences

Every `EnforcementBackend` implementation accepts the owner. There are three in the repository and
the change is mechanical in all of them.

`BackendError::UnknownObject` names the owner. Without it the fixed code would answer a revoke of
another plasmid's object with "no such object", which is false — the object is there and is not
theirs. The two read differently now.

`OsState::owner_of` keeps its first-match answer and is now only a diagnostic read: it still names
one owner where several hold a key, and no removal goes through it. `contains` is the same shape —
it answers whether anyone holds a key, not whether you do.

Two things this does not fix, both filed as task 022. One plasmid granted the same capability twice
still materializes one object, so the second detach fails with `UnknownObject` and residue
verification cannot see what the first one left; that is the discarded fields — `route`, `source` —
in their same-owner form. And the rule that a revoke takes its own plugin's object is enforced by
tests over `FakeBackend` rather than by a clause the conformance suite holds every backend to.
