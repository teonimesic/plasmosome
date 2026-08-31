---
id: 008
title: The per-cell ledger path, the durable ledger generation, and the quarantine report
status: draft
intents: []
---

## Behavior

Decision 002 settled the shape of recovery: one append-only ledger per cell, replayed on
restart and reconciled against what the operating system shows, with quarantine instead of
half-recovery when a ledger does not parse. It left three contract details open. This spec
closes them: a cell's ledger lives at `<instance-root>/cells/<cell>/ledger.ndjson`, derived by
one function that both the writer and recovery call; `ledger_generation` is a per-cell counter
carried on every ledger line, so the file itself is its durable home; and the quarantine
report names the cell, the fault, what the snapshot shows, and exactly what the controller
refuses to claim.

The instance root is not new. Spec 001 §1 already fixes it as the directory holding
`control.uds` (`~/.plasmosome/instances/<name>/` in production), and §4 already places each
cell's `membrane.uds` in `cells/<cell>/`. This spec adds one filename inside that existing
per-cell directory and nothing else. The root always arrives as an argument — tests pass a
temporary directory — and the cell id is validated before it becomes a path component,
because `CellId` today accepts any string, including path-shaped ones.

Recovery reads that layout end to end. It lists the entries of `cells/`, derives each cell's
ledger path, and reads the file with a strict reader: every line must parse, and generations
must never decrease. A clean read yields the cell's desired state — the plasmids whose grants
still stand after any revokes, their effects, their modes, the cell's generation. Any
per-cell fault quarantines that cell and recovery moves on; a failure to list `cells/` at
all aborts recovery with an error, because a partial listing cannot say which cells it
missed. The desired states of the adopted cells become one `DesiredState`, the expected
operating-system objects are rebuilt from the standing grants' inverses, and `Diff::between`
against the backend snapshot names every difference.

### Decision 1 — where a cell's ledger lives

`<instance-root>/cells/<cell>/ledger.ndjson`.

