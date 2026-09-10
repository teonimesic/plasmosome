---
id: 017
title: Exact capability grants and independently removable objects
status: accepted
intents: [004]
---

## Behavior

Every new capability grant produces one separately removable object, even when the same plasmid
already holds an identical capability. Removing one grant takes that grant alone. Two mount
sources at one target, two routes for one host, and an abandoned broker grant beside a new grant
naming the same PID remain distinguishable before and after either removal.

A grant is not a pathname, route, PID, or count of kernel resources. It is one revocable holding
of a capability. Several holdings may use one underlying resource; withdrawing one must preserve
the others, and withdrawing the last must remove the access they enabled. The verification
universe records these holdings individually, with their full capability descriptions and stable
identities. It does not claim that two identical socket grants create two listeners at one path.

Callers keep the identity returned by a grant, or choose an identity before applying a recorded
universe operation. An inverse carries that identity and the complete capability it withdraws.
Snapshots and residue diffs distinguish exact holdings; a separate canonical comparison ignores
arbitrary identity values but preserves capability, owner, and multiplicity. These are different
questions, and neither comparison may silently substitute for the other.

## Contract

### Grant identity and resource description

`GrantId` is an opaque, copyable 128-bit UUID v4 value, serialized as a canonical lower-case,
hyphenated UUID string. `GrantId::new()` creates a fresh value using the UUID implementation's
random source. It is independent of the owner, resource name, process ID, wall clock, local handle
counter, and order in which composite leaves receive work. Deserialization preserves an identity;
it never calls `new()`. Invalid UUID values, the nil UUID, and missing identities are errors.
An identity is never deliberately recycled. UUID uniqueness is probabilistic, not a distributed
allocator or proof of kernel process identity; a detected collision must not overwrite an object.

A grant's exact address is `(UniverseClass, GrantId)`. Qualifying the address by class permits
class-directed removal without searching another leaf. A `GrantId` minted for a new operation
must be fresh even across classes; this is an issuance requirement, not a requirement to scan
every backend in existence. Reusing a standing address with another payload is a conflict, not replacement.
The address is stable for the lifetime of the holding, including when it becomes residue.

`Capability` retains its five existing variants and every existing field. It gains total ordering
and hashing so the existing ordered universe can use the typed value instead of a lossy string:

| Capability | Complete resource description | Diagnostic key, not an address |
| --- | --- | --- |
| `SessionFile` | `path` | `session/{path}` |
| `UdsSocket` | `path` | `path` |
| `ProxyMap` | `host`, `route` | `host` |
| `Broker` | `pid`, `name` | `broker/{pid}` |
| `Mount` | `source`, `target` | `target` |

Paths, hosts, routes, and names retain their exact input strings. This specification adds no path
normalization, delimiter encoding, hostname folding, mount containment, or route priority rule.
Source and route are never discarded when constructing an object. The typed capability prevents
strings containing separators from producing another capability's address.

`OsObject` becomes `{ id: GrantId, owner: PluginId, capability: Capability }`. Its `class()` and
`key()` derive the table's diagnostic values; there are no redundant mutable `class` or `key`
fields. `describe()` includes the ID, owner, and full capability, including source, route, and
broker name. `OsState` holds at most one object at each exact address. Re-inserting the identical
object is a no-op; inserting a different owner or capability at that address returns
`BackendError::IdentityConflict { class, id }` and preserves the entire state. Deserializing a
state containing a conflicting or repeated address refuses it rather than letting set insertion
silently erase a row. `objects()`, `len()`, and `is_empty()` continue to describe individual objects.

### New grants, recorded operations, and handles

`Grant { plugin, capability, kind }` remains unchanged. Each invocation of
`EnforcementBackend::grant(Grant) -> LedgerEntry` is a **new grant**, including a call with the
same arguments as its predecessor. The backend issues a fresh identity and materializes exactly
one holding. It checks standing addresses before publishing the result; a generated UUID
collision is retried, never used to overwrite a holding. `GrantId::new()` panics if its random
source fails. Each backend, including fake and composite implementations, must acquire the
identity before changing observed state, live grant records, or enforcement. Thus a generation
failure creates no object or handle and leaves existing holdings unchanged. The panic follows
the build's Rust panic strategy: it unwinds, or terminates the process with `panic=abort`.
`grant` remains infallible at its return boundary; callers receive neither a `BackendError` nor a
fallback entry for this failure. No automatic random-source failure retry or recovery is added.

