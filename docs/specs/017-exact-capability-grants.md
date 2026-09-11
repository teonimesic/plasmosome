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

Callers choose a fresh identity and complete operation before either grant or direct apply.
An inverse carries that identity and the complete capability it withdraws.
Snapshots and residue diffs distinguish exact holdings; a separate canonical comparison ignores
arbitrary identity values but preserves capability, owner, and multiplicity. These are different
questions, and neither comparison may silently substitute for the other.

The UUID/exact-object model and version-2 single-plugin log are already implemented. The broker
launch description, caller-prepared fallible grants, complete fallible observation, exact
incomplete-effect cleanup and version-3 cutover below are their next contract, not claims that
the current code or a real OS adapter implements them.
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

`Capability` retains five variants, with total ordering and hashing over every field. Each
contains the resolved nonsecret recipe needed to materialize and withdraw its exact holding:

| Capability | Complete resource description | Diagnostic key, not an address |
| --- | --- | --- |
| `SessionFile` | `path`, `recipe: SessionFileRecipe` | `session/{path}` |
| `UdsSocket` | `path`, `recipe: UdsRecipe` | `path` |
| `ProxyMap` | `host`, `route`, `recipe: ProxyRecipe` | `host` |
| `Broker` | `name`, `launch: BrokerLaunch` | `broker/{name}` |
| `Mount` | `source`, `target`, `recipe: MountRecipe` | `target` |

Paths, hosts, routes, and names retain their exact input strings for equality, ordering, hashing
and diagnostics. The realization rules below constrain access and choose visibility; they never
normalize a recorded capability into another one. Source, route and recipe fields are never
discarded. The typed capability prevents strings containing separators from producing another
capability's address.

`OsObject` is `{ id: GrantId, owner: CellOwner, capability: Capability }`, using spec008's
`CellOwner { cell, plugin }`. `PluginId` remains the plasmid/registry identity. `OsObject::class()`
and `key()` derive the table's diagnostic values; there are no redundant mutable `class` or `key`
fields. `describe()` includes the ID, owner, and full capability, including source, route,
broker name and launch. `OsState` holds at most one object at each exact address. Re-inserting
the identical object is a no-op; inserting a different owner or capability at that address returns
`BackendError::IdentityConflict { class, id }` and preserves the entire state. Deserializing a
state containing a conflicting or repeated address refuses it rather than letting set insertion
silently erase a row. `objects()`, `len()`, and `is_empty()` continue to describe individual objects.

### Resolved recipes and their trusted input

The following records are strict serde objects: every named field is required and unknown fields
refuse. The caller resolves trusted operator declarations into them before choosing/preparing the
operation. Recovery never consults a mutable name-to-recipe table or substitutes a digest for the
complete inverse. A `recipe` has the record type selected by its enclosing capability, not another
independently selectable class tag.

| Record | Fields |
| --- | --- |
| `SessionFileRecipe` | `contents: Vec<u8>`, `mode: FileAccess`, `guest_path: String` |
| `UdsRecipe` | `upstream: String`, `guest_path: String` |
| `ProxyRecipe` | `transport: ProxyTransport`, `destination: String`, `port: u16`, `allow_private: bool` |
| `MountRecipe` | `access: FileAccess` |

`FileAccess` is exactly `read_only | read_write`; `ProxyTransport` is exactly `tcp | udp`.
`port` is nonzero. Strings are NUL-free. Host paths (`SessionFile.path`, `UdsSocket.path`,
`Mount.source`, upstream and broker endpoints) and guest paths (`guest_path`, `Mount.target`)
are absolute. Managed guest projections may not replace the immutable system image, trusted
shim/control paths or another unowned entry. File contents are at most 65,536 bytes. Complete
serialized operations/inverses and enclosing messages must fit spec001's existing frame limit;
oversize closure preparation refuses before effects rather than splitting one transaction.
File entries are ordinary byte files, initially mode0600; execution is governed below, not by
an executable mode supplied by workload text.
`contents` is the immutable initial seed in the recipe, not a snapshot that changes after an
authorized write. Observation verifies the original inode/access association and independently
effective permissions; it does not replace the prepared recipe with current file bytes.

