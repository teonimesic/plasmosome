---
id: 017
title: Exact capability grants and independently removable objects
status: accepted
intents: [004]
---

## Behavior

Every new capability grant produces one separately removable object, even when the same plasmid
already holds an identical capability. Removing one grant takes that grant alone. Two mount
sources at one target, two routes for one host, and two broker holdings with the same launch
description remain distinguishable before and after either removal.
A broker's command and exact inverse are known before it is launched; its later kernel PID is
a diagnostic, never a value the durable operation must predict.

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

The UUID/exact-object model and version-2 single-plugin log are already implemented. The broker
launch description, fallible observation, drain-aware exact removal and version-3 cutover below
are their next contract, not claims that the current code or a real OS adapter implements them.
Spec008 supplies the cell-qualified owner and durable ordering; its live-recovery acceptance
is not replaced by the model conformance here.

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

`Capability` retains five variants, with total ordering and hashing over every field. `Broker`
replaces its predeclared PID with the complete launch description defined below:

| Capability | Complete resource description | Diagnostic key, not an address |
| --- | --- | --- |
| `SessionFile` | `path` | `session/{path}` |
| `UdsSocket` | `path` | `path` |
| `ProxyMap` | `host`, `route` | `host` |
| `Broker` | `name`, `launch: BrokerLaunch` | `broker/{name}` |
| `Mount` | `source`, `target` | `target` |

Paths, hosts, routes, and names retain their exact input strings. This specification adds no path
normalization, delimiter encoding, hostname folding, mount containment, or route priority rule.
Source and route are never discarded when constructing an object. The typed capability prevents
strings containing separators from producing another capability's address.

`OsObject` is `{ id: GrantId, owner: CellOwner, capability: Capability }`, using spec008's
`CellOwner { cell, plugin }`. `PluginId` remains the plasmid/registry identity. `OsObject::class()`
and `key()` derive the table's diagnostic values; there are no redundant mutable `class` or `key`
fields. `describe()` includes the ID, owner, and full capability, including source, route,
broker name and launch. `OsState` holds at most one object at each exact address. Re-inserting
the identical object is a no-op; inserting a different owner or capability at that address returns
`BackendError::IdentityConflict { class, id }` and preserves the entire state. Deserializing a
state containing a conflicting or repeated address refuses it rather than letting set insertion
silently erase a row. `objects()`, `len()`, and `is_empty()` continue to describe individual objects.

### New grants, recorded operations, and handles

`Grant { owner: CellOwner, capability, kind }` and `LedgerEntry` use that same complete owner.
Each invocation of `EnforcementBackend::grant(Grant) -> LedgerEntry` is a **new grant**, including
a call with the same arguments as its predecessor. The backend issues a fresh identity and materializes exactly
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
fields except that `owner: CellOwner` replaces `plugin`; `entry.object()` derives the exact
object and `entry.removal()` its inverse. No second copy of the ID is added to the entry.
`Handle::raw()` and numeric-handle constructors disappear; handles support equality, ordering,
hashing, display, and serde, not
arithmetic. Knowing an address is not authentication: the existing backend seam is trusted.

Every `UniverseOp` carries required `id: GrantId` and `owner: CellOwner`. The broker variant is
`SpawnBroker { id, name, launch: BrokerLaunch, owner }`; the other resource fields are unchanged.
`op.object()` preserves that ID and every capability field. `op.removal()` produces the exact
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
and mounts, and broker owns broker holdings. Construction returns
`Result<CompositeBackend, BackendError>` and propagates a failed leaf observation or refuses any
out-of-class observation with `Fault`.
After that validation, the leaves' exact-address sets are disjoint: each class has one leaf and
each leaf's `OsState` already excludes duplicate addresses. The union therefore needs neither
cross-leaf coalescing nor constructor-level `IdentityConflict` selection.

`snapshot_os_state(&mut self) -> Result<OsState, BackendError>` is fallible for every backend.
A failed or unsupported class is an error, never an empty or partial snapshot. Composite returns
the complete union only after all leaves succeeded on that call; construction-time validation
does not excuse a later failure or out-of-class observation. A successful model snapshot remains
a model snapshot, not independent OS evidence.
The mutable receiver permits the original supervisor authority to retain an observed terminal
state or loss of ownership without a second interior-mutable wrapper. It does not permit replacing
independent observation with reconciliation of requested state into the OS.

### Public API migration

Fallible planting, composite construction and snapshots, cell-qualified ownership, and drain-aware
exact removal are clean API changes, not optional wrappers. Change the trait, fake/composite and
test-backend implementations, custom serde wire structs, consumers and conformance factories
together. `FakeBackend::plant_residue` returns `Result<(), BackendError>`; snapshot consumers
propagate failures or assert expected fixture success. Every exact-removal caller supplies
`CellOwner` and `DrainSpec`. No old infallible snapshot, ownerless removal, PID-bearing broker
constructor or compatibility alias remains. Failed planting preserves standing objects; invalid
composite construction and failed runtime observation refuse through the public APIs.

