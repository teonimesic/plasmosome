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

Decision 003 reached the same lookups from the other side and set this question down explicitly:
`owner_of`, `contains` and `remove` "key on class and key alone", two plasmids "already collide
under one class and key", and — its own words — "what an owner *is*, and which object a removal
*names*, are separate questions, and this decision answers only the first." This answers the
second. The two compose rather than compete: 003 widens what an owner is, this decides which
object a removal takes, and neither needs the other to land first.

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

`BackendError::UnknownObject` names the plasmid that asked, and its wording no longer asserts the
object is absent: "`network` holds no broker-pid object `broker/1234`" is true whether or not
someone else holds it. The old wording — "no such object in the verification universe" — was a
false statement in exactly the case this change makes reachable. What the error still does not say
is *who* does hold the key. Nothing branches on that today, and saying it would be a new contract
every backend must implement with no clause holding it to the answer, so it was left out.

Spec 017 later replaced the ambiguous `OsState::owner_of` diagnostic with complete identified
objects. `contains` remains an any-owner diagnostic, while exact object comparisons answer which
owner and holding remains. The owner argument chosen here remains separate from the serialized
removal, which now carries a grant identity and complete capability.

Universe inverses and compensation witnesses are replayed on behalf of the ledger's plugin.
During detach, `backend.apply_removal` checks ownership and refuses a foreign-owner object.
A refusal stops replay and leaves the effects below it standing. `Effect::exact` and
`Effect::compensating` record the supplied inverse without checking ledger ownership.
`InverseVia::Backend` instead passes the exact handle to the backend and may name a grant
issued to another plugin.

Spec 017 and task 022 resolved the two precision gaps recorded by the original decision. Equal
grants now materialize as independently removable identified objects, and the shared conformance
suite checks owner-specific and instance-specific withdrawal for every backend.

Finally, the rule this adds to `crates/plasmosome-backend/AGENTS.md` carries no A/B result, and
decision 001 asks for one before a rule lands. The method there scores code an agent writes from a
prompt, which reaches a style or approach heuristic and cannot reach a statement of what this
crate's types guarantee. The four rules already under that heading are the same kind — "a grant
returns a ledger entry", "wire types stay serde-serializable" — and none was measured. This
decision is the evidence for that line; if the reading is wrong, the line goes rather than the
requirement bending.
