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
stamped on every ledger record, so the file itself is its durable home; and the quarantine
report names the cell, the fault, what the snapshot shows, and exactly what the controller
refuses to claim.

The instance root is not new. Spec 001 §1 already fixes it as the directory holding
`control.uds` (`~/.plasmosome/instances/<name>/` in production), and §4 already places each
cell's `membrane.uds` in `cells/<cell>/`. This spec adds one filename inside that existing
per-cell directory and nothing else. The root always arrives as an argument — tests pass a
temporary directory — and the cell id is validated before it becomes a path component,
because `CellId` today accepts any string, including path-shaped ones.

Recovery reads that layout end to end. It lists the subdirectories of `cells/`, derives each
cell's ledger path, and reads the file with a strict reader: every line must parse, and
generations must never decrease. A clean read yields the cell's desired state — its plasmids,
their granted effects, its generation. Any fault quarantines the cell. The desired states of
the adopted cells become one `DesiredState`, the expected operating-system objects are
rebuilt from the records' inverses, and `Diff::between` against the backend snapshot names
every difference. The controller adopts what reconciles and reports what it refuses.

### Decision 1 — where a cell's ledger lives

`<instance-root>/cells/<cell>/ledger.ndjson`.

One function derives it, and it lives in `plasmosome-core::state` beside the `InstanceName`
parser. It takes the instance root as a `&Path` and the `CellId`, and returns the path or an
error. It applies the same rules `InstanceName::parse` applies: the cell id must be
non-empty, contain no `/`, `\` or NUL, and must not be `.` or `..`. A cell id that fails is
refused with a named error, never sanitized. No other code builds this path; the writer and
the recovery reader both call this function.

The file format is the one `plasmosome-ledger` already writes: ndjson, one `LogRecord` per
line, in grant order.

### Decision 2 — where `ledger_generation` is durably kept

In the ledger itself. `LogRecord` grows two required fields:

```json
{"plugin": "github-pr", "mock": "simulate", "generation": 3, "effect": {"...": "..."}}
```

- `generation: u64` — the cell's generation at the moment the record was appended. A
  mutating transaction on a cell (an attach, a reload, a removal batch) increments the
  cell's generation once and stamps every record it appends with the new value. Within a
  file, generations never decrease; a decrease is a fault and quarantines the cell.
- `mock: MockMode` — the plasmid's mock mode at grant time. Without it a recovered cell
  would have to claim a mode it cannot know, and claiming `passthrough` for a simulated
  plasmid is a claim in the unsafe direction.

A cell's `ledger_generation` is the generation of the last record in its ledger. A cell with
no ledger file, or an empty one, is a fresh cell: generation 0, no granted effects, still
adopted. A quarantined cell has no generation; the controller does not claim one.

Spec 001 §3.3 reports one `ledger_generation` under `controller`. That number is defined
here as the maximum generation across the adopted cells, and 0 when there are none. This is
the value passed to `Controller::new`, whose doc already says the caller supplies it, and it
is the `generation` of the `DesiredState` recovery hands to the `Reconciler`. A per-cell
membrane push (`membrane.cell.desired`, spec 001 §4) carries that cell's own generation.

Both fields are required. The recovery reader does not default a missing one; a line without
them does not parse, and an unparseable line quarantines the cell. Nothing durable exists in
the old shape — no production writer of per-cell ledgers exists before this spec — so there
is no history to stay readable for.

The writer appends exactly one line per newly granted effect. `Ledger::append_to_file`
writes every effect the in-memory ledger holds, so calling it on a ledger rebuilt from disk
writes the whole history a second time, and a doubled history double-undoes on replay. The
crate doc currently recommends exactly that (`open_file` → `push` → `append_to_file`); the
recipe is wrong and this spec retires it. A per-record append replaces it for cell ledgers,
and the invariant a caller may rely on is: each granted effect appears in the file exactly
once, in grant order.

### The strict reader

`Ledger::open_file` is not the recovery reader, on two verified counts. First, it skips any
line that does not parse (`let Ok(record) … else { continue }`), which decision 002 forbids:
a torn write must quarantine, not shorten history. Second, it returns an error when a file
holds records from more than one plugin, while a per-cell ledger holds every plasmid the
cell attached. `open_file` keeps its current single-plugin callers; recovery gets a strict
reader in `plasmosome-ledger`.

The strict reader returns either every record in file order, or a fault carrying the path,
the 1-based line number, and the kind: `Unparseable`, `GenerationDecreased`, or `Io`. A
missing file and an empty file are not faults; they read as a fresh cell. There is no
partial success: one bad line fails the whole read.

### Recovery

Recovery lives in `plasmosome-core` (which gains a dependency on `plasmosome-ledger`; no
cycle — the ledger crate does not depend on core). It takes the instance root and the
backend snapshot (`OsState`, from `snapshot_os_state` on the seam), and returns an outcome
holding:

- `desired: DesiredState` — one `DesiredCell` per adopted cell. Its `plasmids` are the
  distinct plugins in the cell's ledger, each with its recorded mock mode. Its `genome` is
  `None`: the ledger does not record a genome name and recovery does not invent one.
- `expected: OsState` and `drift: Diff` — for each adopted record whose reversibility names
  a universe object (`Exact` via `InverseVia::Universe`, or `Compensating`), the removal's
  class and key plus the record's plugin form an expected `OsObject`.
  `Diff::between(&expected, &snapshot)` then names every object the ledgers promise but the
  snapshot lacks, and every object the snapshot holds but no adopted ledger accounts for.
- `unmatched` — the adopted records recovery cannot verify: an `Exact` inverse via
  `InverseVia::Backend(Handle)` names no class and key, and a handle is process-local, so
  it is dead after a restart. Each entry carries the cell, the plugin, and the effect
  description. A caller must not read their absence from `drift` as verification. `Delayed`
  and `External` effects create no snapshot object and produce neither drift nor an entry
  here.
- `quarantined: Vec<QuarantineReport>` — one per cell whose read faulted.

### Decision 3 — the quarantine report

`QuarantineReport` carries:

- `instance: InstanceName` and `cell: CellId` — which cell.
- `ledger: PathBuf` and `fault` — the file, the line number, and the fault kind, so an
  operator can open the exact line the controller stopped at.
- `records_parsed: usize` — how many lines read cleanly before the fault. Context only; the
  prefix is not trusted.
- `found: Vec<OsObject>` — every snapshot object whose owner is a plugin named by any line
  of the file that did parse. This is what exists in the world and plausibly belongs to the
  cell; the snapshot does not need the ledger, so the operator sees it even though the
  controller will not act on it.
- `refuses: Vec<String>` — the fixed claims the controller declines, stated outright: it
  does not adopt the cell, does not trust the parsed prefix as the cell's history, claims no
  `ledger_generation` for it, and revokes none of the found objects without an operator
  `Force`.

The report implements `Display` in the `ResidueReport` style: a header naming the cell and
the fault, one `FOUND` line per object via `OsObject::describe()`, one `REFUSES` line per
claim. A caller may rely on: a quarantined cell never appears in `desired`, contributes
nothing to any generation, and its objects are left exactly as the snapshot found them — so
they surface as drift against whoever audits the instance next, rather than vanishing into
an adopted history.

## Contract

- `plasmosome-core::state`: `cell_ledger_path(instance_root: &Path, cell: &CellId) ->
  Result<PathBuf, CellPathError>`. Errors: empty id, path-shaped id (`/`, `\`, NUL, `.`,
  `..`). The only place the literal `ledger.ndjson` appears outside tests.
- `plasmosome-ledger`: `LogRecord { plugin: PluginId, mock: MockMode, generation: u64,
  effect: Effect }`, all fields required in serde. A strict read function returning
  `Result<Vec<LogRecord>, LedgerReadFault>`; `LedgerReadFault { path, line: Option<u64>,
  kind: Unparseable | GenerationDecreased | Io }`. A per-record append that writes one line
  per new effect. Missing or empty file reads as `Ok(vec![])`.
- `plasmosome-core::recovery`: `recover(instance_root: &Path, snapshot: &OsState) ->
  RecoveryOutcome { desired: DesiredState, expected: OsState, drift: Diff, unmatched:
  Vec<UnmatchedRecord>, quarantined: Vec<QuarantineReport> }`. `MockMode` moves into
  `LogRecord`'s reach without a dependency cycle: `plasmosome-ledger` must not depend on
  `plasmosome-core`, so `MockMode` either moves to `plasmosome-backend` or is mirrored by a
  ledger-owned type with the same three closed values; the implementer picks, the
  vocabulary stays closed.
- `plasmosome-core::control`: the value handed to `Controller::new` as `ledger_generation`
  is `RecoveryOutcome::desired.generation` — the maximum adopted cell generation, 0 with no
  adopted cells.
- Callers may rely on: the path function is total over valid ids and refuses invalid ones;
  each granted effect is on disk exactly once, in grant order, before the grant is reported
  done; generations within a file never decrease; a fresh cell reads as generation 0; a
  quarantined cell is absent from `desired`, has no claimed generation, and loses no
  objects.

## Acceptance

- `cell_ledger_path` exists beside `InstanceName` in `plasmosome-core::state`; a test shows
  `../x`, `a/b`, and the empty id refused with a named error, and a valid id resolving under
  `cells/<cell>/ledger.ndjson` of the given root.
- `git grep -l 'ledger.ndjson' -- crates` names one non-test source file.
- `LogRecord` serializes with `plugin`, `mock`, `generation`, and `effect`; a line missing
  `generation` or `mock` fails to parse in the strict reader.
- Strict-reading a file whose last line is torn returns a fault carrying that line's number;
  no record set is returned.
- Strict-reading records with generations `2, 2, 1` returns `GenerationDecreased` naming
  line 3.
- Strict-reading a missing path and an empty file both return no records and no fault.
- A per-record append extends a file holding M records to exactly M + N lines after N new
  effects; a test re-reads and sees each effect once.
- `recover` over a temp instance root with two cells at generations 3 and 5 returns
  `desired.generation == 5` and both cells in `desired.cells`.
- A snapshot missing one promised object and holding one stray produces a `drift` naming
  both, and nothing else.
- A record whose inverse is `InverseVia::Backend` appears in `unmatched` and never in
  `expected` or `drift`.
- A cell with one unparseable line is quarantined: absent from `desired`, excluded from the
  generation maximum, and its report names the instance, the cell, the path, the line
  number, every found object via `describe()`, and each refusal claim; the report's
  `Display` output shows one `FOUND` line per object and one `REFUSES` line per claim.
- A controller constructed from a recovery outcome answers `plasmosome.status` with
  `controller.ledger_generation` equal to the maximum adopted cell generation.
- The `plasmosome-ledger` crate doc no longer recommends `open_file` → `push` →
  `append_to_file`.

## Out of scope

- Surfacing quarantine over the control protocol. Spec 001's cell states and error codes
  are closed sets with no slot for it; adding one is a change to that contract and needs
  its own spec.
- Recovering a cell's genome name. The ledger does not record it; a recovered cell reports
  `genome: null` until something durable carries it.
- A snapshot alongside the log for restart speed. Decision 002 names it as the future fix
  if replay ever makes restart slow; nothing here forecloses it.

## Blocked on

Nothing. Every behavior above runs against `FakeBackend`, `tempfile` directories, and the
existing crates on a Mac, with no VM and no running cell.