### Exact withdrawal

`UniverseRemoval` becomes `{ id: GrantId, capability: Capability }`, replacing the five lossy
removal variants. It contains no owner:
`apply_removal(removal, owner: &CellOwner, drain: DrainSpec) -> Result<(), BackendError>` receives
the owner separately from the ledger or caller. `removal.class()` and `removal.key()` derive
diagnostics. A timeout's diagnostic Handle names the exact address; it issues no grant authority.
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

Exact universe removal obeys the same graceful deadline and Force policy as handle revocation.
It resolves the exact owner/capability before draining. A graceful timeout retains the selected
holding, its original authority and all peers; it is not successful removal. Removing one of
several shared-resource holdings drains only its access, not another owner's use. The last
withdrawal also removes the access enabled by the shared resource. Spec008 requires its durable
operator assertion before forced cell cleanup. Neither API's acknowledgement replaces the fresh
enforcement-side absence observation required before that journal's finish.

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

### Broker launch without a predicted PID

The former `SpawnBroker { id, pid, name, owner }` cannot describe a new broker under spec008's
prepare-before-apply rule: `fork` supplies the PID only after creating the child. Spawning first,
using a placeholder/logical PID, predicting the next PID or backfilling the prepared inverse
would leave an unlogged effect or change the durable operation. None is a permitted implementation.

`BrokerLaunch` is the strict serde record `{ command: Vec<String>, control_socket: String }`.
Both fields are required; unknown fields refuse. `command` is nonempty, its first word is an
absolute executable path, and every word is NUL-free and retained exactly. `control_socket` is
an absolute, NUL-free host-private endpoint. Structural decoding does not consult the filesystem;
preflight separately validates the actual executable, private endpoint parent and ability to
stage the operation before preparing. The broker receives the recorded command directly, not
through a shell, PATH search, interpolation or ambient environment expansion. The exec call
receives an explicitly empty environment; this does not claim that a runtime never creates
its own environment entries afterward. Additional launch inputs require an accepted contract,
not inherited credentials. Trusted configuration supplies non-secret launch inputs; structural
validation cannot detect arbitrary secret values embedded in strings. This is not a new plasmid
manifest grammar, credential transport or permission for workload text to execute host commands.

The endpoint requires spec001 §4.1's trusted-directory, socket-mode and kernel peer-UID boundary
for its host-private control transport. It is not a UdsSocket capability granted to the workload. A
different launch description cannot silently take over an occupied endpoint. Before prepare,
the adapter must establish that it can stage a replacement without disturbing standing access
or refuse it. This validation creates no listener or process. Paths and arguments are exact
payload, not a recipe to be guessed from a broker name or reconstructed from a running PID.

With no matching resource, the first holding creates the actual broker only after the complete
operation and inverse are durably prepared. The surviving supervisor reserves its in-memory
association before creation, then binds the original owned child and any newly owned endpoint
to that exact ID, owner and launch. It must retain cleanup authority even if application or
response delivery fails; an incomplete association cannot be published as successful observation.
This holding record is not an invented `LedgerEntry`: direct apply still does not issue an
opaque grant handle. Only actual issued grant records belong in spec001's `grants` reply.

Repeating the same standing operation creates neither another holding nor another child.
Two fresh IDs are different holdings; equal launches may share one original owned broker if
each has independently withdrawable enforcement-side access. Removing one leaves its peer
usable; the last withdrawal removes the enabled access and cleans the original resource.
A desired-state reference count alone is not that enforcement. This does not require two
listeners at one endpoint or two processes at one PID, and it does not restrict the first
holding to adoption of an already running broker.

The runtime PID is diagnostic only and appears in neither broker capability nor inverse.
The surviving original child authority, not a serialized UUID, PID, name or observed row,
permits process operations. Loss of that authority refuses; a new process using the same number
cannot be signalled or adopted. Native process observation must not declare absence from a
cached terminal result or a successful signal alone. Spec018's retained owned-child/group
limits apply to that supervisor; they do not prove arbitrary descendant absence or authorize
recovery after the supervisor itself died. Controller loss after fork but before its reply
uses the surviving association and fresh observation, never another spawn to reconstruct it.

In the model, equal owner/name/launch with different IDs remains distinguishable residue.
Real same-number PID-reuse refusal is a separate authority obligation, not a claim that two
live processes share one PID. Neither the old PID-bearing model nor the revised recipe is
proof that a real observer or enforcing broker adapter already exists.

### Exact and canonical comparison

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
`Grant` and `LedgerEntry` carry `CellOwner`; `Capability::Broker` replaces `pid` with `launch`.
`InverseVia`, `Compensation`, `Effect`, `Diff`, and `ResidueReport` retain their outer fields or
variants and embed the new exact types.
No missing identity receives a default. Identity-bearing structures reject unknown fields, so an
old lossy field cannot quietly accompany and disagree with its replacement.