One function derives it, and it lives in `plasmosome-core::state` beside the `InstanceName`
parser. It takes the instance root as a `&Path` and the `CellId`, and returns the path or an
error. It applies the same rules `InstanceName::parse` applies: the cell id must be
non-empty, contain no `/`, `\` or NUL, and must not be `.` or `..`. A cell id that fails is
refused with a named error, never sanitized. No other code builds this path; the writer and
the recovery reader both call this function.

The file is ndjson: one line per event, in append order. The line shapes are the next
decision's.

### Decision 2 — where `ledger_generation` is durably kept

In the ledger itself, on every line. A line is one of two shapes, externally tagged:

```json
{"grant": {"plugin": "github-pr", "mock": "simulate", "generation": 3, "effect": {"...": "..."}}}
{"revoke": {"plugin": "github-pr", "generation": 4}}
```

- A `grant` line records one effect granted to the cell on the named plasmid's behalf.
  `mock` is the plasmid's mock mode at grant time; without it a recovered cell would have to
  claim a mode it cannot know, and claiming `passthrough` for a simulated plasmid is a claim
  in the unsafe direction.
- A `revoke` line records that every effect the named plasmid was granted before this line
  is no longer desired. `plasmid.remove` writes one. `plasmid.reload` writes one followed by
  the reloaded plasmid's fresh `grant` lines, all at the same new generation. A reload is
  not atomic across its lines; the crash window between them is defined below, under what
  the writer guarantees.

All fields are required in both shapes; the recovery reader defaults nothing. Nothing
durable exists in any older shape — no production writer of per-cell ledgers exists before
this spec — so there is no history to stay readable for.

`mock` takes exactly three values, and their wire strings are part of this contract:
`"simulate"`, `"capture"`, `"passthrough"` — the closed D2 vocabulary spec 001 §3.9 freezes,
in the lowercase serde form `plasmosome-core::state::MockMode` already writes. A fourth
string does not parse, and on the recovery path an unparseable line quarantines the cell.

The generation rules:

- The generation has no durable home other than the lines. An accepted mutating transaction
  on a cell increments the cell's generation once and stamps every line it appends with the
  new value. A transaction that appends no line cannot move the generation — and the
  transactions that grant nothing, removal and reload, always append their `revoke` line, so
  the generation they took is on disk and recovery reads it back.
- Within a file, generations never decrease. A decrease is a fault and quarantines the cell.
- Every line's generation is at least 1. Generation 0 is reserved: it is the reading of a
  cell with no lines at all, and only that. A line carrying 0 is a fault and quarantines
  the cell.
- A cell's `ledger_generation` is the generation of the last line of its ledger, whichever
  shape that line is. A cell with no ledger file, or an empty one, is a fresh cell:
  generation 0, no granted effects, still adopted. A quarantined cell has no generation; the
  controller does not claim one.
- The per-plasmid `generation` spec 001 §3.9–§3.12 reports is the generation of that
  plasmid's last line in its cell's ledger.

Spec 001 §3.3 reports one `ledger_generation` under `controller`. That number is defined
here as the maximum generation across the adopted cells, and 0 when there are none. This is
the value passed to `Controller::new`, whose doc already says the caller supplies it, and it
is the `generation` of the `DesiredState` recovery hands to the `Reconciler`. A per-cell
membrane push (`membrane.cell.desired`, spec 001 §4) carries that cell's own generation.

A plasmid's mode after replay is defined the same way its effects are: by its standing
lines. The recovered mode is the `mock` of the plasmid's **last `grant` line after its last
`revoke` line**. This is not the last-write-wins that D2b forbids: D2b governs conflicting
declarations at attach time, and those are resolved before anything is written. The ledger
records only resolved outcomes, so the latest record is not one side of a conflict — it is
the newest fact.

### What the writer guarantees — and what it does not

- Appending is per line: one ndjson line per grant or revoke. A transaction's lines are
  flushed to disk before the transaction is acknowledged to its caller.
- Write-ahead order: a `grant` line is durable **before** its effect is applied to the
  world. The invariant this buys is the one the project needs: no object the controller
  created exists without a line claiming it. A crash between flush and apply leaves a line
  without a world object, and recovery names that as `missing` drift rather than hiding it.
- A `revoke` line is write-ahead too: durable before any inverse runs. The direction is
  chosen so a crash can never resurrect a capability — once the revoke is on disk, no
  recovery will claim those effects as desired again. What a crash can leave is objects
  that exist but are no longer claimed, and those are named on the snapshot side of
  `drift`.
- A multi-line transaction has no boundary marker, and this spec does not add one — a
  marker would be a transaction in everything but name, and this contract has already
  said it does not have transactions. The real crash window is a reload's, and appending
  per line makes it a spectrum: disk can hold the `revoke` and any prefix of the fresh
  grants, from none of them to all but the last. Every point on that spectrum is a valid
  ledger, and **recovery adopts exactly what the standing lines say**. The empty prefix
  reads as a completed removal: the plasmid is not desired, and whichever of its old
  objects still stand surface as drift, named with their owner. A non-empty prefix reads
  as a **partially reloaded plasmid**: desired, with only the effects whose lines were
  written, at the reload's generation, with the last written grant's mock. That is a
  state the system can be in, not an error, and the ledger cannot distinguish it from a
  smaller reload that finished.
- What a partial reload can and cannot cost. Every written-but-unapplied grant is named
  as `missing` drift; every old object not yet undone is named on the snapshot side. The
  grants the crash prevented from being written are different: they produce no drift and
  cannot — write-ahead means an unwritten grant also never touched the world, so there is
  nothing anywhere to reconcile. The loss is capabilities the plasmid was meant to get
  back, never an object owned by nothing: a partial reload under-provisions, and
  under-provision is the failure direction a capability kernel chooses. The party that
  notices is the reload's caller: acknowledgement comes only after every line is flushed
  and every effect applied, so a crashed reload was never acknowledged, and the re-issued
  reload writes a fresh `revoke` and the full grant set at a new generation, converging
  over the partial state.
- **Exactly-once is not guaranteed.** A line carries no transaction identifier beyond its
  generation, and an append can succeed while its acknowledgement is lost — a crash at that
  moment leaves the caller unsure, and a re-issued transaction appends again, under the next
  generation. A caller must tolerate three consequences. A recorded effect that was never
  applied surfaces as `missing` drift. The same world object promised by more than one
  `grant` line collapses in `expected` — an `OsState` is a set keyed by class, key and
  owner — and produces no drift of its own. And any replayer of a cell ledger must treat an
  inverse whose object is already absent as done, never as failure, because a duplicated
  line's inverse runs twice. The retry is never silent: both lines sit in the file, and
  their generations show what happened.
- `Ledger::append_to_file` is not the writer. It writes every effect the in-memory ledger
  holds, so calling it on a ledger rebuilt from disk writes the whole history a second
  time. The crate doc currently recommends exactly that (`open_file` → `push` →
  `append_to_file`); the recipe is wrong for cell ledgers and this spec retires it. A
  per-line append replaces it.

### The strict reader

`Ledger::open_file` is not the recovery reader, on two verified counts. First, it skips any
line that does not parse (`let Ok(record) … else { continue }`), which decision 002 forbids:
a torn write must quarantine, not shorten history. Second, it returns an error when a file
holds records from more than one plugin, while a per-cell ledger holds every plasmid the
cell attached. `open_file` keeps its current single-plugin callers; recovery gets a strict
reader in `plasmosome-ledger`.

The strict reader returns either every line in file order, or a fault carrying the path,
the 1-based line number, and the kind: `Unparseable`, `GenerationZero`,
`GenerationDecreased`, or `Io`. A
missing file and an empty file are not faults; they read as a fresh cell. There is no
partial success: one bad line fails the whole read.

### Recovery

Recovery lives in `plasmosome-core` (which gains a dependency on `plasmosome-ledger`; no
cycle — the ledger crate does not depend on core). Its signature:

```
recover(instance: &InstanceName, instance_root: &Path, snapshot: &OsState)
    -> Result<RecoveryOutcome, RecoveryError>