`destination` is one ASCII DNS hostname or numeric IP address, with no scheme, path, wildcard,
userinfo or embedded port. A DNS name is resolved only for this authorized destination. Every
resolved address is checked before connecting; loopback, link-local, private, multicast and
unspecified addresses refuse unless the operator explicitly set `allow_private` for that exact
recipe. That opt-in does not permit arbitrary destinations. Host/route remain exact logical
selection names, not implicit permission to run a program, follow a redirect or use ambient DNS.
The operator is responsible for authorizing these nonsecret inputs; the parser cannot determine
whether arbitrary bytes contain a secret. Credential material must not be placed in contents,
arguments or recipes. Existing credential references and core custody remain separate.

The manifest resolver produces these complete capabilities before durable prepare; this is not
a new user-editable manifest grammar. An unavailable declaration, unsupported operation or
unsafe/non-stageable target refuses before prepare. A resource race after preflight remains a
fallible grant/apply result, including the incomplete-effect transition after any actual effect.
The model, all concrete adapters, builders, custom serde and embedded inverses use the same
required fields. There is no missing-recipe default or compatibility lookup.

### New grants, recorded operations, and handles

The public grant boundary is
`grant(&mut self, operation: UniverseOp, kind: GrantKind) -> Result<LedgerEntry, BackendError>`.
The old `Grant` input struct and backend-minted-ID/infallible overload disappear. The same
contract applies to every fake, composite and real backend, in all five classes.

The caller creates `GrantId::new()`, constructs the complete operation and its exact inverse,
validates it, then durably prepares that operation before entering either `grant` or `apply`.
The backend never changes or allocates the supplied ID. Two intended equal holdings use two
fresh IDs; a retry does not choose another. A detected standing/incomplete address collision
refuses before mutation. UUID source failure occurs at the caller's pre-operation allocation,
before preparation or backend invocation; it cannot strand a child created by `grant`.

`Ok(entry)` means that exact holding is independently established and its original issued record
is retained. The entry carries the supplied address, owner, complete capability and kind.
Retry of the same independently standing issued operation and kind returns the original entry
without another holding, child or handle. Changed owner, payload or kind at that address is
`IdentityConflict`. A direct-applied or planted object cannot be promoted into an issued grant
by calling `grant` at its address. A retained receipt without its complete original holding
does not authorize respawn or a successful retry.

Ordinary validation, IO, spawn, access and readiness failures return `Err`, not a fabricated
entry or an undocumented panic. An error after creating any resource follows the explicit
incomplete-effect transition below; only failure with no remaining owned effects may report
ordinary absence. There is no backend-owned WAL, extra durable authority or model-only exemption.
Preparation is the trusted caller's obligation, as for direct apply, not something learned
from the returned entry or authenticated by a UUID. A recovering cell caller using issued grants
follows spec008's prepare/decision/cleanup/finish/publication ordering with that predeclared
operation and universe inverse. GrantKind is receipt metadata, not a replacement recovery log.

`Handle` changes from a local integer to `{ class: UniverseClass, id: GrantId }`. It is the exact
address of a granted holding, not a separate sequence number. `LedgerEntry` keeps its existing
fields except that `owner: CellOwner` replaces `plugin`; `entry.object()` derives the exact
object and `entry.removal()` its inverse. No second copy of the ID is added to the entry.
`Handle::raw()` and numeric-handle constructors disappear; handles support equality, ordering,
hashing, display, and serde, not
arithmetic. Knowing an address is not authentication: the existing backend seam is trusted.

Every `UniverseOp` carries required `id: GrantId` and `owner: CellOwner`. The broker variant is
`SpawnBroker { id, name, launch: BrokerLaunch, owner }`; each other variant additionally carries
the matching required recipe above, retaining its existing path/host/route/source/target fields.
`op.object()` preserves that ID and every capability field. `op.removal()` produces the exact
inverse before execution. `apply(op)` remains `Result<(), BackendError>`:

- An address absent from both complete and incomplete holdings materializes its object once,
  unless a retained issued record requires reconciliation rather than re-creation.
- Repeating the same recorded operation while that object stands succeeds without creating a
  second holding. Two intended grants therefore use two IDs, even with equal owner and capability.
- A standing address with a different owner or capability returns `IdentityConflict` without change.