The single-plugin `LogRecord` now requires `format: 3`. This explicitly replaces the implemented
format2 boundary; it is not a format2 payload with a silently changed broker meaning.
Writers and readers accept only version3. Unversioned/version2 records, other versions, numeric
handles, identity-less inverses and PID-bearing broker payloads refuse. Preserve old files and
return an error rather than synthesize IDs or launch descriptions, skip effects or rewrite bytes.
Even a version2 record that needs no broker recipe is refused at this explicit whole-format cutover.
The separate, not-yet-implemented per-cell `CellJournalRecord` remains spec008's format1.

Before extending a nonempty legacy file, append validates all existing complete LF-terminated
version3 records. A refused or incomplete target is left byte-for-byte unchanged; appending is
not a way to repair a torn suffix or concatenate onto a valid JSON object lacking LF. This
append precondition is stricter than the reader-only EOF allowance below.

`Ledger::open_file` must therefore stop skipping arbitrary malformed lines. Every newline-ended
record must parse completely with version3 or the call returns `io::ErrorKind::InvalidData` naming
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
these types rather than retain the retired lossy wire path. Spec008's `CellOwner` participates
in every ownership comparison. The legacy single-plugin ledger still stores its plugin;
its replay caller supplies the cell explicitly. No cell is inferred from old bytes or defaulted.

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
then covers distinct mount sources, distinct proxy routes, and same-owner/name/launch broker residue.
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

No real OS adapter, process supervisor or recovery coordinator is delivered by this spec change.
The shared model/API and single-plugin format cutover can be implemented independently, but
their passing conformance does not settle the actual file/listener/proxy/mount/broker realization,
workload confinement or complete independent observer that spec008 requires. Its all-five-class
live-recovery acceptance remains in force. The launch description resolves one concrete ordering
contradiction without declaring the other reserved runtime interfaces implemented.

## Acceptance

- **A1 — identical grants:** for each of the five classes, one owner holds two equal capabilities
  as two exact objects with distinct handles; either withdrawal order and policy preserves the
  other object and then removes it without `UnknownObject` on an unrevoked live grant.
- **A2 — resource collisions:** two mount sources at one target and two routes at one host retain
  all input fields, coexist for one owner, and each exact removal preserves the other object.
- **A3 — broker residue:** same owner, broker name and complete launch with different IDs remain
  distinct; live revoke preserves exact planted residue, which its own inverse can then remove.
  Process diagnostics are separate: loss of original child authority cannot authorize a signal
  to a replacement with the same numeric PID.
- **A4 — exact refusal:** wrong owner, wrong capability, missing ID, and spent handle cannot take
  a neighbouring object. Runtime removal failures preserve the full state. Graceful timeout and
  force-after-timeout are exercised with equal peer holdings still standing.
  A deterministic random-source failure witness triggers a panic before mutation and shows no
  new object or live handle and no change to existing holdings.
- **A5 — apply identity:** replay of one live recorded op is a no-op; two equal ops with fresh IDs
  create two holdings. Standing payload collision refuses without mutation. Apply or plant after
  exact removal does not revive a spent grant handle; new grants still use fresh identities.
  Cross-entry paths through apply, plant, direct removal, and revoke preserve those transitions.
  For broker apply, retry after a lost response retains the original owned child rather than
  launching another. A new resource's complete operation and inverse exist before any fork;
  no PID prediction, placeholder, pre-prepare spawn or post-fork backfill can satisfy the witness.
- **A6 — independent backends:** independently minted grant identities do not alias across leaves
  or fresh backend instances; class routing preserves IDs and errors. Invalid initial leaf
  observations and subsequent failed/out-of-class observations refuse without a partial union.
  A spent or prior-backend handle cannot withdraw an equal new grant.
- **A7 — two comparisons:** canonical equivalence ignores IDs but detects changed owner, source,
  route, name, complete broker launch and multiplicity. Exact diff identifies replacement,
  partial-survivor identity, leaked/lost objects, and preservation of baseline residue
  independently of canonical equality.
- **A8 — serialized inverses:** current log write/read, direct inverse and compensation replay,
  LIFO, and interrupted/resumed detach retain exact selection with equal neighbouring holdings.
  Explicit inverses also select objects after serialized observation reconstruction; that does
  not masquerade as restoration of backend handles or process authority.
- **A9 — refusal at cutover:** round-tripped objects retain identity; duplicate/conflicting state
  rows, missing identities, old wire shapes and unsupported log versions refuse. A bad complete
  middle/final record never disappears; only an incomplete non-newline final JSON fragment may
  be discarded. No refused read changes its source file or yields a partial ledger.
  Format2 and PID-bearing broker records cannot acquire a guessed recipe; a malformed or
  incomplete existing append target remains unchanged. Strict broker fields, absolute program
  and endpoint, nonempty command and NUL refusal are exercised independently of OS preflight.
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
  Include fallible-snapshot and ignored-drain-timeout counterexamples. A bounded real-child
  probe can establish prepare-sync-before-fork, surviving authority and lost-response deduplication,
  but it must identify its surrogate policy and cannot stand in for spec008's actual cell,
  five-class enforcement, private-transport or platform acceptance.