```

The caller names the instance; recovery does not derive an identity from a path, and a test
passes any valid name over a temporary root. The snapshot comes from `snapshot_os_state` on
the seam.

Discovery draws one line between two failure scopes. A missing `cells/` directory is a
fresh instance: `Ok`, an empty outcome. A `cells/` that exists but cannot be listed is
`Err(RecoveryError::Discovery)` and recovery aborts adopting nothing — a partial listing
cannot name the cells it missed, and a live cell adopted around would become unowned residue
without a word, the failure decision 002 exists to prevent. Every fault scoped to one cell
quarantines that cell and recovery continues: an entry of `cells/` that is not a directory,
or whose name the path function refuses, is quarantined with fault `NotACell`; a ledger the
strict reader faults on is quarantined with that read fault. Instance-wide failure aborts,
per-cell failure quarantines — the blast radius decision 002 chose.

A clean read becomes the cell's desired state by the standing-lines rule. A plasmid is
desired when it has at least one `grant` line after its last `revoke` line. Its desired
effects are those standing grants, in file order; its mode is the last standing grant's
`mock`. A cell whose plasmids were all revoked is still adopted: a `DesiredCell` with empty
`plasmids`, at the generation of the ledger's last line. `genome` is `None` for every
recovered cell: the ledger does not record a genome name and recovery does not invent one.
A reload's partial prefix is one of the states this rule can return — a plasmid desired
with only the effects whose lines were written before a crash. Recovery adopts it as it
stands; what that state can and cannot cost is defined under what the writer guarantees.

`RecoveryOutcome` holds:

- `desired: DesiredState` — one `DesiredCell` per adopted cell, built as above;
  `generation` is the maximum adopted cell generation.
- `expected: OsState` and `drift: Diff` — for each standing grant whose reversibility names
  a universe object (`Exact` via `InverseVia::Universe`, or `Compensating`), the removal's
  class and key plus the line's plugin form an expected `OsObject`.
  `Diff::between(&expected, &snapshot)` then names every object the ledgers promise but the
  snapshot lacks, and every object the snapshot holds but no adopted ledger accounts for.
  One caveat binds the instance-wide union: `OsObject` ownership is a `PluginId` alone, so
  when one plugin is attached to two cells their expected objects collapse into the same
  set entries, and drift between those cells can hide. Exactness of `expected` and `drift`
  across cells that share a plugin waits on the ownership decision in `## Blocked on`; per
  cell, and across cells with disjoint plugins, they are exact.
- `unmatched` — the standing grants recovery cannot verify: an `Exact` inverse via
  `InverseVia::Backend(Handle)` names no class and key, and a handle is process-local, so
  it is dead after a restart. Each entry carries the cell, the plugin, and the effect
  description. A caller must not read their absence from `drift` as verification. `Delayed`
  and `External` effects create no snapshot object and produce neither drift nor an entry
  here.
- `quarantined: Vec<QuarantineReport>` — one per quarantined cell.

### Decision 3 — the quarantine report

`QuarantineReport` carries:

- `instance: InstanceName` — the name the caller passed to `recover` — and `cell: CellId` —
  which cell. For a `NotACell` entry the cell id is the entry's name verbatim, so the
  operator can find the directory even though the path function refuses it.
- `path: PathBuf` and `fault` — where and what. For a read fault, `path` is the ledger
  file, derived by `cell_ledger_path`, and the fault carries its line number, so an
  operator can open the exact line the controller stopped at. For `NotACell`, `path` is
  the raw `cells/` entry as the listing returned it — explicitly not a validated ledger
  path, which a refused name cannot have — carried so the operator can find the entry.