The backend retains standing objects, incomplete effects and original issued records, not an
unbounded history of retired IDs. Callers must never use a retired ID for a new operation and
must resolve uncertain results from the durable journal and fresh observation before retry.
Applying an explicitly supplied address after its object was removed can materialize an
observation again, but does not restore a spent grant handle; startup must not do that.
This seam provides exact selection and standing-operation deduplication, not exactly-once
ordering across completed withdrawals. New intended grants always use fresh caller-chosen IDs.

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
Class ownership covers standing objects, incomplete operations and issued records alike.
Each collection is validated before union; no duplicate address within one collection, conflicting
record or standing/incomplete overlap is coalesced into success. Assigned classes make valid
leaf domains disjoint, so the composite need not choose between conflicting owners.

`snapshot_os_state(&mut self) -> Result<EnforcementSnapshot, BackendError>` is fallible for every
backend. `EnforcementSnapshot` is the strict serde record
`{ state: OsState, grants: Vec<LedgerEntry>, incomplete: Vec<IncompleteEffect> }`;
`IncompleteEffect` is `{ operation: UniverseOp }`. Fields are required and unknown fields refuse.
`state` reports independently observed complete holdings. `grants` contains only original
issued records whose actual withdrawal authority remains; it is not reconstructed from objects.
An incomplete record identifies an unfinished effect and its original cleanup obligation,
never a successful holding or an assertion that the recipe's process exists.

Standing and incomplete exact addresses are disjoint. Repeated/conflicting addresses in either
collection or across them refuse the whole snapshot. An issued record may remain while its
withdrawal is incomplete, but failed first materialization issues no record. Independent
observation must account for every owned partial resource in its incomplete association;
unknown resource state or lost original authority is an observation error, not an empty list.
A successful snapshot can therefore contain incomplete effects; omitting one cannot turn it
into a successful absence observation.
Issued records must also have unique addresses and agree in full owner/capability with any
matching standing or incomplete row; their metadata never substitutes for present OS access.
An issued record without independently verified original authority cannot be reported as recoverable.

A failed/unsupported class is an error, never an empty or partial snapshot. Composite validates
class partitioning and preserves all three collections on every call, including constructor
validation; one failed/out-of-class leaf refuses the entire result. A model snapshot remains
a model snapshot. The mutable receiver permits retaining observed terminal/lost ownership;
it does not permit replacing independent observation with desired-state reconciliation.

### Public API migration

Caller-prepared fallible grants, complete fallible snapshots, planting/composite results,
cell-qualified ownership and drain-aware exact removal are clean API changes. Migrate trait,
fake/composite/test backends, custom serde, callers, builders, benches and shared conformance
together. Remove `Grant`, backend grant-ID generation and all old infallible/bare-snapshot
wrappers. Grant callers construct the operation before durable prepare and handle `Result`;
snapshot consumers use its complete state/issued/incomplete account, not only `.state` when
deciding completion. Every exact-removal caller supplies CellOwner and DrainSpec. No ownerless,
PID-bearing, guessed-recipe or drain-free compatibility path remains.

### Exact withdrawal

`UniverseRemoval` becomes `{ id: GrantId, capability: Capability }`, replacing the five lossy
removal variants. It contains no owner:
`apply_removal(removal, owner: &CellOwner, drain: DrainSpec) -> Result<(), BackendError>` receives
the owner separately from the ledger or caller. `removal.class()` and `removal.key()` derive
diagnostics. A timeout's diagnostic Handle names the exact address; it issues no grant authority.
`OsState::remove(&removal, owner)` takes only the row matching the exact address, full capability,
and owner; it returns that object or none. No selector uses an arbitrary matching resource,
a count, or first/last insertion order.

A removal absent from both standing and incomplete sets, or with wrong owner/capability, returns
`BackendError::UnknownObject { class, key, owner, id }` and changes no holding, issued record
or incomplete obligation. The diagnostic key alone never selects. Success removes the object and,
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

### Incomplete effects and their terminal transition

Before any resource creation, reserve the exact operation and enough owned bookkeeping to
retain every resource created by that attempt. Before an operation returns, its outcome is
exactly one of: a complete standing holding with original authority; verified absence with no
remaining owned effects; or an `IncompleteEffect` retaining all original cleanup authority.
Fork/bind followed by access/readiness/association failure is incomplete even though no complete
OsObject or issued entry was published. Return
`BackendError::IncompleteEffect { class, id, detail }`; do not hide the reservation or use a
generic error that lets the caller infer absence. Detail is diagnostic, never a selector.

