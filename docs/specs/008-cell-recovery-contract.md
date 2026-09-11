---
id: 008
title: Durable cell journals and controller recovery against live observation
status: accepted
intents: [003]
---

## Behavior

A controller restart recovers the cells whose supervisors stayed running. It reads one journal
per cell, reconnects to those supervisors, rebuilds desired state, and compares it with fresh
observations. A corrupt journal quarantines its cell, not its neighbours: the controller names
the cell and its observed holdings without adopting a prefix or withdrawing those holdings.

The journal is `<instance-root>/cells/<cell>/ledger.ndjson`. It retains exact grant identities,
resolved modes, durable generations and unfinished withdrawal obligations. Recovery never turns
a pathname, PID, UUID, old handle or desired record into evidence of operating-system authority.
A missing observation is an error, not an empty snapshot. Missing and unexpected holdings remain
visible even when the cell's supervisor reports ready.

This is a contract for implementation, not a claim that recovery is already built. Accepted
spec017 supplies exact grant identity and predeclared operations; decisions002,003 and006 supply
per-cell recovery and owner-selective withdrawal. The startup and diagnostic protocol additions
are stated in spec001. Implementation admission still requires this revision accepted on main;
a local acceptance candidate or a model experiment is neither admission nor product delivery.

## Contract

### Identity, paths and the writer domain

`CellId` moves from core to `plasmosome-backend`; core consumes that one type. `CellOwner` is
`{ cell: CellId, plugin: PluginId }`. It replaces plugin-only ownership in `OsObject`,
`UniverseOp`, `LedgerEntry`, owner comparisons and removal arguments. LedgerEntry becomes
`{ handle, owner: CellOwner, capability, kind }`. Spec017 removes the old Grant input:
the fallible grant method takes a predeclared UniverseOp and GrantKind. Universe operations
keep the field name `owner` with its widened type.
This independently extends spec017's owner representation, not its grant identity semantics.
Journal changes still name a plugin and infer their cell from the validated directory; embedded
operations must carry that same cell in their owner. This is decision003's already-decided
ownership, not a restriction making plugins unique across an instance. Tool-registry ownership
is outside this recovery change.

Spec017's `GrantId`, exact `(UniverseClass, GrantId)` address, full `Capability`, conflict
refusal, handle/drain semantics and canonical comparison remain unchanged. Exact recovery diffs
compare complete objects including `CellOwner`. They never use canonical equivalence. A recorded
operation replay keeps its ID; two intended holdings use two IDs, even for identical capabilities
in one cell. Removing one cell's holding cannot remove another's. No migration manufactures IDs
or owners for old lossy records. The instance is the observation/writer domain; sharing a backend
between instances requires an independent instance partition and is not introduced here.