- `lines_parsed: usize` — how many lines read cleanly before the fault. Context only; the
  prefix is not trusted. Zero for `NotACell`.
- `found: Vec<OsObject>` — every snapshot object whose owner is a plugin named by any line
  of the file that did parse, grant or revoke. This is what exists in the world and
  plausibly belongs to the cell; the snapshot does not need the ledger, so the operator
  sees it even though the controller will not act on it. Empty for `NotACell`, which has no
  parsed lines to name a plugin. Attribution is by plugin name, the only ownership
  `OsObject` carries, so a plugin attached to a second cell can bring that cell's objects
  into `found`; the ownership decision in `## Blocked on` settles the ambiguity.
- `refuses: Vec<String>` — the fixed claims the controller declines, stated outright: it
  does not adopt the cell, does not trust the parsed prefix as the cell's history, claims no
  `ledger_generation` for it, and revokes none of the found objects without an operator
  `Force`.

The report implements `Display` in the `ResidueReport` style: a header naming the instance,
the cell and the fault, one `FOUND` line per object via `OsObject::describe()`, one
`REFUSES` line per claim. A caller may rely on: a quarantined cell never appears in
`desired`, contributes nothing to any generation, and its objects are left exactly as the
snapshot found them — so they surface as drift against whoever audits the instance next,
rather than vanishing into an adopted history.

## Contract