Once failure makes an effect incomplete, matching grant/apply retries return that incomplete
error without continuing creation or spawning another child. A conflicting operation refuses.
Plant cannot overwrite or promote an incomplete address. The only completion transition is
exact withdrawal of the original partial effects; it is not retry-until-application-succeeds.
If a withdrawal fails after partial resource release, expose that same incomplete state and
retain any previously issued original record. Ordinary pre-effect refusals preserve the full
state; post-effect incompleteness is explicit, not a false promise of atomic OS rollback.
`DrainTimedOut` preserves the pre-withdrawal holding: it applies before destructive release.
A timeout after release has already changed resources is IncompleteEffect instead; remaining
authority and peers are preserved, but released resources are not falsely reported restored.
Spec001 §4.2 therefore uses a deadline-limited host-local pause and an observational guest
drain: no guest-gate restoration RPC is required for an ordinary pre-destructive timeout.
A genuine bridge-channel loss is a distinct enforcement failure: affected holdings move to
incomplete with original authority retained and permit only exact withdrawal, never reactivation.

`apply_removal` also accepts an incomplete address when the supplied owner and full removal
match its recorded operation. It drains/releases only resources belonging to that association,
including a newly owned broker child and private endpoint, preserving shared peers and any
endpoint it never owned. A timeout leaves the incomplete marker and original authority.
Other partial-cleanup errors do likewise. Remove the marker and retire any issued record only
after independent observation proves all that association's owned effects absent. A lost
cleanup reply is then recoverable from absence in both sets. Loss of authority or unknown
observation refuses instead of signalling a PID or deleting a pathname supplied by the record.

The controller is the cleanup actor: spec008 durably aborts a failed uncommitted introduction,
then invokes this existing exact-withdraw path, observes both sets, and only then finishes.
A marker with no matching durable cleanup obligation, or belonging to quarantine, is reported
and keeps readiness false, without authorizing automatic cleanup. This defines an executable recovery
transition for a retained partial launch; it does not rely on an unspecified autonomous reaper
or allow Force before its existing durable authorization.

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
capabilities. Both remain represented and independently removable. A real adapter multiplexes
them rather than overwriting a path or relying on bare stacked mounts. New unqualified
lookups/connections choose the lexicographically smallest active full GrantId at that logical
target; an already-open file handle or flow stays bound to its original grant. Withdrawal never
silently retargets it to a surviving peer. Each covered root/rule still has an actual independent
access attachment, not only an entry in a desired-state count.
Spec001 §4.2 defines the exact host-to-guest install/activate/drain/remove transitions and the
ProxyMap DNS/address/TUN-flow binding. Closed staged attachments do not participate in selection.
Activation is per operation after durable prepare, not an atomic visibility change for an
entire transaction; failed later operations are durably aborted and all new bindings withdrawn.
Already acquired bytes or remote side effects are not rolled back by that cleanup.
Effective unqualified selection is host-authoritative through spec001's bounded4091grant.select,
not a stale guest-side active list. It excludes a host-closed/incomplete grant before delayed
guest cleanup finishes, so fresh peer access survives as well as existing peer handles.

### Five actual enforcement adapters

Spec001's selected hardware guest and trusted host/guest boundary carry these accesses. No host
descriptor or uncontrolled host-directory export is passed to the workload. Each adapter keeps
original authority independently of the controller and inventories actual owned resources,
including resources not requested by a journal. A verified independently created stray binding
is an exact extra object. An unbound inode, listener, flow, mount or child is a named observation
fault, never a made-up GrantId or an empty class. Requested configuration is not inventory.

- **SessionFile:** create an exclusive private per-grant inode, retain its original FD and
  identity, and expose it through a host-gated byte service and grant-bound guest filesystem
  inode/handle. Inspect actual inode/link/FD, gate and guest projection. Equal capabilities have
  separate access attachments even if immutable backing bytes are shared. Cleanup closes this
  gate and its original handles, then unlinks only a still-matching owned inode. A substituted
  path is named and left untouched. Removing a pathname while an access path still serves fails.