`plasmosome-core::state::cell_ledger_path(root: &Path, cell: &CellId) ->
Result<PathBuf, CellPathError>` is the sole journal path constructor. Valid IDs are nonempty UTF-8
strings without `/`, `\`, NUL, `.` or `..`. Invalid IDs are refused, never sanitized. The filename
is constructed once in non-test source; reader and writer use the same function. `MockMode`
moves to backend alongside `CellId`, with its existing lowercase serde vocabulary and methods;
there is no second enum, alias compatibility path or core/ledger dependency cycle.

The daemon receives an explicit absolute `instance_root`, independently of its configured
`control_socket` and validated instance name. Tests use temporary roots. The usual instance
layout in spec001 does not imply that today's arbitrary configured socket identifies a root.
The trusted operator supplies the root. The root and its ancestors are outside the cell's write
authority; cell workloads must not be able to replace journal directories or files.
Private supervisor sockets additionally require spec001 §4.1's owner/mode, kernel peer-UID and
workload-confinement boundary. A trusted path or writer lock alone does not authorize an RPC.

There is one controller writer per instance. Startup opens `<root>/controller.lock` without
following a symlink and takes a nonblocking exclusive OS file lock before discovery; contention
refuses startup. The lock is held through serving and recovery, released by process death, and
its file is not unlinked. This is cooperative local writer exclusion, not authentication or a
cross-host lock. All journal mutation runs under it and is serialized per cell. No later
transaction on a cell starts until the preceding transaction finishes or the cell is blocked.

After opening the trusted root directory, discovery and journal IO use directory-relative,
no-follow opens. `cells`, cell directories and journal files must not be symlinks; journals must
be regular files. Do not validate a path and then follow a different object through it. A
non-UTF-8 or invalid cell entry is reported by its raw Unix name, never lossy-converted into a
`CellId`. The supported platforms are macOS and Linux; their no-follow directory operations and
advisory locks are the filesystem seam, not a virtual-machine dependency in core.

### Journal records and generations

The new per-cell format is version1 of `CellJournalRecord`, distinct from spec017's version3
single-plugin `LogRecord` cutover from the implemented version2. The former draft's unversioned
grant/revoke lines were never a production per-cell format and are refused, not inferred or
rewritten. Every record is one UTF-8
JSON object followed by LF. Required fields have no defaults; unknown fields, enum variants and
versions refuse. CRLF is accepted as JSON trailing whitespace followed by LF. Empty lines refuse.

The serde envelope is `{ "format": 1, "generation": N, "event": EVENT }`. `EVENT` is one of:

- `{"prepare":{"changes":[...],"force":null}}`;
- `{"commit":{}}`;
- `{"abort":{}}`;
- `{"finish":{}}`.

`force` is required and is either null or `{ "operator": STRING, "reason": STRING }`, both
nonblank. It is a recorded operator assertion, not authentication. Each change is
`{ "plugin": PluginId, "replacement": Replacement-or-null }`; a replacement is
`{ "mock": MockMode, "effects": [RecordedEffect, ...] }`. Null removes the attachment. An empty
effects array is a real attached plasmid with a known mode, not an accidental removal. Changes
contain each plugin at most once, are nonempty, and carry the complete replacement for each
changed attachment. Unmentioned attachments remain unchanged. Effect order is grant order.

`RecordedEffect` is `{ "effect": Effect, "operation": UniverseOp-or-null }`. Exact universe
inverses and compensation witnesses carry spec017's exact ID and full capability. Their operation
is required; its object, owner and inverse must agree with the journal cell, change plugin and
witness. An identity cannot be assigned conflicting payloads or reassigned to another holding
anywhere in that cell's history. A replacement may retain an identical standing effect/ID;
otherwise newly introduced operations use fresh IDs, not retired ones. The same exact retained
record appears at most once in a replacement. Identical capabilities with different IDs remain
separate effects. Across successfully validated journals, a repeated exact address is an instance
consistency error, including when owners differ; recovery never chooses a file by listing order.
A quarantined prefix is not used as a trusted identity index.

Every complete capability carries spec017's resolved recipe, including broker control and data
endpoints. Initial file bytes/access/guest projection, upstream, exact destination/transport/port,
and mount access policy are part of the operation and its full inverse, not mutable lookup keys.
For brokers the capability is `{name, launch:BrokerLaunch}`, never a PID. Missing recipe fields,
unknown fields or any operation/inverse mismatch are strict-reader faults. Never infer a recipe
from the supervisor's current configuration, substitute a hash for its contents, or patch prepare
after creating a resource. The unimplemented per-cell format1 incorporates these required fields
before its first writer ships; implemented version2 single-plugin logs are not cell histories
and acquire neither a cell nor a recipe by being placed at the cell path.

Forensic replay can encounter an `Exact(Backend(handle))`, `External`, or `Delayed` effect with
null operation. The reader retains it rather than pretending it is a universe operation. A
backend handle is reported as unmatched unless the surviving supervisor independently proves
its original grant record and drain state; planting its snapshot is not that proof. Published
delayed effects and external assertions remain outstanding safe-removal obligations. They do
not become observed OS objects. The transaction writer defined here refuses newly introduced
opaque backend-handle or irreversible/published effects before preparing: it cannot perform
them with a predeclared undo and spec001's rollback guarantee. It can retain such records while
changing another attachment. Force can discharge recorded external/published assertions, but
cannot restore a missing backend handle's authority: an unverified opaque inverse blocks cleanup
and remains unmatched until its original supervisor authority is established.
Unpublished delayed records have no world operation and may be discarded on withdrawal. This
contract adds no external publisher or handle-import mechanism.

A generation identifies one accepted, prepared cell mutation, not a process or an object.
The first prepare is 1; subsequent prepares are exactly the preceding generation plus 1.
All terminal records for a prepare carry that same generation. Generation 0 appears in no
record and means only an empty/missing journal. `u64::MAX` refuses the next mutation before IO
or effect application; it never wraps. A prepare always records the new generation, including
an empty attachment or removal. Rejected validation and failed pre-write preparation take no
generation. Once bytes may have been appended, the writer cannot assume that number is unused.

The only valid event sequences within a generation are `prepare, commit, finish` and
`prepare, abort, finish`. At EOF, any prefix of either sequence is valid unfinished work;
commit and abort are mutually exclusive. Finish without a terminal decision, a second prepare
before finish, a skipped/decreasing/zero generation, or a repeated terminal record is a semantic
fault. Retry resolves the observed journal state; it never appends a duplicate terminal record
because its acknowledgement was lost. There is no transaction ID beyond the cell/generation.
A client retry after a completed transaction is a new request, subject to normal verb validation;
this specification promises neither exactly-once client execution nor lost-reply deduplication.

### Durable publication and interrupted transactions

Each append writes exactly one complete record and calls `File::sync_all` before returning
success. `Write::flush` alone is not durability. Creating the root's child directories, lock
file or journal also syncs the containing directory entries before a journal operation may
apply effects or acknowledge success. Directory creation is performed from opened trusted
parents. A new journal never replaces an existing inode. The durability claim assumes the OS
and filesystem honour successful sync operations; it covers controller crashes and host restart,
not storage hardware lying about flushes or recovery of running processes after power loss.

A write, sync or directory-sync error returns failure and blocks that cell's writer. No further
append, operation, rollback acknowledgement or client success is allowed from guessed memory.
Reopen under the instance lock and strict-read the original file: a valid prefix of complete
records determines recovery, while a partial final record quarantines the cell. Even complete
JSON without its LF is a torn cell-journal record and quarantines. No automatic truncate,
repair, prefix adoption or in-place migration occurs. The single-plugin reader's narrow EOF
allowance in spec017 remains a different API and is never used for cell recovery.

A mutation validates the full dependency closure, resolved mock modes, exact ownership,
identity conflicts and safe-removal/Force requirements before preparing. It computes replacement
states, newly introduced operations and old effects to retire without changing the world.
The caller chooses a fresh ID and constructs the complete UniverseOp and `op.removal()`
before preparation or either public actuation method. This controller uses direct `apply`;
a caller using spec017's fallible `grant(op, kind)` follows the same write-ahead protocol with
the predeclared operation and universe inverse. Neither grant-then-record nor backend ID
minting is permitted; receipt metadata does not introduce another journal or RPC writer.

Preflight resolves and validates every complete trusted recipe and its non-destructive staging
conditions without creating a process, listener, host attachment or guest projection. Only after
prepare is durable may the surviving supervisor create a resource and bind original authority.
Runtime handles/PIDs never replace the recorded inverse. Shared backing is allowed only with
spec017's actual independently withdrawable attachments; last removal cleans original resources.
The selected hardware runtime and trusted host/guest boundary are spec001 §4.2, not a controller
implementation dependency on libkrun or permission to launch an unrestricted host process.

The sequence is:

1. Append and sync prepare. The previous committed desired state remains authoritative.
   If prepare carries Force, durably append its session-log assertion as specified below before
   using that authority. Failure stops this cell; it does not continue toward a forced result.
2. Apply only newly introduced operations, in recorded order, through the owning supervisor.
   Retained IDs are not reapplied. The supervisor enforces the exact operations; an independent
   snapshot, not an echoed request, establishes which holdings exist.
   Spec001 §4.2 installs closed guest bindings from the exact prepared operations and activates
   each after its host authority is ready. These per-effect accesses can become visible before
   commit; there is no atomic-access or acquired-byte rollback promise. All partial bindings
   remain owned and independently observable through abort cleanup. A reload that cannot meet
   the preflight non-widening/retained-holding condition below is refused, not staged unsafely.
3. If all new operations succeeded, append and sync commit. This atomically publishes all
   replacements in that cell's desired state. Only now may old, unretained effects be retired,
   in reverse original grant order, preserving order across changed plugins from the replayed
   cell history. Safe-removal assertions were checked before prepare, not after publication.
4. If application fails before commit, append and sync abort BEFORE withdrawing newly introduced
   complete or incomplete effects, in reverse prepare order. Old desired state and old holdings remain unchanged.
   A failed attach's `CommitFailed` reply names this rollback; no rolled-back success is
   reported until cleanup and finish are durable. If abort cannot be made durable, stop and
   let startup resolve the pending prepare; do not acknowledge a rollback that is not recorded.
5. Append and sync finish only after all required cleanup is independently observed complete.
   Finish records journal-side completion; it does not yet permit a client result or the
   next transaction. A timeout or uncertain cleanup blocks the cell and reports the unfinished
   generation rather than declaring success.
6. Send the complete settled DesiredCell to `membrane.cell.desired` at this cell generation,
   require an acknowledgement matching request ID, cell and the entire published record, then
   freshly observe that same complete record. Only then may a committed mutation return success or an
   aborted mutation report completed rollback and admit the next transaction. An abort publishes
   the unchanged committed attachments at its consumed new cell generation, not the old one.
   A lost, malformed or wrong acknowledgement is not success; block further cell mutation until
   the bounded publication exchange below succeeds. Never append another finish or apply effects
   again merely to retry this publication.

Reload prepares fresh replacement holdings while old holdings remain, commits the generation
swap, then retires only the old ones. It is not a remove/add pair and never exposes a partially
published replacement as desired. If a backend cannot stage a proposed replacement without
withdrawing an old holding or widening access, validation refuses it before prepare; recovery
does not bypass that backend constraint. A multi-plugin attach uses one prepare/commit pair
for the cell, so a failure cannot publish only a prefix of its closure.

On restart, a prepare without commit is durably resolved to abort, then only its newly introduced
complete or incomplete effects are cleaned up. A commit without finish retains its new desired state and resumes the
old effects' cleanup. An abort without finish retains the prior desired state and resumes new
effects' cleanup. A completed generation is not replayed as new operations. Recovery never
reapplies a withdrawn ID or automatically creates a missing desired holding; it reports that
drift for an explicit new mutation. This prevents stale recovery from resurrecting capability.
A valid finish with a stale membrane generation still requires publication reconciliation below.
It is not an unfinished journal transaction, but it is not a settled controller/supervisor pair.

Before each resumed withdrawal, request a fresh complete EnforcementSnapshot. Match exact
address, full capability and cell/plugin owner against both its standing `state` and typed
`incomplete` operations. Only absence from BOTH sets establishes completion by observation.
An empty OsState with a matching incomplete effect is unfinished, not successful cleanup.
A present conflicting payload or duplicate classification blocks that cleanup obligation.
An incomplete effect without a matching durable cleanup obligation remains named and keeps
readiness false; it is not selected for cleanup. A matching complete or incomplete
effect is withdrawn through the same exact universe inverse and DrainSpec, using the surviving
supervisor's original authority, then freshly observed absent from both sets.

`UnknownObject`, `UnknownHandle`, IncompleteEffect, timeout and lost reply are not success.
Get a fresh complete observation and apply those same rules. A failed partial cleanup retains
the incomplete marker and all remaining original authority; it cannot disappear on a retry.
Once incomplete, grant/apply cannot resume activation or create another child. Only exact
withdrawal may complete it. Direct application and failed grant never fabricate opaque records.
The controller is this cleanup actor: it durably aborts before invoking cleanup of an uncommitted
introduction, and cannot append finish, publish settlement or return completed rollback while
any matching incomplete resource remains. No undefined autonomous cleanup is assumed.

For brokers, a post-fork/bind access or association failure exposes the original operation as
incomplete even though no successful OsObject was published. Only original child authority may
clean it; neither a diagnostic PID nor a GrantId permits signalling a replacement or unlinking
another process's endpoint. Loss of authority or unobservable residue is a blocking observation
fault. A lost successful apply response instead retains its complete standing association.
Startup resolves the durable decision and never repeats apply to revive either case.

For every class, independently account for the physical resources behind both complete holdings
and incomplete operations: open file authority, accepted sockets, established flows, guest
filesystem connections/handles and original children. Neither unlink nor lazy detach nor deleting
a route table entry establishes absence while the original access still serves. The supervisor
retains outstanding original IO authority through partial cleanup; an uncancellable operation
remains incomplete until terminal. Spec017's per-grant barriers preserve equal peers and the
pre-destructive timeout guarantee. Data already copied into private guest scratch is not an
ongoing host capability; this does not excuse a live host access path or an unobserved projection.

Finish records completion of all obligations, not a cursor or a claim that every historical
effect was undone twice. Crash after removal but before finish is safe because restart observes
the exact ID absent. Crash during an uncompleted rollback cannot drop the old desired state.
Force authorizes only the exact obligations recorded in that prepared operation. After its
prepare is durable and before using Force, append a `force` session-log event carrying the
cell, generation and exact operator/reason pair. `SessionLog::append` becomes
`Result<u64, SessionLogError>`: successful append means the complete LF-terminated event was
written, flushed and `File::sync_all` succeeded, not just that a sequence number was reserved.
Creating the log or its parent directories also durably syncs their containing entries.
Propagate write, flush, file-sync and directory-sync errors; none may be swallowed.
An audit error blocks this cell before forced cleanup and prevents a forced acknowledgement
or further mutation, even if the journal prepare is already durable. Reopen/recover rather
than proceeding from guessed memory. The session log is not a second recovery authority:
the strict cell journal still determines exact obligations. Recovery durably appends the same
cell/generation assertion before resuming Force; duplicate audit assertions after an uncertain
append are allowed and do not mean the operation ran twice. They attest authorization, not
completion. This does not require an atomic commit across the journal and session log.
The shared log writer refuses every later append after an IO error until it is reopened and
validated. Before extending an existing log, require complete LF-terminated UTF-8 JSON objects;
a torn or malformed audit log refuses further audit rather than appending onto a corrupt suffix,
skipping it or truncating it. Reopening re-establishes the required directory durability too.
Quarantine is never an implicit Force and has no automatic cleanup path.

This deliberately replaces the earlier draft's boundary-free partial-reload proposal. That
proposal could not preserve spec001's whole-closure rollback, empty attachments or durable
teardown obligations. The four records are an append-only protocol in one per-cell file, not a
database, multi-file atomic commit, or a second durable generation store. No record is edited.

### Strict read and the recovered account

`plasmosome-ledger::read_cell_journal(path: &Path) ->
Result<Vec<CellJournalRecord>, CellJournalFault>` returns every record or no history.
`CellJournalFault` carries `path: PathBuf`, `line: Option<u64>`, `lines_parsed: u64`, and kind
`Io | Unparseable | UnsupportedFormat | GenerationZero | GenerationOrder | EventOrder |
InvalidEffect`. Read framing as bytes: invalid UTF-8, a malformed complete record and any
non-LF-terminated final record all fault. A nonempty physical line is counted once; CRLF does
not add a line. The reader stops at the first fault. `lines_parsed` counts only complete,
syntactically and semantically accepted records before it, including terminal records.
A mid-read IO error identifies the next physical line where possible; open errors have no line.
No parsed records or decoded prefix owners are returned on failure. Missing and empty files
return an empty history, not a history read using the single-plugin compatibility rules.

`recover(instance: &InstanceName, root: &Path, observation: &RecoveryObservation) ->
Result<RecoveryOutcome, RecoveryError>` is a read-only core operation. It does not append,
apply, remove, signal, manufacture a snapshot or create controller status from desired data.
Recovery orchestration separately resolves pending transactions and repeats read/observation
before serving. The writer retains validated journal handles under the same instance lock;
strict reads are never raced against another controller append.

`DesiredState` retains `generation: u64` and `cells: BTreeMap<CellId, DesiredCell>`.
`DesiredCell` is `{ generation: u64, genome: Option<GenomeName>,
plasmids: Vec<DesiredPlasmid> }`. `DesiredPlasmid` is `{ plugin: PluginId, mock: MockMode,
generation: u64, effects: Vec<RecordedEffect> }`. This is separate from the existing compact
status `PlasmidRecord`; a status projection cannot replace the recovered effects.
`RecoveryOutcome.tombstones` is `BTreeMap<CellId, BTreeMap<PluginId, u64>>`, retaining removed
attachments' last committed change generations rather than fake attached plasmids. A cell's
generation is the last prepare generation, including an aborted or unfinished transaction;
committed attachment generations only change on commit. The instance generation is the maximum
adopted cell generation, or 0. It is a status aggregate, never a watermark deciding whether
another cell's mutation should run. Per-cell desired pushes carry the cell's settled generation.
For publication, both live execution and restart use the same settled journal replay: plasmids
are ordered by PluginId, effects retain recorded order, and genome is None because this journal
does not persist it. Unrecorded live metadata cannot enter the published record and disappear
on restart. Complete-content equality compares every decoded field, including attachment
generations, modes, ordered effects and exact operations, not JSON formatting or only OS holdings.

An empty journal adopts an empty generation-0 desired cell; an empty attachment remains an
attachment. No recovered genome is invented: it is None internally and omitted on spec001's
wire. Recovery retains both pending retirement obligations and the newly prepared effects in
`PendingCellTransaction { generation, phase, changes, cleanup }`. They are not silently mixed
into committed desired state. Reconstructed effects retain file order, and cleanup order is
reverse grant order, not map-key order. A per-effect replay ordinal derived from journal position
provides that order; it is not an additional durable identity.
`pending` is a `BTreeMap<CellId, PendingCellTransaction>`. Its phase is the closed enum
`Prepared | Committed | Aborted`; cleanup is an ordered list of
`CleanupObligation { plugin, effect, grant_ordinal }`. Retained effects keep their original
ordinal; newly committed effects take prepare position plus their changes/effects position.
The ordinal is a tuple, not a plugin-name sort or a wrapping arithmetic counter.

`RecoveryObservation` contains `snapshot: EnforcementSnapshot` plus observed cell
records and observation faults. Its original issued records are the only recoverable handles.
The observer has no desired-state input. Each cell record names actual lifecycle/readiness and the complete
last published `desired: DesiredCell`, retained by the supervisor atomically with its generation.
That record proves publication content only, never liveness or enforcement. A snapshot includes
all holdings reported by the queried supervisor,
including holdings absent from its cell journal. Supervisor lifecycle/readiness comes from
actual child/broker observation; holdings come from enforcement-side observation bound to
original identities, not a copy of controller requests or reconstructed ledgers.
Every backend snapshot is fallible under spec017. A failed or unsupported class fails the
observation; it cannot be replaced by an empty class, a partial Composite union or the requested
broker launch. That launch identifies the intended capability, not evidence that its process
exists. The original runtime association is required independently.

Each observed cell record additionally carries spec001's complete `guest: GuestObservation`,
obtained independently from that same original supervisor/guest association. Its physical
projection inventory is not a second requested-state model or additional logical grant class.
Validate immutable boot/policy identity, actual managed mounts/handles/listeners/flows, and their
exact associations to this cell's complete or incomplete operations before returning a complete
observation. Stray verified holdings still appear in exact drift; unknown/unattributable physical
resources, policy loss or incomplete guest inventory are Observation errors naming the resource,
not empty projections or guessed grant IDs. The actual guest system image and private copied data
are not managed capability projections; the observer must explicitly delimit the managed
namespaces rather than classify arbitrary guest processes as trusted evidence.
Every projection array, including all nested network kinds, and actual per-binding admission
states are mandatory even when empty; omitting an otherwise-empty array fails the observation.

Cell records and the resulting `observed_cells` diagnostic retain this guest account so a
controller cannot silently discard it while preserving only OsState. No journal field grants
authority to recreate a dead guest, reload a missing policy, adopt a replaced boot or manufacture
original host handles. A source recipe or matching artifact digest proves intended input, not
effective isolation. The runtime/configuration and observation faults remain separate from
the sole durable per-cell generation/decision journal.

`RecoveryOutcome` contains `desired`, `tombstones`, `expected: OsState`, `drift: Diff`,
`unmatched: Vec<UnmatchedRecord>`, `incomplete: Vec<IncompleteEffect>`, `pending`,
`quarantined: Vec<QuarantineReport>` and independently observed cells.
Incomplete contains every independently observed incomplete operation, with exact cell/plugin
ownership; it is a separate unfinished-effects account, not coerced into an OsObject or silently
removed from drift. Matching cleanup obligations are resolved as above; other incomplete effects
remain reported and block readiness without authorizing guessed cleanup.
Expected contains exact objects of standing committed
universe/compensating effects. A standing backend-handle effect can additionally contribute the
object from an independently retained original LedgerEntry only when its exact handle and
cell/plugin owner match; the entry is original grant authority, not a snapshot row converted
into a receipt. Otherwise it is `UnmatchedRecord { cell, plugin, effect }` and contributes no
expected object. An equal capability or planted observation is never substituted.
For `Diff::between(&expected, &observation.snapshot.state)`, added means observed but not desired,
removed means desired but missing. Pending and quarantined holdings may therefore be in added;
their reports explain attribution rather than erase differences. Replayed exact operations
produce one object; fresh IDs remain distinct. External and published delayed assertions stay
explicit obligations, not claims of snapshot verification. `recover` refuses observation faults
with `RecoveryError::Observation { cell, source }`; it cannot return a successful partial account.

A missing `cells/` directory is a fresh instance. Failure to enumerate its complete contents,
including an iterator error after some entries, is `RecoveryError::Discovery { path, source }`
and yields no outcome/adoption. A symlink or non-directory at `cells/` is also Discovery.
A known entry that is not a directory, has an invalid/non-UTF-8 name, or is a symlink is a
per-entry quarantine. A symlink/non-regular journal, per-cell open/read/permission failure or
strict-reader fault quarantines that cell while good siblings remain recoverable. A missing
journal is empty, but never implies that independently observed holdings are absent or revocable.
An instance identity collision is `RecoveryError::IdentityConflict { class, id, paths }` and
aborts the account rather than merging objects. Observe all valid cell directories, including
ones whose journals quarantine, not just the adopted subset. Unknown names remain reported
without being turned into requests to arbitrary socket paths.

### Quarantine and startup integration

`QuarantineReport` carries instance, optional validated cell, raw entry name bytes, raw path,
fault, optional line, lines_parsed, found objects, found incomplete effects and refusal claims.
A valid cell's `found` is every independently observed object whose owner.cell is that cell;
`incomplete` likewise contains every incomplete operation owned by the cell, even if the first
journal line is corrupt and no plugin was parsed. Neither set authorizes cleanup.
Another cell's same-plugin records are excluded. Invalid raw names have no guessed attribution.
The report does not expose an adoptable prefix and claims no generation.
`fault` is `NotACell` for a refused directory entry or `Journal(CellJournalFault)` for its
journal. A non-regular/symlinked journal is a Journal Io fault with no parsed lines; a
directory entry's invalid name is never passed to the journal opener.

Display escapes raw bytes and control characters unambiguously, names the instance, entry/path
and fault, emits one FOUND line per full OsObject and one INCOMPLETE line per full operation,
and states these refusals:
not adopted; no trusted prefix; no claimed generation; no withdrawal without separately recorded
operator Force and independently verified exact authority. The wire report encodes raw names and
paths as arrays of byte values, not lossy strings. `cell` and `line` are omitted when absent.
A quarantine report with unavailable observation says so; an unknown found set is not `[]`.

Startup holds the instance lock, binds the configured control socket without replacing an
existing path, discovers every entry, and queries every validated cell's
`<root>/cells/<cell>/membrane.uds` using spec001's live recovery observation contract. Only
socket paths under validated no-follow directories are used. A missing/failed/timed-out or
malformed live response is an observation failure, not an empty cell. A cell's supervisor is
allowed to report an observed dead cell; the controller must not derive that state from a PID
file, socket existence or inability to connect.

No requests are answered until complete discovery and observation have succeeded, journals have
been classified, pending cleanup has completed and every adopted cell's complete settled record
has been reconciled with its membrane. Compare generations before any resumed effect operation:
an observation newer than the journal is an unaccounted participant state and refuses startup
without cleanup. A pending generation cannot already have been published at that same generation:
publication follows durable finish, so that case also refuses rather than acknowledging an
unfinished or subsequently aborted payload.

After cleanup and durable finish, reconcile each adopted, non-quarantined cell independently:

| Observed membrane generation versus settled journal generation | Startup action |
| --- | --- |
| Lower | Send the complete settled DesiredCell at the journal generation; require matching request-ID/cell/full-record acknowledgement and a fresh observation of that same full record before serving. |
| Equal | Continue only when every field of the observed DesiredCell matches settled journal replay. A content mismatch refuses without overwriting the membrane; matching records require no operations or republication. |
| Higher | Refuse startup as unaccounted participant state. Never send an older generation and treat its no-op acknowledgement as repair. |

These rules include generation 0 and aborted generations. For an abort, publication carries
the unchanged committed attachments and attachment generations under the consumed new cell
generation; the recovery account's tombstones are unchanged. If the controller dies before
sending, after the request but before acknowledgement, or after acknowledgement but before
replying to its client, restart re-observes
and applies this table. Lost acknowledgement may safely resend the identical settled record:
equal generation is a no-op only with identical content, and its matching acknowledgement must
still be followed by fresh complete-record observation. A lower post-ack generation retries
within the remaining deadline; a higher generation or equal-generation content mismatch refuses.
No retry applies effects, changes IDs, consumes another generation or adds journal records.
An older desired request remains a no-op but returns the newer retained record, not a claim that
the requested old content was published. No historical payload store is required for old requests.

All startup publication requests, acknowledgements, re-observations and retries share the existing
single recovery deadline; they do not replenish it. The same publication exchange after a live
mutation is bounded by one recovery_deadline_ms budget starting after durable finish, with no
budget reset on retry. Failure blocks that cell's next mutation and prevents a successful client
result; recovery readiness is false until publication is settled.
If observation, cleanup or publication remains incomplete, startup emits a structured recovery
error and available quarantine diagnostics on stderr, releases its own socket/lock and exits
nonzero rather than serving a partial instance. Generation refusal reports the cell and both
numeric generations as an observation error. Equal-generation content mismatch is
`RecoveryError::DesiredConflict { cell, generation }`, reported with spec001's structured
`desired_conflict` kind. It is not repaired by overwriting content or ignoring a field.
This availability cost leaves live supervisors alone. An operator can repair the cause and
restart. Journals and unknown socket paths are never
unlinked as recovery. A stale control socket after SIGKILL still requires explicit operator/caller
handling, as the existing daemon does.

On successful startup, construct ControllerState from recovered identities/modes and actual
supervisor state, retain the entire recovery account, and pass desired.generation to
Controller::new. Quarantined cells are absent from the ordinary cell registry and visible in
`plasmosome.recovery`. The controller's ready flag is false if any quarantine, unmatched effect,
incomplete effect, drift or unsettled publication remains; an empty instance is ready only after the same startup
requirements. An observed ready cell is not relabelled dead because its desired holdings drift.
The diagnostic surface states that discrepancy separately. Equal object snapshots do not
establish equal participant generations or publication content. Matching published records do
not establish matching holdings. No journal record or recovery result by itself proves liveness.

The controller reconnects to surviving supervisors and retains exact inverses for subsequent
ordinary revocation. It does not restart those supervisors, recreate missing grants or turn
snapshot rows into grant handles. The publication exchange above sends only complete settled
state after durable finish and before a completed client result. Spec001 binds equal-generation
no-op acknowledgement to identical content; older requests return the current retained record.
Generation and complete-content comparison, not an echoed acknowledgement, decide settlement.
An unfinished or partial desired record is never published.

The concrete enforcement path is the surviving membrane's exact-operation and withdrawal RPCs
specified in spec001. Its adapters must observe and retain actual resource/holding associations,
including process incarnation for brokers. Implementing only FakeBackend or serving snapshots
from remembered requests cannot complete the live-controller acceptance below. No such real
adapter is claimed to exist today; acceptance of this contract is not evidence that it does.
The first broker holding can create a new broker only under the predeclared launch contract.
Spec017 now defines the five concrete adapter obligations and spec001 selects the hardware runtime
and private bridge. Their actual guest artifacts, effective strict-mmap policy, host confinement
and platform witnesses remain required before implementation-ready claims. A generic subprocess,
sandbox or file-mapping probe does not supply a complete cell, all-five observer or deployment.
Missing Linux FUSE/LSM access or Darwin HVF/profile compatibility must be recorded as unavailable,
not replaced by model success. None weakens the live-recovery acceptance.

### Existing APIs and implementation boundary

Core currently serves only plasmosome.status from an empty registry; its existing daemon has no
recovery root or live observation client. Membrane currently serves membrane.status, not the
recovery/operation RPCs below. The current ledger helper's append-whole-history recipe is not a
cell writer and must no longer be recommended for reopen/extend. The implemented legacy reader
already has spec017's strict version2 rules; the revised broker representation now requires its
explicit version3 cutover. That reader's narrow torn-tail allowance is unchanged, but append
refuses an incomplete target. Neither legacy reader supplies this cell reader's strict-LF rule.

Implementation adds core's ledger dependency, the recovery module, explicit recovered-state
carriers and daemon integration; ledger owns the typed journal reader/writer. Backend owns the
shared CellId/CellOwner/MockMode and exact-operation types; core never depends on a VMM crate.
All affected ownership constructors, comparisons, serde consumers and conformance factories
migrate together. No plugin-only or lossy compatibility fallback remains in the recovery path.
This includes broker launch constructors and embedded inverses, caller-prepared fallible grant
calls, all complete-snapshot consumers and every `apply_removal(removal, &CellOwner, DrainSpec)`
call. No Grant input, backend ID minting, infallible/partial observation, absence-only completion,
default recipe, PID fallback or drain-free exact-removal wrapper remains.
Complete resource recipes, guest observation records and all snapshot/desired/recovery serde
consumers cut over together; no old three-collection RPC drops the required guest account.
The public logical backend snapshot remains spec017's three collections; the supervisor's
physical guest account is additional independently checked observation, not a fake sixth class.
The session-log return-contract change includes all append callers and benchmarks; no infallible
audit fallback remains. Socket creation, both connection endpoints and workload launch must
establish the spec001 authorization boundary before exposing recovery operations.
Native implementation plans, ownership, API migration lists, proof output and admission state
remain in Beads under specs012/016, not duplicated as a task in this document.

## Acceptance

- **R1 — common path and safe discovery:** valid cell IDs resolve beneath the supplied root;
  empty, dot/dot-dot, slash, backslash and NUL IDs refuse. Both real writer and reader resolve
  through the same constructor. Invalid/raw non-UTF-8 entries, symlinked cells/journals and
  regular-file entries quarantine without accessing the link target; good siblings survive.
  Missing cells/ is empty, but non-directory/symlink cells/ and partial listing failure abort.
- **R2 — complete strict history:** missing/empty journals are empty; every required field,
  closed mode, record format and exact inverse is validated. Bad middle/final JSON, invalid
  UTF-8, unknown fields/version and complete final JSON without LF fault at the right physical
  line. No partial history escapes, no source file changes, and diagnostics count only the
  accepted prefix. A corrupt first line still permits cell-aware found attribution.
- **R3 — durable generation:** prepare/commit/finish and prepare/abort/finish round-trip;
  interrupted legal prefixes are pending. Zero, decreasing/skipped generations, duplicate or
  reordered terminal events refuse. Empty attachments and removal consume durable generations;
  aborted generation remains consumed. Exhaustion refuses without write/apply. Two cells at
  3 and 5 report instance5 while retaining their own generations and attachment tombstones.
- **R4 — exact identity:** two cells share a plugin and capability without coalescing objects or
  quarantine attribution; one cell's two equal fresh grants remain independent. Replayed exact
  operations retain IDs, changed-payload/recycled-ID records refuse, and duplicate addresses
  across cell histories abort instead of choosing a file. Exact drift detects replacement,
  lost objects and stray objects; canonical equivalence is never used as its substitute.
  Include broker launch content in exact equality; PID diagnostics cannot manufacture an
  original association. Equal broker holdings retain distinct actual access until their own
  withdrawal; the final removal cleans the shared resource.
- **R5 — one durable append:** independently reopen after each event and observe exactly one
  added record without history duplication. Deterministic write/sync/parent-sync failures block
  further mutation and produce no successful acknowledgement or unlogged effect. Kill the
  writer after append-before-apply and inspect the file independently; fault injection separates
  successful sync ordering from any claim about actual power-loss hardware guarantees.
  The broker case independently inspects a complete, synced prepare and inverse before the
  actual fork. A spawn-before-prepare or post-fork-PID-backfill mutant exposes an unlogged child
  and fails; a recipe-only policy check is not that real-child ordering proof.
  Exercise that same order through public grant(op, kind), not only direct apply; the returned
  entry must preserve the prepared ID and full operation. A post-preflight resource race is a
  fallible result with no new issued entry, not a panic-based orphan or hidden grant-side WAL.
- **R6 — transaction publication:** exercise a multi-plugin attach and reload at every event
  boundary and after each apply/withdrawal. Before commit, old desired survives and new holdings
  roll back in reverse order. After commit, complete replacement desired survives and old
  withdrawals resume. No partial reload becomes published. Durable finish precedes the complete
  desired request, matching full-record acknowledgement and fresh full-record observation; all
  precede client success or a completed rollback reply. Kill before request, after request/before ack,
  after ack/before reply and after reply, for committed and aborted generations. A staged
  replacement that cannot preserve old holdings refuses before prepare. Missing desired holdings
  are named, never auto-regranted.
- **R7 — exact resumption:** kill after an inverse but before finish, then resume against fresh
  observation. The exact address absent from both standing and incomplete collections is not
  withdrawn again; an equal neighbouring holding survives. Inject UnknownObject/lost reply,
  conflicting payload/unavailable observation: only verified complete absence completes.
  Preserve graceful timeout, explicit Force requirements, external assertions and broker incarnation refusal.
  No blanket error swallowing.
  Loss of a broker apply response followed by controller restart must retain the original
  association and resolve pending cleanup without a duplicate spawn. An unknown incarnation
  cannot target a same-number replacement. Direct exact removal must enforce its supplied
  graceful deadline and preserve peers on timeout, not bypass drain because it uses an inverse.
  In the selected runtime, stall the non-mutating guest drain observation while streams remain
  live: the host-local deadline restores access with no remote activation dependency. Actual
  stream loss instead exposes affected operations as incomplete and permits only exact cleanup
  under original authority; a same-boot reconnect or changed-boot replacement cannot promote them.
  After real fork and successful bind, inject access/association failure before successful
  publication. Observe the typed incomplete operation with no complete object or fabricated
  entry. Kill the controller, restart, durably abort, then exact-withdraw using retained original
  authority. A timed-out/failed cleanup retains that marker and authority; exact retry cleans
  only its remaining resources, including no endpoint owned by a competing process.
  Finish and publication are forbidden while the marker remains. A successful cleanup with
  lost reply completes only after fresh absence from both sets. Unknown/quarantined incomplete
  effects remain named, untouched and blocking, not automatically selected for cleanup.
  For Force, independently reopen the session log and match its cell/generation/operator/reason
  to the prepared assertion. Inject write, flush, file-sync and new-parent-sync failures: no
  forced cleanup, successful acknowledgement or next mutation may pass the failed audit.
  Kill between prepare, audit persistence and forced cleanup; restart must durably reassert
  authorization before resuming exact obligations, without treating an audit event as completion.
  A partial audit write must also prevent another cell from appending to that shared writer;
  reopening a torn audit log refuses without changing its bytes. A complete uncertain append
  may be followed by another durable assertion with the same cell/generation after validation.
- **R8 — quarantine blast radius:** a corrupt cell is absent from desired and ordinary status,
  excluded from generation maximum and never mutated. Its report names raw path, fault/line,
  parsed count, every cell-owned found object and incomplete operation, and every refusal.
  Same-plugin siblings are adopted normally. Unknown observation prevents startup success.
- **R9 — observed startup:** run actual plasmosomed over a persistent instance with independently
  surviving supervisors and nonzero journals. While the controller is stopped, remove a promised
  holding, leave a stray, and corrupt one sibling journal. Restart, query status/recovery over
  the actual UDS, and observe correct desired cells/generations, both drift directions,
  quarantined cell absence, untouched quarantine holdings, omitted genome and false readiness.
  Inspect supervisor-side enforcement state, not a mirror populated from the same journals.
  An unavailable supervisor, partial discovery, unfinished cleanup or unresolved publication
  prevents serving and emits the specified error without killing cells or unlinking another
  process's socket. Lower observed generation republishes and converges within the single
  deadline; equal continues only with identical complete content; higher refuses even when all
  holdings match. Change an empty attachment's mode at the same generation: equal empty OS
  snapshots must not hide the publication conflict. Refuse without overwriting either record.
- **R10 — wire and liveness:** recovery observer replies preserve exact typed ownership and grant
  IDs, distinguish lifecycle/readiness from desired state, and bound deadlines. Wrong-cell,
  malformed, partial or wrong-ID replies cannot pass as complete observation or successful undo.
  Equal-generation identical desired requests are no-ops; conflicting content returns the typed
  refusal. Older requests return the current record without mutation and cannot settle the old
  request. Wrong/lost acks cannot settle a mutation; lower post-ack generation retries within the
  deadline, and higher generation or equal-generation content mismatch refuses. Check complete
  ordered effects/operations, attachment generations/modes, missing retained content, and the
  same journal-derived representation for live and restarted publishers, including generation0.
  Aborted generations publish unchanged settled attachments at the consumed cell generation.
  A Backend handle without original supervisor records remains unmatched, even with a matching
  planted observation; a UUID or reused PID cannot restore authority.
  Exercise the actual private UDS boundary on each supported platform: the trusted peer can
  observe, while a different-UID local client and a confined cell workload cannot reach any
  recovery handler or change holdings/publication. Wrong-owner, permissive/ACL-exposed or symlinked
  socket parents refuse startup; peer-credential lookup failure or mismatch closes before
  dispatch. A passing same-UID client alone does not prove workload exclusion. Report any
  unavailable distinct-UID/confinement fixture as unproved, never as a successful authorization test.
  Exercise both platforms' actual selected hardware guest and all five adapters, including
  host confinement, privileged-FD exclusion and guest policy/boot identity. Unknown or stray
  managed guest projections cannot disappear from a complete observation. Use the strict-file
  denial/private-copy witnesses in spec017A14; ordinary chmod/unlink and an existing Docker
  process are not substitutes for managed guest FUSE/LSM enforcement.
  Omitting one otherwise-empty projection-kind array must likewise fail complete observation.
- **R11 — discriminating proof:** in disposable mutations, skipping daemon recovery fails the
  nonzero restart case; adopting a parsed prefix fails quarantine; comparing expected to itself
  fails the missing/stray case; collapsing owners/IDs fails cross-cell and repeated-grant cases;
  moving sync after apply fails write-ahead; ignoring commit/abort or dropping pending cleanup
  fails the interrupted attach/reload cases. Permitting a client result before the matching
  desired acknowledgement, ignoring a stale/ahead membrane generation, comparing only generation,
  or checking reply content without fresh observed content must fail the publication/restart
  cases. Record actual failures for the claimed reason, restored passes and limitations, not
  source wording checks or calls to an inert mock.
  Accepting a recovery connection without its authorization boundary must fail the unauthorized
  client case; ignoring audit IO errors or acknowledging Force before durable audit must fail
  the Force crash/fault cases.
  Returning an empty snapshot on a class error, ignoring exact-removal drain, recreating a broker
  after an uncertain apply reply or guessing a launch for an old PID record must fail their
  corresponding cases. Label injected PID aliases separately from actual kernel PID recycling.
  Omitting incomplete state or deciding completion from OsState alone must fail while the
  post-fork failed child's resource still serves. Minting an ID inside grant after preparation,
  or accepting a new issued receipt for a direct-applied object, must fail the public grant
  identity/order/receipt cases. Preserve the same all-five real-backend conformance contract.
  An unlink-only file/socket inverse, table-only flow removal, lazy-detach-only mount cleanup,
  hidden original IO, discarded guest inventory or changed-boot adoption must fail the matching
  real held-resource/restart scenario. Disabling strict managed mapping denial must fail while
  MAP_PRIVATE still succeeds; acquired private copies must not be falsely reported as revoked.
- **R12 — integration:** the complete accepted control/recovery contract is served, all affected
  consumers migrate and the root gate passes. Independent review checks the acceptance against
  the actual final head. Portable journal/model proofs are reported separately from macOS/Linux
  filesystem proofs and real supervisor/enforcement proofs. A green model alone is not delivery.

## Out of scope

A database or compacting snapshot, multi-host writer coordination, exactly-once client requests,
automatic repair of corrupt journals, genome persistence, restoring dead supervisors/VMs after
host failure, automatic creation of missing capabilities, a new external-effects publisher,
resource reconstruction from UUIDs/PIDs, and a task-storage or review-workflow change.
Operator-authorized quarantine cleanup requires a separate exact-authority operation; no startup
code silently turns a report into a Force request. These limits do not exclude reconnecting to
live cells, durable rollback/withdrawal recovery or actual observation: those are this contract.