`Handle` changes from a local integer to `{ class: UniverseClass, id: GrantId }`. It is the exact
address of a granted holding, not a separate sequence number. `LedgerEntry` keeps its existing
fields, including `handle`; `entry.object()` derives the exact object and `entry.removal()` its
inverse. No second copy of the ID is added to the entry. `Handle::raw()` and numeric-handle
constructors disappear; handles support equality, ordering, hashing, display, and serde, not
arithmetic. Knowing an address is not authentication: the existing backend seam is trusted.

Every `UniverseOp` variant gains required `id: GrantId`; the other fields remain unchanged.
`op.object()` preserves that ID and all its resource fields. `op.removal()` produces the exact
inverse before execution. `apply(op)` remains `Result<(), BackendError>`:

- An absent address materializes its object once.
- Repeating the same recorded operation while that object stands succeeds without creating a
  second holding. Two intended grants therefore use two IDs, even with equal owner and capability.
- A standing address with a different owner or capability returns `IdentityConflict` without change.

The backend retains standing objects and live grant records, not an unbounded history of retired
IDs. Applying an explicitly supplied address after its object was removed can materialize that
observation again, but does not restore a spent grant handle. Callers must not use a stale apply
as a new grant or replay it after withdrawal without reconciling the current observation. This
seam provides exact selection and live-operation deduplication, not cross-operation retry ordering
or crash recovery. New grants always receive fresh identities.

Grant, direct apply, and plant share the address space, so applying or planting a matching
observation of a live grant is not a second grant and cannot replace its handle record. The
corresponding operation with a changed payload conflicts. `GrantKind` remains entry metadata,
not part of the observed capability, and direct apply does not convert a holding into a grant.

`plant(object)` and `FakeBackend::plant_residue(object)` return `Result<(), BackendError>` and
preserve the supplied identity. Planting is explicit observation/fixture insertion, not a grant
request, handle issuance, PID lookup, or license to make up an ID during recovery. An identical
standing observation is a no-op; a conflicting standing address refuses as above.
A fresh fixture creates an explicit fresh identity for its planted object.

`CompositeBackend` routes handles by their class, without rewriting their IDs or keeping a second
integer-handle namespace. Leaves keep their grant records and drain state; an error names the
original handle. Snapshot union preserves every distinct address, even when leaves previously
would have used the same local counter. Composite construction first validates that each leaf
observes only its assigned classes: network owns proxy maps and UDS paths, filesystem owns files
and mounts, and broker owns broker PIDs. Construction returns
`Result<CompositeBackend, BackendError>` and refuses any out-of-class observation with `Fault`.
After that validation, the leaves' exact-address sets are disjoint: each class has one leaf and
each leaf's `OsState` already excludes duplicate addresses. The union therefore needs neither
cross-leaf coalescing nor constructor-level `IdentityConflict` selection.

### Public API migration

Fallible planting and composite construction are breaking API changes, not optional wrappers.
Change `EnforcementBackend::plant`, its fake/composite and test-backend implementations, and
`FakeBackend::plant_residue` to return the specified `Result<(), BackendError>`. Change
`CompositeBackend::new` and its consumer/conformance factories to handle its `Result`. Callers
must propagate failures or explicitly assert expected fixture success; discarding the result
does not satisfy the cutover. Existing success fixtures and new conflict/invalid-leaf cases must
exercise these outcomes through public APIs: failed planting preserves standing objects, and
invalid composite construction refuses. No old infallible wrapper remains.

### Exact withdrawal