- **UdsSocket:** create the owned host listener/selector and a guest Unix listener, relaying to
  the explicit operator-owned upstream. Retain each accepted stream pair, tagged with its grant,
  and both original relay ends. Inspect actual listeners, inodes, credentials, stream pairs and
  guest projection. Never pass SCM_RIGHTS through to the workload. An unavailable upstream is a
  real service/materialization failure, not readiness inferred from bind. Removal closes this
  grant's relay streams; the last attachment removes the matching owned listener/projection.
  Unlink alone is not cleanup. This adapter neither owns nor signals the external upstream.
- **ProxyMap:** use a real per-cell host egress relay with independently effective grant rules
  and original TCP/UDP flow sockets, reached through the trusted guest TUN/network bridge and
  fixed data channel. The host derives the cell from that channel and enforces the exact
  destination/transport/port. No NIC, TSI, arbitrary raw host socket request or DNS fallback
  bypasses it. Inspect effective relay rules, actual flow sockets and guest route/TUN state.
  Removing a rule while its established flow still transports data is a failed withdrawal.
  Guest synthetic addresses use198.18.0.0/15 only inside this isolated bridge; refuse an
  operator-route overlap rather than silently altering inputs or the reserved10.29.0.0/24
  subject range. Network revocation cannot undo a remote side effect already issued.
  Its complete host/route/recipe and original per-boot address reach the shim through the
  closed install schema; data requests alone cannot create or activate a rule. Inspect a rule
  even when it has no flows, and preserve its exact grant binding through DNS caching and
  equal-host selector changes, as specified by spec001 §4.2.
- **Broker:** use the original owned child and answered private control readiness described
  below, plus a separately declared data endpoint and independently withdrawable access relay
  for each grant. Inspect actual process authority, answered readiness and access resources.
  The first equal holding may launch; the last shuts down/reaps the original owned child after
  removing its accesses. Neither a PID nor a launch description reacquires lost authority.
  No implicit ownership link to another class's upstream exists: any dependency must be an
  explicit manifest closure, not a removal that secretly takes another independent grant.
- **Mount:** retain the authorized source-root FD and use descriptor-relative no-follow
  traversal, rejecting symlinks and escapes. Linux may use
  `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)`; Darwin uses a component-wise
  `openat(O_NOFOLLOW)` walk with verified directory FDs. The trusted Linux guest filesystem
  multiplexes one target with distinct root/channel attachments and inode-generation/file-handle
  identities per grant. Inspect actual root authority, effective host gates, guest mountinfo,
  connection and handle inventory. Bare mount/unmount calls or requested configuration are not
  this realization. Never abort a shared filesystem connection while a peer still needs it.

Every class closes only the selected grant's new admission and drains its actual admitted work
under the supplied single deadline. Before destructive release, timeout restores that holding's
admission and preserves its full authority and peers. After destructive progress, timeout or
failure is incomplete, not `DrainTimedOut` with a fictitious rollback. Force first closes the
selected access gate, then releases only original resources; outstanding uninterruptible host
IO remains owned/incomplete until actually terminal. Dropping a future is not cancellation.
Last removal additionally tears down the shared backing only after actual peers are absent.
For mounts, lazy detach alone does not establish absent connections/handles or closed host
access. For sockets, closed pathnames do not establish closed established transports.

### Managed files, mappings and acquired data

SessionFile and Mount use a strict mediated Linux guest filesystem. FUSE opens use direct IO;
writeback cache, passthrough, DAX and shared file mappings are disabled. Direct IO alone is
insufficient: Linux permits MAP_PRIVATE on that path. A trusted guest kernel policy must reject
**every file-backed mmap**, private or shared, of a managed inode and reject direct executable
loading from it, even while its grant is active. It must not reject ordinary anonymous memory.
An already-authorized copy into ordinary private guest scratch may be mapped or executed;
those acquired bytes are not a continuing host capability and cannot be erased by revocation.
There is no promise to revoke a secret or data already delivered to the cell.
Managed multiplexed targets use zero FUSE entry, attribute and negative-entry cache timeouts.
Every fresh unqualified resolution obtains host-authoritative selection before exposing a
grant-specific inode or using its metadata; a cached revoked dentry cannot conceal a surviving
peer. An already-bound inode/handle is never rebound to a different grant. Revalidation races
refuse or restart a fresh lookup under its original bound, not an IO replay across roots.

