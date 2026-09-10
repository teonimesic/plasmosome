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
`UniverseOp`, `Grant`, `LedgerEntry`, owner comparisons and removal arguments. `Grant` becomes
`{ owner: CellOwner, capability, kind }`; `LedgerEntry` becomes `{ handle, owner: CellOwner,
capability, kind }`. Universe operations keep the field name `owner` with its widened type.
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

The new per-cell format is version 1 of `CellJournalRecord`, distinct from spec017's version-2
single-plugin `LogRecord`. The former draft's unversioned grant/revoke lines were never a
production per-cell format and are refused, not inferred or rewritten. Every record is one UTF-8
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
`UniverseOp` supplies a fresh ID and `op.removal()` before `apply`; using
`grant()` followed by recording its returned handle is expressly not this writer.

The sequence is:

1. Append and sync prepare. The previous committed desired state remains authoritative.
2. Apply only newly introduced operations, in recorded order, through the owning supervisor.
   Retained IDs are not reapplied. The supervisor enforces the exact operations; an independent
   snapshot, not an echoed request, establishes which holdings exist.
3. If all new operations succeeded, append and sync commit. This atomically publishes all
   replacements in that cell's desired state. Only now may old, unretained effects be retired,
   in reverse original grant order, preserving order across changed plugins from the replayed
   cell history. Safe-removal assertions were checked before prepare, not after publication.
4. If application fails before commit, append and sync abort BEFORE withdrawing newly applied
   operations, in reverse prepare order. Old desired state and old holdings remain unchanged.
   A failed attach's `CommitFailed` reply names this rollback; no rolled-back success is
   reported until cleanup and finish are durable. If abort cannot be made durable, stop and
   let startup resolve the pending prepare; do not acknowledge a rollback that is not recorded.
5. Append and sync finish only after all required cleanup is independently observed complete.
   A successful mutation reply requires commit and finish durable. A completed rollback reply
   requires abort and finish durable. A timeout or uncertain cleanup blocks the cell and
   reports the unfinished generation rather than declaring success or admitting another change.

Reload prepares fresh replacement holdings while old holdings remain, commits the generation
swap, then retires only the old ones. It is not a remove/add pair and never exposes a partially
published replacement as desired. If a backend cannot stage a proposed replacement without
withdrawing an old holding or widening access, validation refuses it before prepare; recovery
does not bypass that backend constraint. A multi-plugin attach uses one prepare/commit pair
for the cell, so a failure cannot publish only a prefix of its closure.

On restart, a prepare without commit is resolved to abort, then only its newly introduced
holdings are cleaned up. A commit without finish retains its new desired state and resumes the
old effects' cleanup. An abort without finish retains the prior desired state and resumes new
effects' cleanup. A completed generation is not replayed as new operations. Recovery never
reapplies a withdrawn ID or automatically creates a missing desired holding; it reports that
drift for an explicit new mutation. This prevents stale recovery from resurrecting capability.

Before each resumed withdrawal, request a fresh independent snapshot and match exact address,
full capability and cell/plugin owner. If the exact address is absent, record completion of that
obligation by observation; do not invoke its inverse again. A present conflicting payload is a
fault and blocks cleanup. A matching holding is withdrawn via its exact universe inverse and
then observed absent. `UnknownObject`, `UnknownHandle`, a timeout or a lost response is NOT
success: obtain a new observation and use the same rules. A live conflicting or unobservable
holding remains unfinished. Spec017's backend refusal contract is not weakened. For brokers,
only the surviving supervisor's original process-incarnation authority can act; a journal PID
or GrantId never permits signalling a replacement process.

Finish records completion of all obligations, not a cursor or a claim that every historical
effect was undone twice. Crash after removal but before finish is safe because restart observes
the exact ID absent. Crash during an uncompleted rollback cannot drop the old desired state.
Force authorizes only the exact obligations recorded in that prepared operation, with the
assertion also written to the existing session log before an acknowledged forced result.
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