`UniverseRemoval` becomes `{ id: GrantId, capability: Capability }`, replacing the five lossy
removal variants. It contains no owner: `apply_removal(removal, owner)` continues to receive the
owner separately from the ledger or caller. `removal.class()` and `removal.key()` derive diagnostics.
`OsState::remove(&removal, owner)` takes only the row matching the exact address, full capability,
and owner; it returns that object or none. No selector uses an arbitrary matching resource,
a count, or first/last insertion order.

A removal with an absent address, wrong owner, or wrong capability returns
`BackendError::UnknownObject { class, key, owner, id }` and leaves every holding and grant record
unchanged. The diagnostic key alone never decides the outcome. Success removes the object and,
if the holding was granted, removes its live handle record too. Later `revoke` of that handle is
`UnknownHandle`; direct removal is not permission to leave a stale live handle pointing elsewhere.

`revoke(handle, drain)` first resolves that exact live grant. An unknown or spent handle returns
`UnknownHandle`. A graceful drain timeout leaves all objects, the requested handle, and all other
handles unchanged. Force after timeout withdraws only the requested holding. Success returns the
original entry, removes its exact object, and retires the handle. Either revoke order is legal;
source/target or route/host coincidence does not impose an order on this seam. LIFO remains the
ledger's order, not a restriction that a backend imposes on independent grants.

An absent address is never automatically treated as a successful withdrawal. The sealed ledger's
existing pending cursor prevents repeat undo during interruption/resume in that ledger instance.
Reopening a log after a crash has separate recovery obligations; this specification does not
silently turn unknown-object or unknown-handle failures into proof of a completed inverse.

### Holding multiplicity and operating-system equivalence

Two identical capabilities held by one owner are two objects and two independent handles. If an
adapter shares their underlying resource, each holding must still have an independently
withdrawable enforcement attachment. While another legitimate holding remains, the resource and
its remaining access stay available; the last withdrawal removes the access those holdings enabled.
Reference counting a shared resource is permitted. Merely deleting a ledger row while that row's
exclusive access remains enabled is not revocation. Sharing must not widen either owner's grant.

Different mount sources at one target and different routes at one host are different complete
capabilities. Both remain represented and independently removable; this specification does not
pick which stacked mount is visible or which route wins traffic. A real adapter must realize the
holdings without withdrawing another one as a side effect. The in-memory backend models these
holdings; passing its tests is not evidence that a platform's bare mount or routing syscalls
already provide that realization.

A broker PID and name are resource diagnostics, not an incarnation witness. A new grant receives
a new ID even if an abandoned grant has the same owner, PID, and name. Revoking the live handle
preserves the abandoned object; its exact explicit removal can subsequently retire it. This
fixture proves identity separation and residue accounting, not that two live OS processes occupy
one PID simultaneously. Real process observation must bind each holding to the correct process
incarnation and must never signal a reused numeric PID on the strength of this model's UUID.
Process signalling authority and observation of process disappearance are not implemented here.

Exact `OsState` equality and `Diff::between` compare complete identified objects. Losing A and
creating B with the same owner and capability is one lost and one leaked object, not an empty
diff. `ResidueReport` keeps its variants and fields, but each named item carries the new `OsObject`.
Partial withdrawal names the exact survivor; unchanged pre-existing residue is neither lost nor
newly leaked when the baseline already includes it.

`OsState::canonically_equivalent(&other)` is a separate read-only comparison of the multiset of
`(owner, capability)`, ignoring only grant IDs. Equal independent executions may differ in their
UUID values while representing the same holdings. Multiplicity remains significant: two equal
holdings are not equivalent to one. This compares the verification universe, not physical inode,
listener, mount, or process counts. It never authorizes removal and never replaces identity-aware
lifecycle diffs. `contains(class, key)` remains an any-owner diagnostic. Remove the ambiguous
first-match `owner_of` API and migrate its diagnostic/test callers to complete object comparisons;
the existing `objects()` iterator already exposes every owner without selecting an arbitrary one.

### Serialization and ledger interoperability