The policy binds immutable filesystem-lifetime/inode identity to grant generation before
workload access; map misses for managed files deny. Different grants never share cache identity,
and an old inode/handle cannot be rebound while references survive. A trusted privileged guest
loader owns/pins the policy and its maps/links; the workload cannot detach, replace or update it.
The exact guest kernel build, effective LSM activation, BTF/hook coverage and fail-closed owner
lifetime are required runtime artifacts, not inferred from a compiled CONFIG option. Relevant
Linux MAC hooks include `mmap_file` and `bprm_check_security`; all alternate syscall/loader paths
must satisfy the same denial contract. The host still checks the active grant on every real
backend request. Guest policy is not a replacement for that host boundary.

Do not claim `mmap_file` plus `file_permission` revokes existing private mappings: later faults
can consume cached or prefetched pages without either hook or a new FUSE request. The selected
contract instead prevents managed mappings from existing. Transparent mmap/exec of revocable
host-backed files would require another accepted mechanism and discriminating proof; it cannot
be enabled as a convenience fallback. Both strict denial and private-copy execution must be
proved on the actual pinned guest before implementation-ready claims.

### Broker launch without a predicted PID

The former `SpawnBroker { id, pid, name, owner }` cannot describe a new broker under spec008's
prepare-before-apply rule: `fork` supplies the PID only after creating the child. Spawning first,
using a placeholder/logical PID, predicting the next PID or backfilling the prepared inverse
would leave an unlogged effect or change the durable operation. None is a permitted implementation.

`BrokerLaunch` is the strict serde record
`{ command: Vec<String>, control_socket: String, data_socket: String }`.
All fields are required; unknown fields refuse. `command` is nonempty, its first word is an
absolute executable path, and every word is NUL-free and retained exactly. `control_socket` and
`data_socket` are distinct absolute, NUL-free host-private endpoints. Only the controlled relay
exposes the data service; the private control service never becomes workload authority.
Structural decoding does not consult the filesystem;
preflight separately validates the actual executable, private endpoint parents and ability to
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
to that exact ID, owner and launch. Failure after fork/bind exposes the typed incomplete effect
and original cleanup authority defined above; it cannot be omitted from successful observation.
Failure of reply delivery after complete application leaves the standing association intact.
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
`Grant` is removed; `LedgerEntry` carries CellOwner; `Capability::Broker` replaces PID with launch.
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
The complete nonbroker recipes and broker data endpoint refine that still-unimplemented format3
before its first writer ships; the repository baseline writes format2, not format3. A future
already-published incompatible format3 writer would require another explicit version transition,
not reuse of this refinement as silent compatibility. Missing recipes or data endpoints refuse.
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
Every clause uses caller-chosen predeclared operations and handles the fallible grant result;
an isolated fixture supplies caller-owned preparation before any real effect, not a hidden
backend WAL. Keep the same generic factory signatures and run the same behavioral clauses
against every real backend as well as models. Successful grant, retry, refused/partial grant
and direct apply have distinct receipt outcomes. Complete snapshot checks include incomplete
effects, not only an empty OsState.

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
their passing conformance does not implement or prove the actual file/listener/proxy/mount/broker
realization, workload confinement or complete independent observer that spec008 requires.
The concrete contracts above and spec001's selected runtime preserve all-five live recovery;
source acceptance still supplies neither deployed artifacts nor successful platform witnesses.

## Acceptance

- **A1 — identical grants:** for each of the five classes, one owner holds two equal capabilities
  as two exact objects with distinct handles; either withdrawal order and policy preserves the
  other object and then removes it without `UnknownObject` on an unrevoked live grant.
  IDs and complete inverses are chosen before caller-owned durable prepare and grant entry.
  The returned entry preserves them. Same-standing issued retry returns that original entry;
  a changed kind or direct-applied address cannot issue another one.
- **A2 — resource collisions:** two mount sources at one target and two routes at one host retain
  all input fields, coexist for one owner, and each exact removal preserves the other object.