`RecoveryObservation` contains a fresh `OsState` plus observed cell records, observation faults
and the independently verified recoverable handle records, if any. The observer has no desired
state input. Each observed record names its cell, actual lifecycle/readiness and last fully
applied generation. A snapshot includes all holdings reported by the queried supervisor,
including holdings absent from its cell journal. Supervisor lifecycle/readiness comes from
actual child/broker observation; holdings come from enforcement-side observation bound to
original identities, not a copy of controller requests or reconstructed ledgers.

`RecoveryOutcome` contains `desired`, `tombstones`, `expected: OsState`, `drift: Diff`,
`unmatched: Vec<UnmatchedRecord>`, `pending`, `quarantined: Vec<QuarantineReport>` and the
independently observed cells. Expected contains exact objects of standing committed
universe/compensating effects. A standing backend-handle effect can additionally contribute the
object from an independently retained original LedgerEntry only when its exact handle and
cell/plugin owner match; the entry is original grant authority, not a snapshot row converted
into a receipt. Otherwise it is `UnmatchedRecord { cell, plugin, effect }` and contributes no
expected object. An equal capability or planted observation is never substituted.
For `Diff::between(&expected, &observation.objects)`, added means observed but not desired,
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
fault, optional line, lines_parsed, found objects and refusal claims. A valid cell's `found`
is every independently observed object whose `owner.cell` is that cell, even if the first
journal line is corrupt and no plugin was parsed. Another cell's same-plugin objects are
excluded. An invalid raw name has no validated cell and no guessed object attribution.
The report does not expose an adoptable prefix and claims no generation.
`fault` is `NotACell` for a refused directory entry or `Journal(CellJournalFault)` for its
journal. A non-regular/symlinked journal is a Journal Io fault with no parsed lines; a
directory entry's invalid name is never passed to the journal opener.

Display escapes raw bytes and control characters unambiguously, names the instance, entry/path
and fault, emits one FOUND line per full `OsObject::describe()`, and states these refusals:
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
been classified, and all non-quarantined pending cleanup that can safely finish has been
resolved. If observation is incomplete or cleanup remains blocked, startup emits a structured
recovery error and available quarantine diagnostics on stderr, releases its own socket/lock
and exits nonzero; it does not serve a misleading partial instance. This deliberate availability
cost leaves live supervisors alone. An operator can repair the cause and restart. Journals and
unknown socket paths are never unlinked as recovery. A stale control socket after SIGKILL still
requires explicit operator/caller handling, as the existing daemon does.

On successful startup, construct ControllerState from recovered identities/modes and actual
supervisor state, retain the entire recovery account, and pass desired.generation to
Controller::new. Quarantined cells are absent from the ordinary cell registry and visible in
`plasmosome.recovery`. The controller's ready flag is false if any quarantine, unmatched effect
or drift remains; an empty instance is ready. An observed ready cell is not relabelled dead
because its desired holdings drift. The diagnostic surface states that discrepancy separately.
No journal record or recovery result by itself is an observation of liveness.

The controller reconnects to surviving supervisors and retains exact inverses for subsequent
ordinary revocation. It does not restart those supervisors, recreate missing grants or turn
snapshot rows into grant handles. `membrane.cell.desired` is sent only after a generation's
cleanup is finished and is a publication of the complete committed state. Equal/older generation
acknowledgements retain spec001's no-op meaning; they do not authorize skipping journal recovery
or fresh observation. The controller never sends an unfinished transaction as a partial desired
record at a generation the supervisor might thereafter ignore.

The concrete enforcement path is the surviving membrane's exact-operation and withdrawal RPCs
specified in spec001. Its adapters must observe and retain actual resource/holding associations,
including process incarnation for brokers. Implementing only FakeBackend or serving snapshots
from remembered requests cannot complete the live-controller acceptance below. No such real
adapter is claimed to exist today; acceptance of this contract is not evidence that it does.

### Existing APIs and implementation boundary