- `plasmosome-core::state`: `cell_ledger_path(instance_root: &Path, cell: &CellId) ->
  Result<PathBuf, CellPathError>`. Errors: empty id, path-shaped id (`/`, `\`, NUL, `.`,
  `..`). The only place the literal `ledger.ndjson` appears outside tests.
- `plasmosome-ledger`: `LedgerLine`, one per ndjson line, externally tagged `grant` |
  `revoke`. `grant` carries `plugin: PluginId`, `mock`, `generation: u64`, `effect: Effect`;
  `revoke` carries `plugin: PluginId`, `generation: u64`. All fields required in serde.
  `mock` serializes to exactly `"simulate"`, `"capture"`, or `"passthrough"`; the set is
  closed. A strict read function returning `Result<Vec<LedgerLine>, LedgerReadFault>`;
  `LedgerReadFault { path, line: Option<u64>, kind: Unparseable | GenerationZero |
  GenerationDecreased | Io }`. A per-line append that flushes before returning. Missing or empty file reads as
  `Ok(vec![])`.
- `plasmosome-core::recovery`: `recover(instance: &InstanceName, instance_root: &Path,
  snapshot: &OsState) -> Result<RecoveryOutcome, RecoveryError>`. `RecoveryOutcome {
  desired: DesiredState, expected: OsState, drift: Diff, unmatched: Vec<UnmatchedRecord>,
  quarantined: Vec<QuarantineReport> }`. `RecoveryError::Discovery` carries the path that
  could not be listed and the io error; it is returned only for that instance-wide failure,
  never for a per-cell one. `QuarantineReport.path` is the derived ledger file for a read
  fault and the raw `cells/` entry for `NotACell`. Recovery requires of the backend that an
  object's ownership answers which cell it belongs to; `## Blocked on` carries that
  requirement. `MockMode` moves into `LedgerLine`'s reach without a dependency
  cycle: `plasmosome-ledger` must not depend on `plasmosome-core`, so `MockMode` either
  moves to `plasmosome-backend` or is mirrored by a ledger-owned type with the same three
  wire values; the implementer picks, the vocabulary and the strings stay fixed.
- `plasmosome-core::control`: the value handed to `Controller::new` as `ledger_generation`
  is `RecoveryOutcome::desired.generation` — the maximum adopted cell generation, 0 with no
  adopted cells.
- Callers may rely on: the path function is total over valid ids and refuses invalid ones;
  every acknowledged effect has at least one durable line, written before the effect
  touched the world; a retried transaction may leave duplicate grant lines, visible in the
  file, collapsing to one expected object; generations within a file start at 1 and never
  decrease; a
  removal or reload moves the generation durably via its `revoke` line; a fresh cell reads
  as generation 0; a quarantined cell is absent from `desired`, has no claimed generation,
  and loses no objects; a replayer treats an inverse whose object is already absent as
  done.

## Acceptance

- `cell_ledger_path` exists beside `InstanceName` in `plasmosome-core::state`; a test shows
  `../x`, `a/b`, and the empty id refused with a named error, and a valid id resolving under
  `cells/<cell>/ledger.ndjson` of the given root.
- `git grep -l 'ledger.ndjson' -- crates` names one non-test source file.
- `LedgerLine` round-trips both shapes; a `grant` line missing `mock` or `generation`, and
  a line whose `mock` is any string outside `simulate`/`capture`/`passthrough`, fail the
  strict reader.
- Strict-reading a file whose last line is torn returns a fault carrying that line's number;
  no line set is returned.
- Strict-reading lines with generations `2, 2, 1` returns `GenerationDecreased` naming
  line 3.
- Strict-reading a file whose first line carries `generation: 0` returns `GenerationZero`
  naming line 1.
- Strict-reading a missing path and an empty file both return no lines and no fault.
- The per-line append extends a file holding M lines to exactly M + 1; a test appends, then
  independently reopens the path and strict-reads the new line back.
- A ledger holding a grant at generation 1 and a revoke at generation 2 recovers a cell at
  generation 2 with no desired plasmids and no expected objects.
- The same ledger, over a snapshot still holding the granted object, yields a `drift`
  naming that object as unaccounted for — the mid-reload crash reads as a completed
  removal, loudly.
- A ledger holding a grant at generation 1 for one object, a revoke at generation 2, and
  a grant at generation 2 for a different object — a reload crashed after its first fresh
  grant — recovers the plasmid as desired with exactly the second object in `expected`;
  over a snapshot still holding the first object and lacking the second, `drift` names
  the first as unaccounted for and the second as missing, and nothing else.
- A ledger holding a grant with `mock: simulate` at generation 1, a revoke at generation 2,
  and a grant with `mock: capture` at generation 3 recovers one desired plasmid whose mode
  is `capture`.
- Two grant lines in one cell's ledger promising the same universe object produce one object in `expected` and
  an empty `drift` when the snapshot holds it.
- `recover` over a temp instance root with two cells at generations 3 and 5 returns
  `desired.generation == 5` and both cells in `desired.cells`.
- A snapshot missing one promised object and holding one stray produces a `drift` naming
  both, and nothing else.
- A grant whose inverse is `InverseVia::Backend` appears in `unmatched` and never in
  `expected` or `drift`.
- A cell with one unparseable line is quarantined: absent from `desired`, excluded from the
  generation maximum, and its report names the instance passed to `recover`, the cell, the
  path, the line number, every found object via `describe()`, and each refusal claim; the
  report's `Display` output shows one `FOUND` line per object and one `REFUSES` line per
  claim.
- A `cells/` entry that is a regular file, and one whose name contains `\`, are each
  quarantined with fault `NotACell`, each report carrying the raw entry path, while a
  valid sibling cell is still adopted.
- An instance root where `cells` is a regular file returns `RecoveryError::Discovery` and
  no outcome; an instance root with no `cells/` at all returns `Ok` with an empty outcome.
- A controller constructed from a recovery outcome answers `plasmosome.status` with
  `controller.ledger_generation` equal to the maximum adopted cell generation.
- The `plasmosome-ledger` crate doc no longer recommends `open_file` → `push` →
  `append_to_file`.

## Out of scope

- Surfacing quarantine over the control protocol. Spec 001's cell states and error codes
  are closed sets with no slot for it; adding one is a change to that contract and needs
  its own spec.
- Exactly-once append. It would need a durable transaction identifier in the line format
  and an acknowledgement protocol between writer and caller; the contract above is honest
  instead — at-least-once, write-ahead, retries visible in the file.
- Recovering a cell's genome name. The ledger does not record it; a recovered cell reports
  `genome: null` until something durable carries it.
- A snapshot alongside the log for restart speed. Decision 002 names it as the future fix
  if replay ever makes restart slow; nothing here forecloses it.

## Blocked on

- **A decision on object ownership identity.** `OsObject` is `{class, key, owner:
  PluginId}`, and `plasmosome-backend` has no notion of `CellId`. Recovery requires one of
  two things to be true: an object's ownership names the cell it belongs to, or a plugin
  name belongs to at most one cell of an instance at a time. Neither holds today, and the
  choice between them is one with rejected alternatives someone will argue for again — a
  decision for `docs/decisions/`, not a contract detail for this spec. Until it is made,
  two behaviors above cannot be finished: quarantine `found` for an instance where one
  plugin spans cells, and `expected`/`drift` exactness across cells that share a plugin.
  Everything else is implementable now, single-cell and disjoint-plugin cases included.
- Nothing else. Every other behavior runs against `FakeBackend`, `tempfile` directories,
  and the existing crates on a Mac, with no VM and no running cell. One rule is testable
  only later rather than blocked: the write-ahead order binds the transaction writer
  inside the daemon, and no such writer exists yet. It is stated now so the first one is
  built against it, not discovered against it.