- **A3 — broker residue:** same owner, broker name and complete launch with different IDs remain
  distinct; live revoke preserves exact planted residue, which its own inverse can then remove.
  Process diagnostics are separate: loss of original child authority cannot authorize a signal
  to a replacement with the same numeric PID.
- **A4 — exact refusal:** wrong owner, wrong capability, missing ID, and spent handle cannot take
  a neighbouring object. Pre-effect refusal preserves full state; post-effect failure exposes
  an incomplete obligation rather than hiding changed resources. Graceful timeout preserves
  selected authority and peers. Caller UUID failure occurs before prepare/backend invocation.
  Stall a guest drain reply while keeping all bridge streams live: the host deadline restores original
  access without a remote activation reply. A mutant that closes the guest gate and depends on
  that reply must fail. Separately break a stream: affected exact operations must become
  incomplete, not closed-but-standing or silently absent; reconnection cannot reactivate them.
  While that original drain is still unanswered, authorized Force must close only the selected
  host gate without waiting for it. Exercise priority cleanup and deliberately delayed cleanup:
  the latter stays selected-incomplete and denied while equal-peer access survives. Deliver the
  old drain reply after Force; it cannot reopen the gate. Mutants that serialize Force behind
  drain or reactivate from its late callback must fail against actual selected/peer access.
  Make A lexicographically first, Force A, and stall its guest cleanup. In a new consumer with
  no pre-opened B handle/stream, a fresh unqualified lookup/connection must select and actually
  use B. The old A binding must remain denied, never retargeted. A stale guest-selector mutant
  that black-holes this fresh request must fail; pre-opened B traffic alone is insufficient.
- **A5 — apply identity:** replay of one live recorded op is a no-op; two equal ops with fresh IDs
  create two holdings. Standing payload collision refuses without mutation. Apply or plant after
  exact removal does not revive a spent grant handle; new grants still use fresh identities.
  Cross-entry paths through apply, plant, direct removal, and revoke preserve those transitions.
  For broker apply, retry after a lost response retains the original owned child rather than
  launching another. A new resource's complete operation and inverse exist before any fork;
  no PID prediction, placeholder, pre-prepare spawn or post-fork backfill can satisfy the witness.
  A fork/bind-success then access/publication-failure witness observes typed incomplete state,
  not ordinary absence. Restart durably aborts before exact cleanup, handles a failed cleanup
  retry without losing authority, and finishes only after both sets show absence. The mutant
  that ignores or hides incomplete state must fail while the original child still serves.
- **A6 — independent backends:** independently chosen caller identities do not alias across leaves
  or fresh backend instances; class routing preserves IDs, complete snapshot collections and errors. Invalid initial leaf
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
  All-five generic real-backend conformance remains required. Exercise the public fallible
  grant boundary with predeclared ID/inverse, including a real post-preflight launch failure;
  no entry, panic-based orphan or hidden durable writer may stand in for its error transition.
  Include fallible-snapshot and ignored-drain-timeout counterexamples. A bounded real-child
  probe can establish prepare-sync-before-fork, surviving authority and lost-response deduplication,
  but it must identify its surrogate policy and cannot stand in for spec008's actual cell,
  five-class enforcement, private-transport or platform acceptance.
- **A13 — real realization:** every class's actual access attachment, backing resource and
  independent inventory meet the five-adapter contract above. Exercise missing and independently
  introduced stray resources, equal holdings, both peer withdrawal orders, last removal, safe
  staging refusal and partial cleanup. Unlink-only, route-table-only, lazy-detach-only and
  requested-refcount observers fail while actual access remains. Actual controller recovery
  still separately meets spec008R1–R12 on Darwin and Linux; a standalone mechanism is not a cell.
- **A14 — managed-file boundary:** in the pinned real guest, active and revoked managed file
  descriptors cannot acquire private/shared mappings or direct executable loads; anonymous
  mappings and authorized private-copy execution still work. Removing the mandatory denial
  fails while direct_io alone permits MAP_PRIVATE. Alternate loader/IO paths, policy loss,
  identity reuse and inherited descriptors cannot bypass the boundary. Capture real refusal,
  failed mutant and restored pass, with exact kernel/policy artifacts; an unavailable guest is
  an unproved prerequisite, not successful platform coverage.