Core currently serves only plasmosome.status from an empty registry; its existing daemon has no
recovery root or live observation client. Membrane currently serves membrane.status, not the
recovery/operation RPCs below. The current ledger helper's append-whole-history recipe is not a
cell writer and must no longer be recommended for reopen/extend. Spec017 changes the legacy
single-plugin reader's compatibility rules independently; this contract does not depend on its
old skip-malformed-lines implementation or change its narrow torn-tail allowance.

Implementation adds core's ledger dependency, the recovery module, explicit recovered-state
carriers and daemon integration; ledger owns the typed journal reader/writer. Backend owns the
shared CellId/CellOwner/MockMode and exact-operation types; core never depends on a VMM crate.
All affected ownership constructors, comparisons, serde consumers and conformance factories
migrate together. No plugin-only or lossy compatibility fallback remains in the recovery path.
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
- **R5 — one durable append:** independently reopen after each event and observe exactly one
  added record without history duplication. Deterministic write/sync/parent-sync failures block
  further mutation and produce no successful acknowledgement or unlogged effect. Kill the
  writer after append-before-apply and inspect the file independently; fault injection separates
  successful sync ordering from any claim about actual power-loss hardware guarantees.
- **R6 — transaction publication:** exercise a multi-plugin attach and reload at every event
  boundary and after each apply/withdrawal. Before commit, old desired survives and new holdings
  roll back in reverse order. After commit, complete replacement desired survives and old
  withdrawals resume. No partial reload becomes published, and no finished/acknowledged result
  precedes observed cleanup plus durable finish. A staged replacement that cannot preserve old
  holdings refuses before prepare. Missing desired holdings are named, never auto-regranted.
- **R7 — exact resumption:** kill after an inverse but before finish, then resume against fresh
  observation. The absent exact holding is not withdrawn again; an equal neighbouring holding
  survives. Inject UnknownObject/lost reply/conflicting payload/unavailable observation: only
  verified exact absence completes an obligation. Preserve graceful timeout and explicit Force
  requirements, external assertions and broker incarnation refusal. No blanket error swallowing.
- **R8 — quarantine blast radius:** a corrupt cell is absent from desired and ordinary status,
  excluded from generation maximum and never mutated. Its report names raw path, fault/line,
  parsed count, every cell-owned found object and every refusal. Same-plugin siblings are
  adopted normally. Unknown observation is represented as unknown and prevents startup success.
- **R9 — observed startup:** run actual plasmosomed over a persistent instance with independently
  surviving supervisors and nonzero journals. While the controller is stopped, remove a promised
  holding, leave a stray, and corrupt one sibling journal. Restart, query status/recovery over
  the actual UDS, and observe correct desired cells/generations, both drift directions,
  quarantined cell absence, untouched quarantine holdings, omitted genome and false readiness.
  Inspect supervisor-side enforcement state, not a mirror populated from the same journals.
  An unavailable supervisor, partial discovery or unfinished cleanup prevents serving and emits
  the specified error without killing cells or unlinking another process's socket.
- **R10 — wire and liveness:** recovery observer replies preserve exact typed ownership and grant
  IDs, distinguish lifecycle/readiness from desired state, and bound deadlines. Wrong-cell,
  malformed, partial or wrong-ID replies cannot pass as complete observation or successful undo.
  Equal/older settled desired generation is a no-op, but does not replace fresh observation.
  A Backend handle without original supervisor records remains unmatched, even with a matching
  planted observation; a UUID or reused PID cannot restore authority.
- **R11 — discriminating proof:** in disposable mutations, skipping daemon recovery fails the
  nonzero restart case; adopting a parsed prefix fails quarantine; comparing expected to itself
  fails the missing/stray case; collapsing owners/IDs fails cross-cell and repeated-grant cases;
  moving sync after apply fails write-ahead; ignoring commit/abort or dropping pending cleanup
  fails the interrupted attach/reload cases. Record actual failures for the claimed reason,
  restored passes and limitations, not source wording checks or calls to an inert mock.
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