The clean wire cutover is explicit: `OsObject` replaces `class`/`key` with `id`/`capability`;
`OsState` retains its `objects` collection envelope with validated new objects; `UniverseOp` gains
`id`; `UniverseRemoval` changes to the struct above; `Handle` changes from a number to `class`/`id`.
`Grant` and `Capability` retain their shapes. `LedgerEntry`, `InverseVia`, `Compensation`, `Effect`,
`Diff`, and `ResidueReport` retain their outer fields or variants and embed the new exact types.
No missing identity receives a default. Identity-bearing structures reject unknown fields, so an
old lossy field cannot quietly accompany and disagree with its replacement.

Each typed ledger `LogRecord` gains required `format: 2`. Writers emit that value; deserialization
accepts only that version. Unversioned records, other versions, numeric handles, identity-less
inverses, and malformed identity-bearing records are refused. There is no automatic migration:
old lossy records cannot tell which of several equal holdings was intended. Preserve the old file
and return an error rather than synthesize IDs, skip its effects, or rewrite it in place.

`Ledger::open_file` must therefore stop skipping arbitrary malformed lines. Every newline-ended
record must parse completely with version 2 or the call returns `io::ErrorKind::InvalidData` naming
the line number, without yielding a partially trusted ledger. The existing torn-final-record
allowance is narrow: a final line with no newline may be ignored only when JSON parsing reports
unexpected end of input. Complete JSON with a missing/old version or invalid identity still
refuses, even at EOF. Complete valid final JSON without a newline remains accepted. Mixed owners
continue to refuse; an input with no valid records still refuses. This is compatibility refusal
needed for exact inverse selection, not a journal repair or durable publication protocol.

Writing and reopening a ledger preserves every exact inverse and its owner. Replaying it against
the still-standing backend reaches the same objects. A fresh model backend may be populated with
serialized observed objects, preserving their IDs, and explicit universe inverses must select
those exact objects without guessing. Restoring `InverseVia::Backend` additionally requires the
backend's actual grant records and drain state; a snapshot is not that record and must not issue
handles merely because objects were planted. A fresh backend without those records refuses an
old handle even while equal new capabilities stand.

No claim is made that UUIDs, serialized snapshots, or the existing append/read helpers recover
OS authority, operation ordering, acknowledged inverses, or atomic attach publication after
a crash. Recovery must preserve known IDs, recover real grant ownership, reconcile observations,
and never reinterpret a new grant as the old one. The separate recovery design must integrate
these types rather than retain the retired lossy wire path. This specification leaves
`PluginId`'s current representation intact; decision003's `(cell, plasmid)` ownership can replace
it independently, and must still participate in every owner comparison.

### Backend conformance

Keep existing public clause names and factory signatures from spec003. Add
`revoke_takes_its_owners_object<B: EnforcementBackend>(make: impl Fn() -> B)` and
`repeated_grants_are_independently_removable<B: EnforcementBackend>(make: impl Fn() -> B)`.
They examine complete expected snapshots after each transition, not only handle inequality,
counts, or a final empty state. Both run grant order and reverse push order with fresh factories,
and both graceful and forced withdrawal policies.

The owner clause gives two lexically distinguishable owners the same complete capability in each
class. Each removal must take only its requested owner's exact object, preserving every other
object. The repeated-grants clause gives one owner two identical capabilities in every class,
then covers distinct mount sources, distinct proxy routes, and same-owner/PID/name broker residue.
Every still-live grant remains removable after its peer is withdrawn.

`apply_and_removal_reach_the_universe` plants another owner's object at the same class/key as each
applied operation. It asserts absence of the exact withdrawn object and, after that owner's last
holding at the key is removed, absence of that owner's key membership. The other owner's exact
object must remain. Add repeated same-owner applied objects and exercise both removal orders;
a remaining same-owner holding must not make the selected removal fail its assertion.

The existing defective-backend harness supplies independent witnesses for wrong-owner theft at
the same key, same-owner duplicate collapse, and wrong-instance removal. Both orders need their
own refusal witness for held grants and applied objects. Preserve the existing wrong-key,
handle-reuse, and order-specific witnesses as different faults. Every new clause runs against
FakeBackend, CompositeBackend, the defect-free control, and the existing mirror oracle. A mirror
of intended state may still pass: these clauses observe snapshots, not kernel enforcement.

### Scope and existing contracts

This specifies the precision requested by approved intent004. Spec003 remains the shared test
architecture. Spec001's LIFO, named residue, observation-side authority, and control verbs remain
unchanged; the embedded backend residue object gains the explicit wire shape described above.
Spec011's per-declaration ownership and reachability obligations are not weakened into bookkeeping
or a promise that every retained reference disappears at detach. Decision006's owner requirement
is preserved and extended with exact holding selection; its historical no-wire-change choice
applied to that owner-only fix, not this new identity contract.

No controller verbs, declaration grammar, owner hierarchy, real OS adapter, process supervisor,
recovery coordinator, durable write/ack protocol, clocks, telemetry, or benchmark program is added.
Implementing this contract in the existing backend/ledger/testkit is useful independently of those
systems; it neither claims their guarantees nor waits for another owner decision about precision.

## Acceptance

- **A1 — identical grants:** for each of the five classes, one owner holds two equal capabilities
  as two exact objects with distinct handles; either withdrawal order and policy preserves the
  other object and then removes it without `UnknownObject` on an unrevoked live grant.
- **A2 — resource collisions:** two mount sources at one target and two routes at one host retain
  all input fields, coexist for one owner, and each exact removal preserves the other object.
- **A3 — broker residue:** same owner, PID, and broker name with different IDs remain distinct;
  live revoke preserves the exact planted residue, which its own inverse can then remove.
- **A4 — exact refusal:** wrong owner, wrong capability, missing ID, and spent handle cannot take
  a neighbouring object. Runtime removal failures preserve the full state. Graceful timeout and
  force-after-timeout are exercised with equal peer holdings still standing.
- **A5 — apply identity:** replay of one live recorded op is a no-op; two equal ops with fresh IDs
  create two holdings. Standing payload collision refuses without mutation. Apply or plant after
  exact removal does not revive a spent grant handle; new grants still use fresh identities.
  Cross-entry paths through apply, plant, direct removal, and revoke preserve those transitions.
- **A6 — independent backends:** composite leaves and fresh backend instances do not alias handle
  counters; class routing preserves IDs and errors. Invalid initial leaf observations refuse.
  A spent or prior-backend handle cannot withdraw an equal new grant.
- **A7 — two comparisons:** canonical equivalence ignores IDs but detects changed owner, source,
  route, name, and multiplicity. Exact diff identifies replacement, partial-survivor identity,
  leaked/lost objects, and preservation of baseline residue independently of canonical equality.
- **A8 — serialized inverses:** current log write/read, direct inverse and compensation replay,
  LIFO, and interrupted/resumed detach retain exact selection with equal neighbouring holdings.
  Explicit inverses also select objects after serialized observation reconstruction; that does
  not masquerade as restoration of backend handles or process authority.
- **A9 — refusal at cutover:** round-tripped objects retain identity; duplicate/conflicting state
  rows, missing identities, old wire shapes and unsupported log versions refuse. A bad complete
  middle/final record never disappears; only an incomplete non-newline final JSON fragment may
  be discarded. No refused read changes its source file or yields a partial ledger.
- **A10 — owner conformance:** the public owner clause rejects a same-key wrong-owner thief;
  post-removal assertions ask about the exact object and named owner's remaining holdings, never
  require another owner's key to disappear. Complete survivor snapshots are checked at each step.
- **A11 — discrimination:** duplicate collapse, wrong-instance selection, and each independent
  removal-order defect fail the relevant clauses, while the defect-free backend passes. The
  original multiplicity reproduction fails before the change and passes after it; executed
  mutation witnesses establish each promised failure rather than relying on annotations.
- **A12 — integration:** FakeBackend and CompositeBackend pass all shared clauses; all affected
  constructors, serialized inverses, properties, integration fixtures, and benchmark consumers
  use the new contract with no lossy compatibility path. Root gate is green. Evidence identifies
  model verification separately from any future real operating-system verification.
