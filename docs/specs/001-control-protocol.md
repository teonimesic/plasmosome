---
id: 001
title: Control protocol
status: accepted
intents: [003, 004, 009, 012]
---

# Plasmosome control protocol — the P1 contract, as delivered

**Scope:** the P1 control contract (91 plan step 1). Everything binding below is
traceable to a decided item: D1/D1a/D1b/D1c/D2/D2b/D3/D4 in `91-p1-plan.md`, the credential
grammar in `80-adr-credential-delivery.md` (D2 as confirmed by E4b, `[commands]` as settled by
E13/E13b), the six must-not-bake-in rules in `86-kernel-process-topology.md` §4, and
F9's measured readiness rule. Items nobody has decided are marked RESERVED and are not decided
here.

**This file records what was delivered. It is not a text nobody may edit.** Every shape below
binds each client until this file changes, and it changes the way anything else here does: on a
branch, in a pull request, with the reasoning written down. So a shape is the shape a client
gets today, and altering one is a change to the contract — announced, never slipped in. What it
is not is permanent. A design here that turns out wrong is rewritten in place; correcting it
needs no amendment layered on top, and the history is in the log. §6 is the record of how much
of this is built.

The protocol is the controller's (`plasmosomed`) **only** control surface. The CLI (`plasmosome`
/ `plasmid` binaries), the future MCP server (D1: a later transposition of the same verbs), and
any test harness are all clients of this one socket. Every response is machine-legible: typed
ids, typed states, structured errors — v1 ships a human-typed client, but the shapes are
agent-consumable from day one.

## 1. Transport and envelope

One Unix domain socket per named instance:

```
~/.plasmosome/instances/<name>/control.uds
```

Framing is **ndjson**: one JSON request per line, one JSON response per line, in request order
per connection. This is the house protocol (`ak-policy` control RPC): the same shape the
brokers already speak, and the same shape the session log is written in.

**Connection edges.**

- A request line is at most 1,048,576 bytes before its terminating newline — every byte counts,
  a carriage return included. A longer line is answered `-32600` under a `null` id, and the
  connection then closes.
- A line that is not UTF-8 is not JSON: it is answered `-32700` under a `null` id, and the
  conversation continues.
- A request the controller fails on internally — a crash while answering — is answered
  `-32603`, and the connection then closes.
- A reply carrying `-32700` or `-32600` always comes from the connection loop itself, never
  from a verb implementation. A verb that answers with either is replaced by `-32603`, and the
  conversation continues.
- **A reply never says whether the connection is about to close.** `-32600` and `-32603` each
  cover one closing case and one continuing case, and a client must not branch on `message`.
  End of input is the only signal that the conversation is over.
- A response field with nothing in it is omitted, never sent as `null`. A cell with no genome
  has no `genome` key (§3.3, §3.6). An empty collection is not such a field: it is sent as `[]`,
  or `{}` for a map, unless its own text says when it is present. Spec 011's per-plasmid
  `denials` (§3.6, §3.9) does: it is omitted when it is empty. Amended on 2026-10-20: this did
  not say whether an empty collection is sent, and spec 011 read it as omitted.

Request envelope:

```json
{"id": 7, "method": "cell.status", "params": {"kernel": "work", "cell": "cell-1"}}
```

- `id` — client-chosen, echoed verbatim in the response. Any JSON value.
- `method` — `<noun>.<verb>`, dotted, lowercase.
- `params` — always an object (never omitted; empty object when the verb takes nothing). A verb
  of §3 refuses a key it does not define with `-32602`. Amended on 2026-10-20: this did not say
  what a verb does with a key it does not define.

Response envelope, success:

```json
{"id": 7, "result": {"ready": true, "state": "serving", "cells": []}}
```

Response envelope, failure:

```json
{"id": 7, "error": {"code": 100, "message": "`cell` is ambiguous: 2 running cells match",
                    "candidates": ["cell-1", "cell-2"]}}
```

Every error carries a **closed integer `code`** (below), a human `message`, and — where the
failure is about *selection* or *resolution* — structured extra fields (`candidates`,
`resolutions`, …). A client must never parse `message` to branch; the code and fields are the
contract. Protocol-level failures reuse the JSON-RPC reserve: `-32700` parse error, `-32600`
invalid request, `-32601` method not found, `-32602` invalid params, `-32603` internal error —
the controller, not the request, failed.

Application error codes (closed set; additions are a contract change):

| code | name | structured fields |
| --- | --- | --- |
| 100 | `ambiguous_target` | `candidates` — the matching ids, per D1b's ambiguity-is-an-error |
| 101 | `unknown_target` | `target` — the noun+name asked for |
| 102 | `already_exists` / `already_attached` | `target` |
| 103 | `unresolved_requirement` | `capability`, `plasmid` |
| 104 | `mock_mode_conflict` | `node`, `modes`, `plasmids`, `resolutions` — per D2b rule 3, or per spec 022 when a mutation would change an attached plasmid's mode or serve one host, port and transport at two modes in a cell; spec 022's cases name one plasmid as `node` and offer only `remove_plasmid` |
| 105 | `illegal_state` | `from`, `to`; `detail`, a record of a kind spec 001 defines, including spec 024's; `cell` whenever the refusal concerns a created cell (spec 023); private recovery methods additionally carry the typed `recovery` refusal in §4.1 |
| 106 | `drain_timeout` | `handle`, `deadline_ms` |
| 107 | `not_running` | `target` — the named instance is not up |
| 108 | `manifest_invalid` | `detail`, `path`; `field` and `fix` when the refusal names a declaration field; `plasmid` when the refusal names the declaration's id |
| 109 | `widening_forbidden` | `plasmid` |
| 110 | `attestation_required` | `verb` — the E13b residual: subject spawn needs host-side attestation |

The 108 `field`, `fix` and `plasmid` are spec 011's amendment to this table. `field` is the
declaration field the refusal names, as its path: TOML keys joined by dots, each bare where TOML
allows and otherwise a quoted string, with `[i]` after a list to name its entry `i`, such as
`network.ports[1]`, `commands.commands."git ops".exec` or `secrets.refs[0].scope.path_scope`. A
108 carries `field` and `fix` together or not at all. `plasmid` is the declaration's `id` itself,
not quoted or escaped as `detail` writes it, and a 108 carries it exactly when the refusal names
that id. Today no refusal of the `id` field names one, because no id parsed: the id is missing,
not a string or empty. Such a refusal carries `field` `id` and its `fix`, and no `plasmid`. A
client reads these members and never splits `detail`: a key or an id may hold `: `.
Amended on 2026-10-21: the first sentence here read "The 108 `fix` is spec 011's amendment to
this table.", and row 108's fields read "`detail`, `path`; `fix` when the refusal names a
declaration field"; the field and the plasmid id travelled only inside `detail`. A 108 whose
refusal names a declaration field carries a `fix` in one of three forms:

- TOML to write in place of that field: a line such as `hosts = ["api.github.com"]`, or a
  section such as `[impl]` followed by `wasm = "component.wasm"`;
- a bare entry, such as `"api.github.example"`, added to the list the field names, or written in
  place of the entry when the field ends in an index, such as `network.pin_cidrs[0]`;
- exactly `remove this entry`, which deletes the entry or key the field names.

One fix writes the field's parent instead: a credential whose `scope.path_scope` is refused gets
`scope = { path_scope = ["/required/path/"] }`, written in place of `scope`. A fix applies when
the field's parents are tables, not inline tables: a section such as
`[commands.commands.git.network]` cannot be added to a command written as
`git = { exec = ["git"], network = 443 }`, and TOML refuses the result.

Exactly three refusals an author can cause name no declaration field and carry no `fix`:

1. a TOML syntax error, which the parser reports by line and column;
2. a declaration over 1 MiB, the maximum accepted size spec 020 §2 gives each declaration;
3. a declaration nested more than 32 levels deep, counted as spec 011 says. Amended on
   2026-10-20: this list had two items before the manifest's depth bound.

An unknown key is not one of them. Every other refusal an author can cause carries a field and a
`fix`. A 108 the author did not cause carries no `fix` either: a file that cannot be read, or a
fault in the file rather than in what it declares, such as spec 022's trust checks.

**Control-socket lifecycle, as delivered (both daemons).** Each daemon binds the control socket
named by its config and never unlinks a path it did not create. A start whose path already
exists — live socket, stale socket, regular file, directory or symlink — is refused without
touching it: clearing the path is the caller's job, because unlinking a live daemon's socket is
how a half-alive controller or supervisor is manufactured. Following a successful bind, the
daemon records the socket's device and inode; on every route out (clean return, later error,
unwinding panic) it removes the socket's name only if a no-follow inspection finds that exact
socket still there, as one best-effort attempt. A replacement settled at the pathname after
start — a regular file or a leaf symlink, target present or dangling — is preserved with its
contents, entry type and link target, and an original socket renamed elsewhere stays there: the
daemon owns the entry it bound, not every name later given to its inode. A missing entry, a
non-socket, or a non-matching identity leaves the namespace untouched. A start whose identity
capture fails refuses before any fork or other setup, without unlinking anything, and can leave
the just-bound socket for explicit operator cleanup. `membraned` captures that identity at the
configured path and inspects the configured path at teardown. `plasmosomed` does both at the
socket's name in the directory it opened and checked at start, which it holds open: it captures
the identity only of a socket its effective UID owns, and if that directory is renamed while it
runs, teardown removes the socket under the directory's new name and leaves whatever is then at
the configured path. A `plasmosomed` start that fails at any step after the capture removes the
just-bound socket as any later error does. Those steps are, in order, the repeated path check, the
change to mode 0600, the re-inspection of the socket after that change, `listen`, and the switch
to nonblocking. Amended on 2026-10-20: this paragraph said each daemon removes the configured
path, and described only `membraned`'s capture and teardown.

These are host-conditioned guarantees, not race-free protection. The caller supplies a trusted,
stable directory and coordinated namespace writers: parent, ancestors and their symlink targets
stay put for the daemon's lifetime, a private 0700 directory is the recommended setup, and none
of that defends against another process with write authority over the directory — the same UID
included. The final inspect-then-unlink pair is two steps, not one atomic one. Device+inode
distinguishes a later socket at the same name only while the original inode stays allocated;
the file-type check still rejects regular files and symlinks even if inode numbers are recycled.
`SIGKILL` and equivalent hard exits run no destructor; the resulting residue is observed by the
next start's refusal, not prevented, and no verb observes or repairs it.

`plasmosomed`'s control socket is held to a stricter rule: it is bound under §4.1's path
boundary for private sockets. The daemon refuses to start when the configured path fails that
boundary, sets the socket to mode 0600 before listening, and closes unread any connection from
another effective UID (spec 025 §5 and §6). This does not change `membraned`'s sockets.

## 2. Naming and addressing (D1a/D1b/D1c)

- A **kernel instance** has a name (`plasmosome start --name work`); its state lives at
  `~/.plasmosome/instances/<name>/` (D1b). Verbs in the `plasmosome.*` group address the kernel
  organism; verbs in the `plasmid.*` group address plugin attachments. **No plasmid verb can
  start the plasmosome** (D1a): if the addressed instance is not running, `plasmid.*` fails and
  never boots one. Each instance has its own socket, so the command line notices a down instance
  itself and exits 1 with client code `no_instance`, `not_running` or `unreachable` (spec 025
  §4). Code `107` is for a service that answers for an instance it does not serve, which only a
  host-level service could be; it also exits 1. Amended on 2026-10-21 (spec 025, amendment A):
  this said "if the addressed instance is not running, `plasmid.*` fails with code `107`, it
  never boots one."
- A **cell** is a running session: one plasmosome + its attached plasmids + the workload (D1c).
  Its declarative definition is a **genome**, `~/.plasmosome`-relative to the project:
  `.plasmosome/genomes/<name>.toml`; the share/export form is `*.genome.toml`. That file is not yet
  a selection source: until a local genome grammar exists, `cell.new` selects a genome only by
  `artifact` (spec 022).
- Addressing chains down `(kernel, cell, plasmid)`. `--kernel` / `--cell` are optional **only
  when unambiguous**. The server resolves a cell: one matching cell is the default, otherwise
  code `100` with the candidate list. Each instance has its own socket, so no server can resolve
  an instance; the command line does, under spec 025 §5. Exactly one instance is the default, and
  more than one, reachable or not, is refused with the candidate list. Neither side guesses. The
  results of `plasmid.list`, `plasmid.add`, `plasmid.remove`, `plasmid.reload` and
  `plasmid.renew` (spec 028 §8) each carry `cell`, the cell the verb acted on, whether the request
  named it or the server resolved it. Amended on 2026-10-20: this did not name `plasmid.renew`.
  Amended on 2026-10-21 (spec 025, amendment A): this said "One running instance → default;
  otherwise code `100` with the candidate list. The server resolves; the client never guesses."

## 3. Verb schemas (the v1 set)

### 3.1 `plasmosome.start`

Idempotent on a running instance with an identical genome set (replies with the existing
record; `started: false`).

```json
{"id": 1, "method": "plasmosome.start",
 "params": {"name": "work", "root": "/path/to/project"}}
```

```json
{"id": 1, "result": {"name": "work", "started": true, "socket": "~/.plasmosome/instances/work/control.uds",
                     "pid": 42117}}
```

### 3.2 `plasmosome.list`

```json
{"id": 2, "method": "plasmosome.list", "params": {}}
```

```json
{"id": 2, "result": {"instances": [
  {"name": "work", "state": "running", "cells": 2, "plasmids": 3,
   "socket": "~/.plasmosome/instances/work/control.uds"}
]}}
```

`state` ∈ `running | stopped | unreachable` (registered but its socket is dead — F9: this is
observed by probing control-`status`, not by reading a pidfile).

### 3.3 `plasmosome.status`

```json
{"id": 3, "method": "plasmosome.status", "params": {"name": "work"}}
```

```json
{"id": 3, "result": {"name": "work", "state": "running", "ready": true,
  "controller": {"uptime_ms": 9142, "ledger_generation": 4},
  "cells": [
    {"id": "cell-1", "genome": "researcher", "state": "ready",
     "plasmids": ["github-pr [mock:simulate]", "workspace [real]"]},
    {"id": "cell-2", "genome": "researcher", "state": "draining",
     "plasmids": []}
  ]}}
```

- Cell `state` ∈ `germinating | ready | draining | dead` (D1c cell lifecycle; mirrors the
  plasmid FSM vocabulary already ported in `plasmosome-core::lifecycle`).
- Plasmid labels carry the D2 mock mode: `[mock:simulate]`, `[mock:capture]`, or `[real]` —
  `plasmid list`/status always shows the mode per plasmid (D2).

Spec008 defines recovered `ledger_generation` as the maximum adopted cell generation, or 0.
It is not a cross-cell acknowledgement watermark. Recovered cells retain their actual observed
supervisor state; replay does not prove readiness. After successful recovery startup, `ready` is
false while the account contains drift, quarantine, a set-aside cell, unmatched effects or any
incomplete effect, while a cell's publication is unsettled (spec 008), or while a spec 028 expiry is
unfinished past its H. Quarantined and set-aside cells are absent from `cells` and remain named in
`plasmosome.recovery`. Amended on 2026-10-21: this did not name set-aside cells, which spec 023 §8
keeps out of `cells` with readiness false. A recovered unknown genome is omitted, as §1 requires.

### 3.3a `plasmosome.recovery`

Read-only diagnostics for the most recent completed recovery observation:

```json
{"id": 30, "method": "plasmosome.recovery", "params": {"name": "work", "deadline_ms": 5000, "snapshot": 0, "offset": 0}}
```

The wire result is §4.1's `ObservationPage`; request IDs are unsigned64-bit integers and
`deadline_ms`, `snapshot` and `offset` are required. Zero/zero pins the most recent completed
RecoveryOutcome for this resolved instance, not a fresh observation. All pages retain that same
outcome even if a newer observation completes. The positive deadline is capped by the configured
recovery_deadline_ms and bounds the entire transfer; later requests cannot renew it.
Connection, method, resolved instance and original controller lifetime bind the capture.
Apply the same bounded retention, exact-page retry, complete assembly, length/hash validation
and failure rules as internal observations. A supported client presents only the fully validated
logical account, never pages or a prefix as a complete recovery diagnostic.

The assembled logical result is `{name, ledger_generation, desired, tombstones, observed_cells,
expected, drift, unmatched, incomplete, pending, quarantined, set_aside}`. `set_aside` is the
required array of the cells spec 023's supervisor check set aside, in byte order of the cell ID
string (so `cell-10` sorts before `cell-9`), and `[]` when there is none. Each entry is `{cell,
reason, lock, marker, desired, pending}`. `reason` is one of `unresponsive`, `germinating`,
`lock_untestable`, `external_obligations`, `supervisor_lost`, `holdings_without_supervisor`,
`retire_unlogged` and `retire_failed`. `lock` is `held`, `free` or `untestable`, the last for a
`supervisor.lock` spec 023's check cannot test. `marker` is required whatever the lock, since the
check reads `launched` for every cell before it acts: `absent`, `earlier_boot`, `this_boot_stopped`,
`this_boot` or `unreadable`, for no marker, one from an earlier boot, one from this boot that
records a clean stop, one from this boot that records none, or one that cannot be read. `desired` is
the cell's settled DesiredCell from its journal, in this section's projection, so each attachment
shows its spec 028 `lifetime`; an empty journal gives the empty generation-0 record. `pending` is
the cell's pending transaction in this section's projection, omitted when it has none. Both are
omitted when the cell's journal is corrupt, which happens only when the lock is held or untestable,
since a free lock with a corrupt journal quarantines; spec 008 then reports that journal under
`quarantined`. Amended on 2026-10-21: `reason` had no `lock_untestable` and `lock` no `untestable`,
so spec 023's row for a lock its check cannot test had no entry here. A
cell appears in `set_aside` once. A set-aside cell's records appear only in its entry, apart from a
corrupt journal's report under `quarantined`, and never in the top-level `desired`,
`observed_cells`, `pending`, `expected`, `drift`, `unmatched`, `incomplete` or `tombstones`. Its
tombstones stay in its journal and are not reported until it is adopted. `set_aside` belongs to the
outcome, like every other key: it names the cells set aside when that outcome completed. A
supervisor check while serving that sets a cell aside, changes its entry, quarantines a cell, or
adopts or retires a set-aside cell, completes a new outcome in which only that cell's records change
and `ledger_generation` is recomputed over the adopted cells. So a new capture shows the current
list, and a capture already pinned keeps its outcome. `set_aside` is not added to §4.1's stderr
diagnostic, since a set-aside cell never refuses startup; the quarantine reports in that diagnostic
carry `lock` and `marker` when spec 023's check quarantined their cell (spec 008). `incomplete` is
the required `[IncompleteEffect, ...]`, preserving every complete operation and its original
cell/plugin owner; it is not coerced into drift, unmatched or an OsObject. `desired`, exact objects,
pending transactions and quarantine reports use spec008's typed records with this wire projection:
absent genome/cell/line, operation and force fields are omitted; a journal change whose replacement
is null becomes `{plugin, remove: true}`, while a replacement change is `{plugin, replacement:
...}`. These are response encodings, not changes to the required nullable journal fields.
`drift` retains `Diff { added, removed }`: added objects were observed but not desired; removed
objects were desired but absent. IDs, complete capabilities and cell/plugin owners are
preserved. This is not a canonical-equivalence comparison. Empty collections are `[]` or their
empty map, not an omitted failure. Quarantine raw names and paths are arrays of unsigned byte
values; human messages escape them and are not machine selectors. The same optional-field
projection applies to the DesiredCell sent to a membrane.
The required spec008 AttachmentSource is retained in every replacement and DesiredPlasmid,
including its full registry/root/member references; it is never omitted as incidental metadata.

The call neither forces cleanup nor claims to refresh a failed observation. Name resolution
and wrong-name errors are the same as `plasmosome.status`. Recovery startup with incomplete
observation or blocked cleanup does not serve this or any other method: it exits with the
structured stderr diagnostic described in §4.1. Quarantine alone can coexist with a serving
controller when all live observations are complete; readiness remains false. A cell that spec 023's
supervisor check sets aside is handled the same way: it is not an incomplete observation, it
coexists with a serving controller, and readiness remains false.
A complete observation containing unrequested incomplete effects may serve this diagnostic,
with readiness false. That does not turn unresolved required startup cleanup into success.

### 3.4 `plasmosome.stop`

Graceful by default: drains every cell (detach cascades, ledger replays LIFO), then stops the
controller. `force: true` is the authority-class immediate stop (RevokePolicy::Force) and
requires an `operator` + `reason` assertion pair, recorded in the session log.

```json
{"id": 4, "method": "plasmosome.stop", "params": {"name": "work"}}
{"id": 4, "method": "plasmosome.stop",
 "params": {"name": "work", "force": true, "operator": "stefano", "reason": "rotation window"}}
```

```json
{"id": 4, "result": {"name": "work", "state": "stopped",
  "drained": ["cell-1", "cell-2"], "forced": [], "residue": "empty"}}
```

`residue` ∈ `empty | items` — the D4 standing row: the post-stop verification over the F9
universe (five host classes + guest classes), **observed off the wire from the membrane/broker
side**, never from controller memory (86 §4 rule 4). Non-empty residue is reported as
`"residue": "items"` plus a `residue_items` array of the named leaked/lost/asserted objects
(the `ResidueReport` shape already serde-typed in `plasmosome-backend`).
Spec017 gives each residue object an exact grant ID and complete capability; spec008 widens
its owner to `{cell, plugin}`. These embedded wire changes do not change the residue variants
or authorize removing a different holding with an equal resource description.

**Parameters.** `name` is optional, as for `plasmosome.status`: a different name is 101 with
`target` `plasmosome <name>`, and nothing stops. `force` is optional. `operator` and `reason` are
nonblank strings, required when `force` is true and refused with `-32602` when `force` is not
true. `deadline_ms` is an optional positive integer that bounds the whole stop from the request's
arrival; its default is `cell_runtime.stop_deadline_ms`, or 30,000 without `cell_runtime`.

**Order.** From the moment a stop is admitted until it replies, every other request that would
change state is refused at once with 105 `{from: "stopping", to: "running"}`, a second
`plasmosome.stop` included. `cell.exec` is refused the same way. This 105 carries no `cell`, at
admission or at a mutation's turn: it concerns the instance, not a cell. Reads are answered. The
stop then takes the cells one at a time, in byte order of the cell ID (so `cell-10` comes before
`cell-9`), each in its place in that cell's arrival order behind the mutations admitted before it
(spec 023 §6). A cell is `draining` while the stop works on it.

**Drain.** A cell's drain is spec 023 §7's step 2: a committed but unfinished transaction of the
cell is resumed first, then every attachment is withdrawn in one removal transaction under spec
008, `Graceful`, or `Force` when `force` is true, with the `force` event spec 008 requires before
the first forced withdrawal. Under `force` too, a resumed transaction that does not finish in time
is 106: the stop never takes step 2's `now` path, which goes on to stop the guest. No
`membrane.workload.stop` and no `membrane.cell.kill` is sent. The cell keeps running with no
attachment after the controller exits, and a later startup adopts it (spec 008). This is spec 023
open question 5's default; that question stays open.

**Residue.** After its drain, each cell's `membrane.residue.snapshot` is diffed against its
settled DesiredCell. `residue` is `empty` only when the diff of every cell in `drained` and `forced`
is empty; otherwise `residue_items` holds the named leaked, lost and asserted objects of every
cell's report, cell by cell in that order. Residue `items` does not keep the controller running: the
cells that hold it keep running, and the next startup observes them.

`drained` lists, in the order above, each cell whose removal ran `Graceful` or that had nothing
to withdraw. `forced` lists each cell whose removal ran under `Force`. So a graceful stop always
has `forced: []`.

**Refusals.** A cell that spec 023's check set aside, or that spec 008 quarantined, cannot be
drained, so the stop refuses before draining anything, `force` included, with 105 `{from:
"set_aside" | "quarantined", to: "stopped", cell}` naming the first such cell in the order above;
`plasmosome.recovery` names the rest. A cell set aside or quarantined after the stop was admitted
refuses it the same way at its turn, after the cells before it have drained. A cell retired before
its turn, by a `cell.kill` admitted earlier, is skipped and is in neither `drained` nor `forced`. A
drain or publication that does not settle within the deadline is 106 `{handle, deadline_ms}`, with
`handle` the cell's ID and `deadline_ms` the stop's deadline. So is a residue snapshot that fails or
does not answer in time, since the stop cannot then say what the cell holds. So is a `force` event
that cannot be appended; nothing is forced on that cell (spec 008). Outstanding external effects
without `force` are §3.11's 105, with `cell`, found for every cell before draining anything, and
again at each cell's turn. A request that changes state checks the gate again once it has its turn
on a cell, so a mutation admitted just before the stop cannot run on a cell the stop has drained. A
`cell.new` checks the gate again under spec 023 §2's allocation mutex, before it creates a cell
directory, and the stop reads its list of cells under that mutex once the gate is closed, so no cell
is created that the stop does not take. After any refusal the controller keeps serving: cells
already drained stay drained, a cell whose drain or snapshot ended in 106 stays `draining` for a
later `plasmosome.stop` or `cell.kill`, every other cell is in the state it had before the stop, and
mutations are admitted again.

**After the reply.** Once the reply is written, the controller accepts no new connection, reads
no further request on any connection, removes its socket as §1 says, releases `controller.lock`
and exits 0. The instance's directory stays, with no socket entry, which spec 025 §5 reads as a
stopped instance. A reply that does not reach the client does not undo the stop. SIGTERM or
SIGINT ends the controller without draining, as today: its cells keep their attachments, and the
next startup recovers them.

Amended on 2026-10-21: this section gave the request and the result but no deadline, no order
against other mutations, no refusal, and no account of what the controller does after it
replies.

### 3.5 `plasmosome.cell.new` — wire method `cell.new`

```json
{"id": 5, "method": "cell.new",
 "params": {"kernel": "work", "genome": "researcher", "mock": "simulate",
  "artifact": {"registry_id": "6f1c2a9e-4b7d-4e3a-9f20-8d5b1c7e0a43",
   "release": {"kind": "genome", "population": "user", "publisher": "acme",
    "name": "researcher", "version": "1.0.0",
    "digest": "sha256:e2ddc2cb169322884a76554796ab8cc32f23231e90e50799ed8a25e13b2b9a48"}}}}
```

```json
{"id": 5, "result": {"cell": "cell-3", "state": "ready",
  "plasmids": ["github-pr [mock:simulate]", "workspace [mock:simulate]"]}}
```

- `genome` is optional; without it the cell starts empty (plasmids attach later).
- Optional `request_key` is 1 to 64 characters from `A-Z a-z 0-9 . _ -`, as in `cell.exec`; anything
  else, `null` and numbers included, is `-32602`. A key belongs to the instance and is shared by
  every client of it. While a cell holds the key, from its creation until it is retired, a
  `cell.new` with that key and the same other parameters, `kernel` aside, creates nothing.
  Parameters are compared as JSON values (spec 023 §3): key order, spacing and string escapes do not
  matter, but a member left out differs from one sent at its default value. Such a repeat gets that
  cell's `ready` result, `{cell, state: "ready", plasmids}`, once the cell's `created` file is
  synced; or, at once, 105 `{from: <state>, to: "ready", cell, detail: {kind: "repeat"}}` while the
  cell is `germinating`, `draining` or `dead`; or 101 `{target: "cell <id>"}` while it is
  quarantined, or set aside and not adopted by the check the repeat runs. The same key with other
  parameters is 102 `{target: "request <key>"}`. The 101 and the 102 carry no `cell` member, and
  none of these replies ends a cell. Spec 023 §4 step 1's host refusals and this spec's §3.4 stop
  gate come before the key is looked up, so a repeat can get them. The key is held with no time
  limit until the cell is retired. Then it is forgotten, and the same request starts a new cell.
  While a cell's key record cannot be read after a restart, every `cell.new` with a key is 105
  `{from: "request_unreadable", to: "germinating", cell}`. Spec 023 §3, §4 and §8 say where the key
  is kept and how it survives a restart. `repeat` is the 105 `detail` kind of this case. Amended on
  2026-10-21: this bullet is new; `cell.new` had no request key, so a repeat after a lost reply made
  a second cell.
- With a genome, optional `lease_ms` and `idle_ms` only shorten the lifetime of each attachment
  the request creates (spec 028). `exec_id` is `-32602`.
- The controller allocates the cell ID and starts its supervisor as spec 023 says. It replies only
  when the guest is up and the genome is attached; until then the cell is `germinating` and refuses
  `cell.exec`. Resolution refusals come before any cell directory exists; every later refusal
  other than a repeat's reply (above) carries `cell`, and its cell is ended first. On a host other
  than Darwin arm64, or without `cell_runtime`, cell.new is 105 `{from: "unsupported_host" |
  "no_cell_runtime", to: "germinating"}`. Amended on 2026-10-21: this read "every later refusal
  carries `cell`, and its cell is ended first", and a repeat's reply ends no cell.
- Optional `artifact: {registry_id,release:ReleaseRef}` selects the exact already imported
  spec020 genome, with required digest and a matching required `genome` name. It does not fetch,
  authorize widening or bypass normal closure checks. Missing import is101; corrupt/mismatched
  content is108. An absent artifact with a named genome refuses with 101 until a local genome
  grammar exists (spec 022). Spec008 journals each attached
  member's source as part of the same transaction; it does not infer a cell-level genome label.
- `mock` is optional; **bare `--mock` ⇒ `simulate`** (D2). When a genome is named, its
  `[plasmids.X] mock = …` table is the default layer; the request-level `mock` overrides it
  per D2's layering (genome table → `plasmid add --mock`; a reload keeps the held mode, spec 022).
- `plasmosome germinate <genome>` is the documented alias of `cell.new --genome <name>` (D1c).
  Until a local genome grammar exists, it also needs the genome's `artifact` (spec 022).
- The controller resolves the genome's plasmid set through **D2b** before any cell exists:
  closure-wide propagation, explicit beats inherited, explicit-vs-explicit on the same node at
  different modes → error `104` naming the node, both modes, both plasmids, and the
  `resolutions` (`force_simulate`, `force_passthrough`, `remove_plasmid`); safety-wins
  (simulate/capture beats passthrough) on inherited collisions.
- RESERVED (undecided): `cell.clone` (tier-2: state + plasmids, fresh brain), `cell.save` /
  `cell.load` (tier-3: dormant captured cell, `*.cell` file), `freeze` (future
  pause-and-resume).

### 3.6 `cell.list` / `cell.status`

```json
{"id": 6, "method": "cell.list", "params": {"kernel": "work"}}
```

```json
{"id": 6, "result": {"cells": [
  {"id": "cell-1", "genome": "researcher", "state": "ready", "agent": {"uid": 1000, "subjects": ["git", "curl"]}}
]}}
```

```json
{"id": 7, "method": "cell.status", "params": {"kernel": "work", "cell": "cell-1"}}
```

```json
{"id": 7, "result": {"cell": "cell-1", "state": "ready", "genome": "researcher",
  "plasmids": [{"plasmid": "github-pr", "mock": "simulate", "generation": 3, "tools": ["pr.read", "pr.comment"]}],
  "subjects": [{"subject": "git", "netns": "10.29.0.3", "attach": "allowed"}],
  "supervisor": {"ready": true, "state": "serving"}}}
```

- `supervisor` is the membrane's control-`status` answer relayed verbatim (F9 readiness: the
  controller reports what the supervisor *answered*, never process-alive heuristics).
- `subjects` are the E13 child-domain subjects: per-tool netns, address from the frozen
  `10.29.0.0/24` compiler constant, per-subject attach state.
- Each object in `plasmids` carries `lifetime`, always present, as spec 028 section 9 defines it.

### 3.7 `cell.kill`

Drain-by-default teardown of one cell; `--now` is the authority-class immediate kill.

```json
{"id": 8, "method": "cell.kill", "params": {"kernel": "work", "cell": "cell-1"}}
{"id": 8, "result": {"cell": "cell-1", "state": "dead", "drained": true, "residue": "empty"}}
```

With `"now": true` the reply carries `"drained": false` and the recorded operator assertion. `now`
requires `operator` and `reason`, nonblank strings: that assertion, which the reply carries as
`force: {operator, reason}`, spec 008's record shape, exactly when `now` is true. Optional
`deadline_ms` bounds the whole kill, default `cell_runtime.stop_deadline_ms` or 30,000. The cell is
`draining` from the start. The controller stops the workload if the guest runs and `now` is not set,
withdraws every attachment, sends `membrane.cell.kill`, waits for the supervisor to exit, and moves
the directory to `<instance>/retired/<cell>` before replying (spec 023). A step that falls short is
listed in `incomplete`, over the closed set `workload_stop`, `shutdown` and `teardown`; a failed
workload stop never blocks. `incomplete` is present only when a step fell short (spec 023 §7,
step 6), as §1 allows for a collection whose text says when it is present, so the clean kill above
has none. Amended on 2026-10-20: spec 023's text did not say when `incomplete` is present, so §1
would have sent `[]` on a clean kill. Every 106 a kill sends, for a drain or publication timeout, a
resumed transaction that does not finish without `now`, or a `now` kill whose session log line
cannot be appended, is `{handle, deadline_ms}` with `handle` the cell ID and `deadline_ms` the
kill's deadline. An unclean teardown replies `"state": "dead"` with its residue and `teardown` in
`incomplete`, and the cell stays listed for a retry. A cell that fails the check before its
retirement (spec 023 §7, step 5), or whose lock the kill cannot test (step 4), is decided at once by
spec 023 §8's table: unless the table retires it, the reply lists `teardown` in `incomplete`, and
the cell is set aside or quarantined and leaves `cell.list`; a later `cell.kill` of it is 101.
Clearing a set-aside cell whose lock is free is spec 023's owner question 3, and a quarantined cell
is outside spec 008; a cell whose lock cannot be tested is decided again by spec 023 §8's next
check once that lock is repaired. A guest not stopped or a supervisor not gone within the deadline,
or a rename that fails after the retire check passed, is 105 naming the cell. Amended on
2026-10-21: this did not cover a lock the kill cannot test.

### 3.8 `cell.exec`

Run a command inside the cell as its unprivileged workload user (spec 024). A named `subject`
requests an E13-style subject spawn, refused with 110 until host-side attestation exists.

```json
{"id": 9, "method": "cell.exec",
 "params": {"kernel": "work", "cell": "cell-1", "argv": ["git", "push"], "request_key": "push-1"}}
```

```json
{"id": 9, "result": {"cell": "cell-1", "exec_id": "e-11", "state": "running"}}
```

Completion is asynchronous; `exec.status` (companion, same envelope) polls:

```json
{"id": 10, "method": "exec.status", "params": {"kernel": "work", "cell": "cell-1", "exec_id": "e-11"}}
{"id": 10, "result": {"cell": "cell-1", "exec_id": "e-11", "state": "exited", "exit_code": 0, "duration_ms": 1823}}
```

```json
{"id": 11, "method": "exec.output",
 "params": {"kernel": "work", "cell": "cell-1", "exec_id": "e-11"}}
{"id": 11, "result": {"cell": "cell-1", "exec_id": "e-11",
  "stdout": {"bytes": [111, 107, 10], "total": 3, "truncated": false, "closed": true},
  "stderr": {"bytes": [], "total": 0, "truncated": false, "closed": true}}}
```

- A subject spawn is a **host-side attested** request (the E13b residual: the wire narrows to
  this one verb); an unattested spawn subject is refused with code `110`.
- The parameters, including `exec.status`'s `wait`, the workload's identity and environment, exit
  codes, retained output and the refusals are spec 024's. Every result names `cell`. A started
  process is not a capability: it is not journaled, observed as a grant or stopped by plasmid
  removal.
- RESERVED: streaming stdout/stderr frames as ndjson on the same connection.

### 3.9 `plasmid.list`

```json
{"id": 11, "method": "plasmid.list", "params": {"kernel": "work", "cell": "cell-1"}}
```

```json
{"id": 11, "result": {"cell": "cell-1", "plasmids": [
  {"plasmid": "github-pr", "mock": "simulate", "generation": 3, "state": "active",
   "label": "github-pr [mock:simulate]"},
  {"plasmid": "workspace", "mock": "passthrough", "generation": 3, "state": "active",
   "label": "workspace [real]"}
]}}
```

`mock` ∈ `simulate | capture | passthrough` — the D2 vocabulary, closed; an addition to it is a
contract change. Absent declarations mean `passthrough`. Each object carries `lifetime`, as in
§3.6.

### 3.10 `plasmid.add`

```json
{"id": 12, "method": "plasmid.add",
 "params": {"kernel": "work", "cell": "cell-1", "plasmid": "github-pr", "mock": "capture"}}
```

```json
{"id": 12, "result": {"cell": "cell-1", "plasmid": "github-pr", "mock": "capture", "generation": 4,
  "propagated": {"mode": "capture", "closure": ["github-pr"]},
  "attach": {"attach_to_first_allowed_ms": 57}}}
```

- `mock` optional; absent = the cell's inherited default (`passthrough` when nothing is
  declared). Setting a mode propagates across the plasmid's **whole dependency closure
  transitively** (D2b rule 1) — the reply reports what was propagated. `github-pr` requires no
  other plasmid, so its closure is itself alone, which is what the reply above shows; a plasmid
  that requires others names every one it reaches.
- **A mock is never a plasmid of its own.** It is the `[mock]` section of the manifest that
  declares the `[network]` hosts it stands in for, and the manifest is refused unless it is: a
  `[mock]` with no `[network]`, or naming a host that `[network]` does not declare, is a code `108`
  naming the host, never a silent acceptance. That is what keeps the two lists from drifting, and
  it is checked when the manifest is read rather than asserted here. A mode still propagates across
  a closure exactly as above — what it never lands on is a node whose only content is a mock.
- Inherited levels yield to the new explicit declaration (D2b rule 2). No level, inherited or
  explicit, changes a plasmid already attached: an attach that would change an attached plasmid's
  mode refuses with 104 naming it, with `resolutions: ["remove_plasmid"]` (spec 022). An
  explicit-vs-explicit conflict at different modes on a node not yet attached → code `104` with
  `resolutions: ["force_simulate", "force_passthrough", "remove_plasmid"]` (D2b rule 3 — never
  last-write-wins), the same three names §3.5 gives. `remove_plasmid` drops one of the two
  plasmids that declared a mode on the shared node, which is what makes it reachable without a
  mock plasmid to remove.
- Attach is the two-phase transaction over the Track B seam: validate the whole subgraph,
  then commit; a mid-commit failure rolls the prefix back and replies `CommitFailed`-shaped
  code `103`/`109` with the rolled-back list. Attach receipts carry the ledger generation so
  the reconciler converges on replay (86 §4 rule 2).
- Credential refs in the manifest follow ADR 80: `delivery` is an ordered non-empty list
  over the **closed** enum `handle | helper | inject | mint`, `consumer` pairs with it
  (`handle⇔wasm`, `helper⇔git`, `inject/mint⇒http/process`, `mint` legal as the git fallback),
  `inject` requires an absolute `path_scope`, and `[commands.<id>]` refs gain exactly one
  extra field, `subject`. A mismatch is a named attach-time error (code `108`), never a silent
  downgrade. Until credential custody exists, a reference that passes this validation still refuses
  the attach with 103 `unsupported:credential:<ref>` (spec 022).
- Optional `artifact: {registry_id,release:ReleaseRef}` selects an already imported spec020
  plasmid whose name matches `plasmid`; the digest is required. Missing import is101 and
  corrupt/mismatched content is108, without local fallback. Normal authority/mock/closure
  refusal codes remain unchanged. Every attached closure member carries spec008's durable
  AttachmentSource, and existing shared providers retain their original matching source.
  Without artifact, the plasmid and its local providers are selected through spec 022's operator
  declarations file: an unlisted ID is 101, and an unsafe or mismatched declaration is 108. Every
  closure member's declared network and workspace needs are resolved into complete capabilities
  through that file before prepare; an uncovered need is 103 naming the capability and plasmid.
- Optional `lease_ms`, `idle_ms` and `exec_id` only shorten the lifetime of every attachment the
  request creates, providers it pulls in included (spec 028). An `exec_id` whose exec is not
  `running` is `105` `{from: <state>, to: "running"}`, and an unknown one `101` `exec <id>`;
  neither prepares anything.

### 3.11 `plasmid.remove`

Drains by default (the no-residue rule: LIFO ledger replay over the F9 universe; the plasmid
never punishes detachment). `--now` is the authority-class immediate removal.

```json
{"id": 13, "method": "plasmid.remove", "params": {"kernel": "work", "cell": "cell-1", "plasmid": "github-pr"}}
```

```json
{"id": 13, "result": {"cell": "cell-1", "plasmid": "github-pr", "state": "removed", "drained": true,
  "replayed": 2, "delayed_discarded": 1, "residue": "empty", "held": []}}
```

`held` lists each reference into the plasmid's software, or a reload's replaced tree, that the
guest's scan saw once its names were gone, as `{path, pid, comm, exec_id}` (spec 027). It is not
residue, so `residue` stays `"empty"`.

Outstanding external effects refuse the safe removal with code `105` carrying the assertion
list; force requires the operator/reason pair exactly as `plasmosome.stop`. A removal that would
strand an attached requirer (spec 011) is refused with code `105`
`{from: "active", to: "removed"}` and no assertion list, and its `message` names the requirers,
sorted; it carries no `fix`, which §1 gives only to 108. Amended on 2026-10-20: this gave spec
011's stranding refusal no code.

### 3.12 `plasmid.reload`

Generation swap of an attached plasmid (reload = new generation, not remove+add); the swap keeps
the plasmid's mock mode, and a `mock` that differs from it refuses with 104 (spec 022).

```json
{"id": 14, "method": "plasmid.reload",
 "params": {"kernel": "work", "cell": "cell-1", "plasmid": "github-pr"}}
```

```json
{"id": 14, "result": {"cell": "cell-1", "plasmid": "github-pr", "mock": "capture", "generation": 5,
  "state": "active", "held": []}}
```

Reload preserves the attachment's spec008 AttachmentSource. A registry source reuses the exact
saved registry/root/member graph and verified imported bytes, including provider bindings;
it never picks the newest version or a local namesake. No artifact parameter is accepted here.
Ordinary generation-swap checks are unchanged. Reload re-resolves the plasmid's capabilities at its
held mode against the operator declarations file as it is at reload time; a registry member keeps
its exact imported declaration. A reload whose new capabilities would widen what the plasmid holds
refuses with 109 before prepare (spec 022). Selecting a different release
requires an explicit permitted detach/add or a separate new cell, not an implicit source switch.
Missing/corrupt imported bytes use101/108 before effects and leave the old attachment intact.

### 3.13 `plasmid.renew`

`plasmid.renew` takes `kernel`, `cell`, `plasmid`, exactly one of `not_after` (an RFC 3339 UTC
instant) or `until_revoked: true`, and `operator` and `reason`, nonblank as for Force. A renewal
only moves the bound later, and never past the ceiling recorded at attach: `not_after` is at most
that ceiling from now, and `until_revoked` needs an `until_revoked` ceiling. Anything else, or a
bound earlier than the one in force, is `-32602` naming the limit. A requirer is never renewed past
a provider's bound: that is 103, with the provider in `plasmid` and the capability it provides in
`capability`. The result is `{cell, plasmid, lifetime}`, with `cell` the cell the verb acted on
and `lifetime` the renewed lifetime in the shape `plasmid.list` gives it (spec 028 section 9). A
`not_after` equal to the bound in force writes nothing and replies as before, so a repeat after
spec 025's exit 3 is safe.

A renewal is a spec 008 transaction whose replacement changes only the lease; an idle limit or exec
scope stays. After its prepare is durable, a session-log `renew` event records operator, reason,
and old and new bounds; like `force`, it records authorization, not completion. Then come commit,
finish and publication. The supervisor raises a gate only from a published DesiredCell, so a raise
always follows a durable commit and no gate is later than the journal. If B (spec 028 section 1)
passes before the commit, the controller appends abort and replies 105
`{from: "draining", to: "active"}`. If a gate fences between commit and publication, the fence
stays, and the controller expires the attachment with cause `lease` and replies the same 105.
Renewal and expiry share spec 023's arrival order: a renewal arriving after B gets that 105
before prepare, and one still running at B fails.

## 4. Controller ⇄ membrane (the supervisor side of the contract)

The controller drives each cell's `membraned` over a second, private ndjson-UDS
(`<instance>/cells/<cell>/membrane.uds`). Same envelope as §1, and the same §1 control-socket
lifecycle: `membraned` refuses an occupied path and removes its own bound socket only, by
no-follow device-and-inode identity. The subset this spec covers:

- `membrane.status` — the F9 readiness probe. Reply `{"ready": true, "state": "serving"}`.
  Readiness = the socket **answers**; accept-without-answer is the half-alive broker and is
  reported not-ready (measured in F9; implemented in `plasmosome-membrane::readiness`).
  A membrane that is **not** ready answers the same `result` object with `"ready": false` and a
  `state` naming which case it is. Every call asks every broker again, and a membrane configured for
  a cell also reports its cell; no answer is kept. Such a membrane is ready exactly when its guest
  has said hello, no stop or kill has arrived, and every broker answers ready.

  | `state` | further fields |
  | --- | --- |
  | `not_serving` | `broker` — the one that held the set back; `reason`; and `broker_state` when, and only when, `reason` is `reported` |
  | `deadline_spent` | `unreached` — the first broker never asked; `asked` — those that spent the budget, in probe order, omitted when the deadline was gone before any broker was asked |
  | `cell_not_ready` | `cell_state` — the cell's lifecycle state: `germinating`, `draining` or `dead` |
  | `empty` | none — a membrane with neither a cell nor brokers, which is never ready |

  `reason` is one of `unreachable`, `timed_out`, `malformed` or `reported`: the socket was not
  there, it did not answer inside the deadline, it answered something that was not a status, or
  it answered that it is not serving and said so in `broker_state`. Delivered in
  `plasmosome-membrane::{control, daemon}` and served by `membraned`.

  **Response size, one fixed budget, and shutdown.** These are new normative decisions, announced
  here as such — not restatements of earlier promises. Measured reason for the addition: on
  b5127d5 (Darwin25.6.0, arm64) a broker that trickled thirty spaces at 20ms intervals before its
  `ready` frame was answered `ready:true, state:serving` after 813.829ms against a 100ms
  `status_deadline_ms`, because nothing bounded the whole query. Delivered in
  `plasmosome-membrane::{readiness, brokers, control, daemon}` and served by `membraned`.

  1. A broker status response is the first newline-terminated NDJSON frame. At most 1,048,576 raw
     bytes may precede that newline, counting every byte including whitespace and CR, before
     UTF-8 decoding or trimming; the newline itself is excluded. The first excess non-newline byte
     is refused immediately as `malformed`, without draining the peer. At-cap plus newline remains
     valid. This is a readiness-broker response limit, not a blanket cap on every controller
     response and not an inference from §1's request limit; the magnitude avoids a second
     arbitrary transport limit while making retention finite.
  2. A missing, refused or invalid socket address, and failed connection establishment, stay
     `unreachable`. A connection still pending when the allowance expires, or temporary
     noncompletion from a full nonblocking AF_UNIX backlog (EAGAIN/EWOULDBLOCK), is `timed_out`.
     A connected peer's silence, an incomplete frame, or EOF before the newline is `timed_out`,
     even when the unterminated bytes form JSON. A terminated non-UTF-8, invalid-JSON, non-status
     or over-cap frame is `malformed`. A valid `ready:false` keeps `reported` with its exact
     `broker_state`; a valid `ready:true` keeps its meaning. Ids are not newly validated, `state`
     need not be nonempty, and no other classification behavior tightens here.
  3. One readiness call takes one monotonic start and one fixed duration covering connect, request
     transmission, reply acquisition and classification. Reaching or exceeding it prevents another
     I/O, poll or probe step and refuses a late answer, including the last broker's. Checks follow
     every system call and the bounded classification. A probe that used its allowance yields
     `not_serving`/`timed_out` naming that broker. `deadline_spent` means a broker was never
     entered because the set budget was already gone, retaining `unreached` and `asked` in order.
     Empty remains empty; no readiness result is cached. A custom synchronous `Probe` cannot be
     preempted by the set; its contract requires observing the supplied budget and cancellation,
     and the set rejects a result that returns late instead of blessing it ready.
  4. SIGINT/SIGTERM's existing shutdown flag is propagated into probes. Cancellation wins when
     both cancellation and expiry are observed at one check. Cancellation is not a new wire reason
     or broker state: the in-flight control connection closes with no status reply, probing and
     accepting stop, and the daemon enters its existing teardown. A status may have been sent if
     shutdown arrives after the final cancellation check, so no transactional signal/reply
     ordering is promised. The reply is re-checked against the flag after construction and before
     writing: no stale `ready` response may be emitted after cancellation was observed inside the
     probe.
  5. Production sockets are nonblocking before `connect` and stay nonblocking through request and
     reply. Each wait is requested for at most min(remaining, 25ms), recalculated from the same
     start, with budget and cancellation checks around waits and every bounded I/O chunk. Partial
     progress, EINTR, WouldBlock and readiness notifications never renew the deadline. Millisecond
     poll rounding adds less than 1ms to a requested wait; OS clock granularity, scheduling, path
     lookup, kernel execution and bounded decoding can overshoot, and there is no unconditional
     wall-clock deadline or universal numerical syscall-overrun bound on macOS or Linux. A finite
     connect overshoot consumes the original budget and is discarded on return; it never grants a
     fresh budget or a late ready. Nonblocking mode removes the intentional wait for socket and
     backlog readiness, not all kernel or filesystem latency. Cancellation is recognized at the
     next check — normally within one requested 25ms wait plus those explicitly unbounded OS
     scheduling and syscall delays and one bounded processing step. This bounds application
     waiting and work, not entire daemon exit: child termination, reaping and destructor limits
     remain separate. No blocking connect is retained behind a claim of cancellability.
- `membrane.cell.desired` — desired-state push, **idempotent and generation-numbered**: the
  full desired cell record plus `generation: u64`. An identical equal-generation request is
  a no-op; conflicting content at that generation refuses. An older request is ignored and
  returns the current published record, never an assertion that the older payload was published.
- `membrane.cell.observe` — the supervisor's observed state (cells, broker readiness, VMM
  liveness). This is the only source the controller trusts for liveness; `sessions.status`-style
  requested state is lifecycle, not liveness.
- `membrane.cell.denials` — `{cell, book?, after: u64}`, answered from the membrane's memory with
  no wait and not paged, like `membrane.cell.desired`, as `{cell, book, lines, entries}`. `book`
  is a UUID the membrane draws when it starts. `lines` are the spec 011 report lines the membrane
  holds with a number above `after`, at most 64, in order. `entries` are every entry that names a
  plasmid, as `{plasmid, denials}` with spec 011's `denials` shape, in the order first seen. A
  request whose `book` equals the membrane's drops the lines numbered `after` and below, so a
  line is dropped only after the controller has appended it; a request with no `book` or another
  one drops nothing and is answered as if `after` were 0, so a restarted membrane never drops a
  line the controller has not seen. The membrane keeps the counts and lines for its own life, and
  forgets a plasmid's entries when a published DesiredCell no longer has that plasmid. Spec 011
  gives the bounds. Amended on 2026-10-21: this method is new. §4 named no method that carries
  spec 011's denial report from the membrane to the controller.
- `membrane.residue.snapshot` — the F9-universe observation taken **from the supervisor/broker
  side** at diff time (86 §4 rule 4). The controller diffs snapshots, never its intentions.
- `membrane.cell.kill` — drain-then-kill with `DrainSpec { deadline, policy }` carried
  verbatim from the seam types; the membrane owns the VMM child, shim, and brokers as **its**
  children — never the controller's (86 §4 rule 5), and per-cell vs per-host brokers is an
  explicit parameter in the desired record. The DrainSpec deadline is positive, and the policy is
  `Graceful`, or `Force` for `cell.kill --now`. The membrane sends the guest `shutdown` within the
  deadline, kills and reaps the helper when it passes, and answers `{cell, state, clean, residue,
  helper_killed}` with residue it observed itself after teardown, and `residue_items` exactly when
  `residue` is `items`. `helper_killed` is a required boolean, true when it killed the helper's
  process group because the helper still ran at the deadline. After a clean teardown it removes its
  launch marker, removes its own socket and exits; otherwise it keeps running with its lock and
  marker (spec 023).
- `membrane.cell.exec`, `membrane.exec.status`, `membrane.exec.output` and `membrane.workload.stop`
  relay to the 4090 `exec`, `exec_status`, `exec_output` and `workload_stop` verbs on the original
  association's normal lane (spec 024).
- The selected hardware runtime, shim and noncredential bridge are specified in §4.2.
  Remaining broker lifecycle verbs beyond the exact apply/withdraw contract stay reserved.
  The credential vsock proxy remains RESERVED: port4041 terminates at the membrane and proxies
  to the controller; custody state stays kernel-core.

### 4.1 Recovery startup and exact operation messages

These are the recovery additions specified by spec008, not claims that the existing membrane
daemon already serves them. Its existing `membrane.status` response and error codes remain.
The configured controller gains required absolute `instance_root` and positive integer
`recovery_deadline_ms`, alongside `control_socket` and `name`; no root is inferred from the
socket. Startup has one monotonic recovery deadline covering discovery, observation, pending
cleanup and settled-generation publication, not a new budget per retry. After a live mutation's
durable finish, its publication exchange gets one recovery_deadline_ms budget, likewise not
reset on retry. Under `cell.kill`, that exchange also ends within the kill's deadline (spec 023).
Exhaustion refuses startup or blocks further mutation of that cell; it is never
an empty observation or successful client result. JSON configuration rejects unknown keys as before.

The same strict JSON configuration also admits optional `registry_root`, an explicitly supplied
absolute trusted import path under spec020. It is independent of instance_root and control_socket;
neither those paths, the caller's HOME nor a request field supplies a default. A relative or
non-string value is invalid configuration. Omission preserves purely local operation. Validate
ownership, no-follow traversal and immutable imported content at every use under spec020.
An artifact-bearing request with no configured root or no matching import refuses with101 and
the full artifact reference as target; unsafe/corrupt imported content refuses with108.
A settled recovered registry source with no configured root, missing bytes or failed validation
refuses startup with ArtifactSource/artifact_source as below, without fallback. If no root was
configured, that diagnostic omits path_bytes; it still includes the complete recorded references.

The same strict JSON configuration also admits optional `operator_declarations`, an explicitly
supplied absolute path to spec 022's operator declarations file. Nothing supplies a default; a
relative or non-string value is invalid configuration. Only the resolving mutations of spec 022
read it. Startup, recovery, status and every withdrawal never open it, and its absence or
corruption never blocks them.

The same configuration admits optional `cell_runtime: {membraned: Artifact, template,
start_deadline_ms, stop_deadline_ms}`, spec 023's deployment input for new cells, in which
`stop_deadline_ms` is optional. Recovery never reads it.

The controller holds spec008's instance writer lock and discovers validated cell directories
before sending requests to their membrane sockets. A membrane is configured for one validated
cell and rejects a request naming another cell with code101. Each connection uses §1 framing,
request IDs and errors. Responses with a wrong request ID or cell, unknown fields/variants,
malformed exact objects, incomplete frames or timeout are failed observations. The controller
must not operate a quarantine's effects merely because its supervisor answered.

The observation methods `membrane.cell.observe`, `membrane.residue.snapshot`, §4.2's 4090
`observe` and public `plasmosome.recovery` return bounded pages of one complete logical result.
Their request IDs are unsigned64-bit integers. Each params record includes required
`snapshot:u64` and `offset:u64`: zero/zero starts a capture and returns its first page; later
requests use that returned nonzero snapshot and the next byte offset.
Their wire result is the strict
`ObservationPage {snapshot:u64, offset:u64, total:u64, sha256:String, bytes:[u8], complete:bool}`.
The logical results are compact UTF-8 JSON without a trailing newline. Internal methods freeze
those exact bytes only after fresh independent collection and validation succeeds; the public
method pins an already completed RecoveryOutcome without refreshing it. `total` is their
positive byte length; `sha256` is their64lowercase-hex SHA256. Each page contains1–65,536bytes;
offset arithmetic is checked, and `complete` is true exactly when offset plus page length equals
total. Compact encoding of the page and bounded request ID fits the1,048,576-byte frame limit
even when each byte takes three decimal digits. No inventory cardinality cap or array truncation
is substituted for paging; both request and page response retain that frame limit.

Snapshot IDs are connection-local, never reused, and bound to the method and resolved cell or
instance. Internal observations additionally bind the original supervisor/guest association;
public diagnostics bind the original controller and the selected retained outcome.
A new zero/zero capture replaces the prior capture on that
connection; at most one is retained. It never resumes another connection's capture. Later pages
read the same immutable bytes, not fresh rows mixed with older pages. A repeated valid offset
returns the same page, allowing a lost reply to be recovered without recapturing. Reject an
unknown snapshot, changed scope or out-of-range offset with `-32602`. Capture/resource failure,
expiry or loss of the original association is `-32603`, not an empty page.
The first request's remaining deadline bounds capture and all pages; later deadline_ms values
may shorten but never renew it. Retained bytes expire at that original deadline or connection
close, even after the last page; no page wait holds an enforcement lock or blocks the independent
withdrawal dispatcher. Replacing a capture discards only observation bytes, not resource authority.

The caller accepts only contiguous pages beginning at zero, with one snapshot/total/hash,
positive bounded payloads and the exact completion flag. It checks the assembled length and hash,
then strictly decodes and validates the complete logical record, including all required arrays
and original identities. No prefix, early completion, missing/overlapping page or mixed capture
can authorize publication, readiness, drift, cleanup or a complete public diagnostic. Connection
loss, deadline expiry or failure at any observation/diagnostic hop fails that whole operation;
a new internal capture uses only the remaining outer recovery budget. Paging preserves the
selected record's observation semantics; it does not stop resources changing during transport.

- `membrane.cell.observe` params are `{cell, deadline_ms, snapshot, offset}`. `deadline_ms` is a positive
  remaining budget capped by the controller's recovery deadline. Its assembled logical result is
  `{cell, state, supervisor, generation, desired, gates}`: `state` is the existing closed cell lifecycle
  enum from actual child/supervisor observation, `supervisor` is the current `membrane.status`
  result, and `desired` is the complete last published DesiredCell retained by this supervisor,
  with `generation == desired.generation`. `gates` lists, for each grant, its `GrantId`, both
  readings of its bound, its idle deadline and whether its gate has fenced (spec 028). It is
  host-gate state the supervisor observes itself, never a `GuestObservation` or `GuestBinding`
  field. It is read fresh at each call, not retained. Generation and content are retained
  atomically.
  A fresh supervisor starts with the empty generation-0 record. Missing retained content is an
  observation failure, not permission to reconstruct it from the requesting controller.
  This publication record is not evidence of liveness or holdings: those still require actual
  supervisor and enforcement-side observation. No PID-file or socket-existence substitute is allowed.
- `membrane.residue.snapshot` params are `{cell, deadline_ms, snapshot, offset}`. Its assembled logical result is
  `{cell, state: OsState, grants: [LedgerEntry, ...], incomplete: [IncompleteEffect, ...],
  guest: GuestObservation}`. The first three required collections are spec017's strict
  EnforcementSnapshot; guest is the independently obtained physical projection account in §4.2.
  An incomplete record contains its complete original operation, not a successful OsObject. All records belong to the
  addressed cell. `state` is fresh enforcement-side observation including unrequested residue.
  `grants` includes only original issued records with retained authority, possibly during partial
  withdrawal; failed initial application/grant and direct apply fabricate none. No collection
  is rebuilt from desired state or planted objects. Unsupported or failed class observation fails
  the whole request with `-32603`, never omits the class or returns a requested-state mirror.
  The logical result contains the five specified capability classes. GuestObservation explicitly
  enumerates their physical guest projections; it does not fabricate another logical grant class.
  Adding a different guest capability still requires explicit enumeration and an actual observer.
  Broker capabilities contain the exact launch description, never a predicted PID. That description
  is not evidence of a running process. The supervisor must independently retain and verify the
  original runtime association. Direct apply does not manufacture a LedgerEntry or opaque handle
  for `grants`. Backend snapshot failures, including a failed Composite leaf, propagate to the
  whole RPC; success requires fresh coverage of every class.
  Duplicate/conflicting addresses, standing/incomplete overlap or an out-of-cell row refuses the
  whole response. Post-side-effect failure is represented in incomplete, not hidden because no
  complete object was published. Empty state alone never establishes cleanup: the exact address
  must be absent from state AND incomplete. Lost authority is a failure, not an empty collection.
- `membrane.effect.apply` params are `{cell, generation, operation, lifetime, deadline_ms}`,
  where `lifetime` is the attachment's bound, in both readings, and its idle limit, which the
  operation's host gate enforces from activation (spec 028). `lifetime` also carries `drain_ms`,
  the attachment's drain budget D, because from activation the gate caps each admitted request's
  work at the bound plus D (spec 028 section 4).
  `operation` is a fully predeclared spec017 UniverseOp with spec008 owner. The trusted
  controller sends it only after that generation's prepare is durable. Result is
  `{cell, generation, id, applied: true}` after enforcement accepted the exact operation.
  Duplicate application of the same standing operation retains spec017's no-op semantics.
  This result does not replace the independent snapshot used for recovery verification.
  `SpawnBroker` carries `{id,name,launch,owner}` with spec017's strict
  `BrokerLaunch {command,control_socket,data_socket}`. Other operations include their complete
  required resource recipes. PID-bearing, missing or malformed recipe fields are invalid
  parameters, not a request to guess materialization inputs.
  Before prepare the controller has validated the trusted recipe and non-destructive staging; the supervisor creates no broker
  or endpoint before that durable boundary. When the resource is absent, actual launch binds
  the original owned child to this holding before successful application can be reported.
  Equal fresh holdings may share an original resource only with independent enforcement-side
  access. A lost reply cannot cause a duplicate child when the same standing operation is
  repeated. On controller restart, spec008's journal decision governs: an uncommitted prepare
  is durably aborted and its complete or incomplete introduced effects cleaned, not reapplied.
  Partial application returns the typed incomplete_effect refusal and retains original cleanup
  authority; grant/apply retry at that incomplete address cannot launch again or complete access.
  Neither PID backfill nor adoption of a matching process name is an alternative implementation.
- `membrane.lifetime.lower` params are `{cell, plugin, bound, deadline_ms}`. The supervisor takes
  `bound` for every grant of that attachment only when it is earlier than the bound it holds, and
  fences at once when it has passed; the controller sends only a bound that has passed, since the
  journal does not record a lowered bound; the supervisor takes a later bound only from a
  published DesiredCell. Result is `{cell, plugin, bound}`, with the bound now held (spec 028).
- `membrane.effect.withdraw` params are `{cell, generation, removal, owner, drain, deadline_ms}`.
  `removal` is the exact spec017 UniverseRemoval, `owner` is CellOwner, and `drain` is the
  existing DrainSpec serde value. Result is `{cell, generation, id, withdrawn: true}`.
  The backend preserves all neighbours, drain behavior and incarnation checks. Unknown-object
  and unknown-handle refusals are not translated into success; the controller must freshly
  observe exact absence from BOTH standing and incomplete collections before completing an obligation.
  Exact removal passes the supplied DrainSpec through to enforcement, just as original-handle
  revoke does. Graceful timeout retains the exact holding and its peers. Withdrawal of the
  last broker holding cleans the original resource; it never signals a numeric PID copied
  from a request or old record. Loss of original authority is backend_fault, not absence.
  A matching incomplete operation is selected by the same full inverse and owner, with the same
  DrainSpec; no new cleanup RPC or authority is invented. Only its original owned partial resources
  may be released, never a competing endpoint or peer. Timeout/partial failure retains the marker
  and remaining authority. Success means every owned partial effect is independently absent.
  The controller must durably abort an uncommitted introduction before this cleanup; an unmatched
  or quarantined incomplete marker is reported and keeps readiness false, never an automatic withdrawal request.
- For either effect method, backend refusal is code105 with `from: "prepared"` or `"held"`,
  `to: "applied"` or `"withdrawn"`, plus `recovery: {kind, ...}`. The closed kinds and fields
  are `identity_conflict {class,id}`, `unknown_object {class,id,key,owner}`,
  `unknown_handle {handle}`, `drain_timeout {handle,deadline_ms}`,
  `incomplete_effect {class,id,detail}`, and `backend_fault {detail}`. Incomplete_effect means
  retained partial original effects, not success; the complete snapshot exposes its operation
  until exact cleanup establishes absence. Unsupported enforcement and lost/unverified original
  authority are backend_fault, not successful model operations. The human message is not a selector.
  For incomplete withdrawal, `from: "held"` names retained cleanup responsibility, not proof
  that a complete holding exists. A deadline after partial release is incomplete_effect;
  drain_timeout preserves the pre-withdrawal holding and does not pretend partial release was undone.
  Protocol parse/parameter/internal errors keep §1's existing meanings.
- `membrane.cell.desired` params are `{cell, generation, desired}` with spec008's complete,
  settled DesiredCell reconstructed from the journal; the outer and desired generations must
  agree. Result is `{cell, generation, desired}`, carrying the complete currently published
  record and its generation, not an unchecked echo of the request. A newer generation atomically
  publishes the full record only after its individual effects, cleanup and durable finish have
  completed; it never infers operations, allocates replacement IDs or resurrects holdings.
  An equal generation is a no-op only if every decoded DesiredCell field matches the retained
  record, including ordered plasmids/effects and their complete metadata. A mismatch changes
  nothing and returns code105, `from: "settled"`, `to: "conflicting"`, with
  `recovery: {kind: "desired_conflict", cell, generation}`. JSON spacing and object-key order
  are not content differences. An older request changes nothing and returns the newer retained
  record; it cannot acknowledge publication of the requested older content.
  The controller requires matching request-ID/cell/generation and complete desired content in
  the acknowledgement, followed by a fresh observation matching that same complete record,
  before mutation success or completed rollback. An abort publishes unchanged attachments at
  its consumed new generation. Lost acknowledgement can resend the identical settled record
  without repeating effects. Neither the reply nor the observed publication record substitutes
  for independent readiness and holding observations.

Recovery sockets are restricted to the instance's trusted host UID: the controller and its
membranes run under that same effective UID. Before binding, the membrane opens and validates
the socket's private parent directory without following symlinks: it is owned by that UID,
mode0700, with no ACL granting access to another principal. Ancestors cannot be replaced by
an untrusted principal. The bound socket is owned by that UID and mode0600 before listening;
umask alone is not the boundary. An existing unsafe directory/socket is refused, not silently
adopted, chmodded or unlinked. The controller validates this same path boundary before connecting.
Both endpoints obtain the connected peer's effective UID from the kernel (`getpeereid` on
macOS, `SO_PEERCRED` on Linux) and require it to equal their own. Missing credentials, a mismatch,
or failure to establish the path boundary closes/refuses the connection before request parsing,
method dispatch or trusting a response. No recovery result is returned on an unauthorized
connection. Client-supplied cell, PID, operator or UID fields are not authentication.

Processes under that host UID, and host root, are inside the trusted boundary; this is not
protection against their compromise or a multi-user authorization service. A cell workload must
not share that host principal's access to the socket namespace: it runs under a different host
UID or confinement that denies the socket path and connections, and receives no connected/listening
socket descriptors. Merely removing cell write permission is insufficient. The launcher must
establish this exclusion before starting a workload; an unsupported or failed confinement is
a refusal, not permission to expose the recovery endpoint. The instance writer lock serializes
controllers, not arbitrary clients, and is not authentication.
The supervisor retains exact resource associations independently of controller memory and must
verify resource/process incarnation before enforcing a withdrawal. No recovery RPC may create
an in-memory backend in production and report its ledger as an OS observation.

Before serving, the controller completes spec008 discovery, observation, pending recovery and
per-cell generation and complete-content settlement within its single startup deadline. For a
settled journal generation, a lower membrane generation requires complete desired republication,
matching acknowledgement and fresh matching observation. Equal generation continues only when
the complete observed DesiredCell matches journal replay; mismatched content refuses without
overwrite. Higher generation refuses as unaccounted participant state, never as a reason to
send an older no-op request. Before pending cleanup, an ahead generation or a membrane already
at that unfinished journal generation also refuses. All requests, acknowledgements and fresh
observations use the remaining deadline. Aborted and empty cells follow the same rules.
A lower post-ack generation can only retry within that budget; higher generation or conflicting
equal-generation content refuses.
An instance-wide fault emits one LF-terminated JSON object on stderr:
`{recovery_error:{kind,path_bytes,detail}, quarantined:[...]}`. `kind` is one of
`writer_busy`, `discovery`, `identity_conflict`, `desired_conflict`, `artifact_source`,
`observation`, `cleanup`, `deadline`, or `io`;
`path_bytes` is the exact related Unix path as byte values, omitted if no path applies.
For a generation observation refusal or equal-generation `desired_conflict`, recovery_error
additionally carries `cell`, `journal_generation` and `membrane_generation` as structured values.
An `artifact_source` refusal additionally carries `cell`, `plugin`, `registry_id`, `root` and
`member` with full spec020 references, projecting spec008's ArtifactSource error. It means the
settled registry source could not be verified under the trusted import root before serving,
not that its journal or enforcement holding may be discarded. Pending recorded cleanup remains
independent of artifact availability. This adds a startup diagnostic kind, not an application
error number or a network lookup during replay.
Equal generations or object snapshots cannot hide conflicting published content in a ready result.
Quarantine entries whose observation failed include `observation_error` and omit `found`;
an empty found list is reserved for an actually observed empty set or an invalid entry with
no attributable cell. Error exit removes only this invocation's control socket and releases
the lock; it neither kills the surviving cells nor rewrites their journals.

Spec008's R9/R10 acceptance exercises this actual socket path and independent enforcement-side
observation. A library-only fake or manually constructed Controller cannot demonstrate it.


### 4.2 Hardware runtime and mediated guest access

This selects the next runtime contract, not an implemented cell. A real cell is a hardware-isolated
Linux guest whose membrane, VMM and original resource authorities survive controller death.
An unrestricted host process or model observer cannot substitute. Spec017 defines all five actual
access adapters; spec008 still requires real restart, complete observation and quarantine.

The membrane execs a dedicated VMM helper linked to libkrun v1.19.4, commit
`728df8125077d0db44265f6e997c72b81b65c015`. Its record is `docs/dependencies/native/libkrun.toml`,
as spec 026 §1 asks of each native artifact. Amended on 2026-10-21: this did not name the record.
Configuration and `krun_start_enter` run inside that
already-exec'd helper, never as unsafe Rust work between fork and exec. The context is consumed
by start; `krun_free_ctx` is not a running-VM shutdown authority. Retain the original child and
prevent leakage of VM descriptors to descendants. Guest-ready requires the trusted shim's actual
handshake, not a successful fork, socket file or recorded PID.

Trusted deployment input supplies a `RuntimeRecipe`:
`{version:1, vcpus:u8, memory_mib:u32, kernel:Artifact, kernel_format:KernelFormat,
initramfs:Artifact, root_image:Artifact, writable_root:String, control_path:String,
data_path:String, helper:Artifact, libraries:[Artifact], host_policy:Artifact,
guest_policy:Artifact, architecture:Architecture}`.
All fields are required and
unknown fields refuse. Artifact is `{path:String, sha256:String}` with an absolute NUL-free path
and64lowercase hexadecimal SHA256 digits; Architecture is `aarch64 | x86_64`. CPU/memory values
are positive and fit the actual platform's admitted limits. Artifacts are publisher-produced,
architecture-matched and pinned before launch. The trusted operator selects them, never workload
text. The root image is explicitly RAW; no format autodetection or guest-selected backing paths.
KernelFormat is exactly `raw | elf | pe_gz | image_bz2 | image_gz | image_zstd`, mapped to the
pinned header's KRUN_KERNEL_FORMAT constants0 through5 respectively. Reject `x86_64` with
kernel format `raw` before launch: the pinned `krun_set_kernel` returns through `map_kernel`
without retaining the supplied initramfs or command line, so it cannot supply this bootstrap.
Every admitted format/architecture pair must preserve the exact protected initramfs and fixed
command line below. The publisher proves that pair supported by the pinned library; enum
membership alone is not admission, and no format guessing or hidden command-line override is
allowed. `writable_root` is an absolute NUL-free path under
an owned nonreplaceable private per-cell directory. Create that copy exclusively from root_image
before VMM launch and retain its original inode/authority; it is never the immutable source path
or a caller-selected existing file. This path is an explicit launch input, not recovery authority.
Use immutable verified kernel/initramfs/base artifacts and a separately owned RAW per-cell copy
for writes. Verification/opening must preserve the selected inode/bytes through the library's
path-based opens; a replaceable parent or mutable artifact is refused, not trusted after hashing.
A runtime recipe is deployment input retained by the original supervisor, not a mutable recovery
database or permission to recreate a lost VM. Recovery reconnects, not relaunches. The controller
starts each cell's supervisor once, as spec 023 §4 says. The supervisor holds the cell's
`supervisor.lock` for its whole life, and creates the `launched` marker, which records the host boot
session, before any runtime resource. Only `membrane.cell.kill`, or a launch that fails before
hello, removes the marker and the guest's disk.
`control_path` and `data_path` are distinct absolute NUL-free Unix socket paths under owned,
nonreplaceable private per-cell directories. The original supervisor creates and retains those
listeners before launching this helper. Their permissions admit only this supervisor and its
authorized original helper principal; they never expose the private recovery endpoint.

The membrane uses direct exec, not a shell: argv is exactly
`[helper.path,"--runtime-fd=3"]` and the **host exec environment is an explicit empty array**.
Before exec, descriptor0 and1 refer to `/dev/null`, descriptor2 to an owned diagnostic pipe,
and descriptor3 to the owned read end of a pipe containing exactly one complete UTF-8 ndjson
RuntimeRecipe and then EOF, within the existing frame bound. No other descriptor survives.
The helper strict-reads that record and closes3 before guest execution. Runtime configuration
does not come from argv expansion, inherited environment or a guest-visible file.

Before exec, verify the helper and full publisher-controlled dependency closure against
`helper`/`libraries` and bind their loading to immutable verified paths. Publisher-fixed absolute
load names, or loader-relative names confined to the verified immutable bundle, must resolve
uniquely without environment, working-directory or fallback searches. Unlisted loadable code
refuses; no unverified plugin/lazy-load path may execute before an after-the-fact inventory.
Darwin's authenticated sealed system libraries/shared cache are a separate platform trust
boundary: retain their actual OS build/cache identity, not a claim that those shared-cache
images are ordinary publisher files. Other Darwin dependencies and every Linux userspace
dependency/interpreter must belong to the verified library closure. Before guest execution,
independently inspect the actual loaded image set and compare those original identities; a
requested library list or removal of LD_*/DYLD_* variables after main starts is not that proof.
Retain this host launch evidence in the original supervisor. If libkrun logging is initialized,
use only `krun_init_log(2,KRUN_LOG_LEVEL_WARN,KRUN_LOG_STYLE_NEVER,KRUN_LOG_OPTION_NO_ENV)`.
The guest-executable environment API below is distinct and cannot sanitize host exec/loading.

Before helper exec, the original supervisor verifies `host_policy.sha256` against the exact
immutable policy bytes and retains their original identity. Apply that verified policy to the
original helper before guest execution; a different generic deny-default policy is not equivalent.
Retain host-side evidence binding the verified policy, its successful installation and that
original process. Before readiness and during every recovery observation, verify that retained
binding and the effective host restrictions independently of guest reports or requested settings.
A changed policy, lost application evidence or unverifiable confinement fails readiness/observation;
neither rehashing a pathname nor a guest-policy hash establishes active host confinement.

Call `krun_create_ctx` and `krun_set_vm_config(ctx,vcpus,memory_mib)`, then
`krun_set_kernel(ctx,kernel.path,kernel_format,initramfs.path,"rdinit=/init panic=-1")`.
The command line is fixed on both admitted architectures. The initramfs is an uncompressed
Linux newc archive with protected regular entries named `init` and `plasmosome/guest-policy`,
materialized at `/init` and `/plasmosome/guest-policy` respectively.
The former is the publisher's trusted PID1/shim/loader; the latter contains exactly the bytes
of guest_policy, not a host pathname or mutable lookup. Before launch verify that archive
binding against the pinned artifact and reject duplicate, escaping, symlinked or mismatching entries.
PID1 remains in the protected initramfs, mounts the single unpartitioned ext4 `/dev/vda` at
`/workload`, establishes the effective guest policy from that retained policy file, and only
then creates the workload's controlled mount and network namespaces and its control group. It starts
no workload at boot. Every workload process is started by a 4090 `exec` (spec 024), with
`/workload` moved over `/` in those namespaces.
Before hello it also makes the software directories of spec 027.
It does not replace itself with a workload-root executable or make its original root/policy
accessible to the workload. Managed guest paths are relative to that workload root. Host-only
RuntimeRecipe fields are never injected into it. Add exactly one disk, first and only,
with `krun_add_disk3(ctx,"root",writable_root,KRUN_DISK_FORMAT_RAW,false,false,KRUN_SYNC_FULL)`.
Thus block ID, guest device, writable mode, host-cache mode and sync mode are fixed, not
publisher-private choices. Do not call the deprecated root-disk API or implicit-init remount.
Call `krun_disable_implicit_init` and `krun_disable_implicit_console`; add no console. Pass an
explicit empty **guest-executable** environment with `krun_set_env(ctx,empty)` where empty is
a non-null array containing only the terminating NULL, not NULL (which copies the host environment).
Only the declared descriptors survive exec. Do not export a host directory through
`krun_set_root` or virtiofs. Call `krun_disable_implicit_vsock` before `krun_add_vsock(ctx,0)`;
zero disables both TSI flags. Add no NIC, TAP, passt or gvproxy backend, and call
`krun_set_port_map(ctx,empty)` with the same explicit empty-array convention.
Omitting a NIC alone does not disable implicit TSI. A negative context ID or nonzero setter
status refuses launch and cleans only this attempt's original resources; no failed call is skipped.
Source for these API constraints is the pinned
[header](https://github.com/libkrun/libkrun/blob/v1.19.4/include/libkrun.h) and
[implementation](https://github.com/libkrun/libkrun/blob/v1.19.4/src/libkrun/src/lib.rs).

Use `krun_add_vsock_port2(ctx,4090,control_path,false)` and corresponding 4091 data mapping
before start. They are fixed guest-initiated mappings; logical grants change over continuing
streams without altering the VM's boot mappings or rebooting it. Port4041 is not reused.
Guest CID3 is not cross-cell identity. The host authenticates through the original per-VMM
mapping/connection and owned private endpoint, not guest-supplied cell/owner/UID/PID fields.
Paths must remain under owned nonreplaceable directories; reconnect cannot silently attach a
replacement listener. The host's private recovery socket is not exposed by either mapping.

The trusted guest shim alone owns these channels, its mount/network namespaces and kernel-policy
loader. Workload uid1000 runs without capabilities, with no-new-privs, and without privileged
inherited FDs or authority to open AF_VSOCK, mount/setns, load BPF or ptrace the shim. Each such
denial returns an ordinary error to the caller and never kills the process (spec 024). Establish
the kernel boundary before workload launch and refuse unsupported enforcement. A poisoned model
or harness is not asked to cooperate. Keep the existing subject attestation/code110 requirement
and10.29.0.0/24 subject range; this runtime does not implement credential custody or attestation
by asserting a guest identity.

Every bridge stream uses §1's UTF-8 ndjson envelope, matching request IDs and maximum frame size.
Their method/parameter/result records are closed: all fields below are required, unknown fields
or methods refuse, and failures use `{code:105,message,from:"guest_request",to:"refused",detail}`
rather than a success-shaped empty result. Framing failures retain §1's protocol codes.
`deadline_ms` is a positive remaining
budget, never renewed internally. `boot` is the original association's opaque nonempty token.

4090 is private control, not another controller API. The shim opens two connections to that
same mapping: first the normal lane, then after its hello the prioritized withdrawal lane.
The original host supervisor is sole RPC caller and assigns that role in hello; the shim is
the responder. ControlLane is exactly `normal | withdrawal`. Normal admits the table's verbs;
withdrawal admits only hello, observe and remove. No install, activate, drain, shutdown, exec,
exec_status, exec_output or workload_stop may enter the withdrawal lane. Each lane has its own
bounded dispatcher and request IDs; several normal requests may be outstanding at once; spec 024
says which wait for an outstanding mutation, and none blocks withdrawal dispatch. A wait in normal
drain must not hold a lock or executor needed by withdrawal. Readiness requires both hellos, the
data association, the workload namespaces and the workload control group; no workload process exists
before readiness. On 4091 the shim is sole caller and the host the responder,
within that same verified original VMM association. No stream uses notifications or accepts
opposite-direction requests. Bounded data requests may overlap with distinct IDs. Actual loss
of any of these streams closes host admission and moves every affected operation out of standing state
into `IncompleteEffect`, retaining its original operation, resources and any issued record.
The guest closes its corresponding admissions when it observes the loss; delayed detection
cannot bypass the already-closed host gate. Never report a closed complete holding as effective,
place one address in both collections or hide its resources. This is a transport/enforcement
fault, not a graceful drain timeout. A controller restart does not close these supervisor-owned
streams. Reconnection first repeats hello and complete observation of the same original boot
and associations; it permits only exact cleanup of these incomplete effects, not activation,
reinstallation or promotion. A changed boot cannot take this path or recreate lost authority.
Connection-local data-handle loss remains distinct from the original grant/resource identity.

| Method | Params | Result |
| --- | --- | --- |
| `hello` | `{lane:ControlLane}` | `{boot:String, policy:String}` |
| `observe` | `{deadline_ms, snapshot:u64, offset:u64}` | `ObservationPage` of `GuestObservation` |
| `install` | `{operation:UniverseOp, address:String\|null, deadline_ms}` | `{boot, grant, installed:true}` |
| `activate` | `{grant:GrantId, deadline_ms}` | `{boot, grant, active:true}` |
| `drain` | `{grant:GrantId, deadline_ms}` | `{boot, grant, drained:true}` |
| `remove` | `{inverse:UniverseRemoval, owner:CellOwner, force:bool, deadline_ms}` | `{boot, grant, guest_removed:true}` |
| `shutdown` | `{deadline_ms}` | `{boot, shutdown_requested:true}` |
| `exec` | `{argv:[String], request_key:String\|null, deadline_ms}` | `{boot, exec_id:String, state:ExecState}`, where `ExecState` is `running \| exited \| signaled` (spec 024) |
| `exec_status` | `{exec_id:String, wait:bool, deadline_ms}` | `ExecStatus` (spec 024) |
| `exec_output` | `{exec_id:String, deadline_ms}` | `{boot, exec_id, stdout:Stream, stderr:Stream}` (spec 024) |
| `workload_stop` | `{deadline_ms}` | `{boot, stopped:true}` |

Before the first connection the trusted PID1 obtains32random bytes from the guest kernel,
waiting for initialized cryptographic randomness, and retains their64lowercase-hex encoding
as this boot token. It never writes the token to workload storage or reconstructs it from a
journal. Both control lanes return that same token; a reboot produces a fresh one. The first
host association is authenticated by the original VMM mapping/listener, not by the token.
Subsequent hellos must match the retained boot. Policy is the SHA256 of the protected guest
policy bytes whose loading and effective hooks/maps the trusted shim actually verifies before
hello; it must equal the original supervisor's guest_policy.sha256. A requested-but-unloaded
policy refuses. The supervisor independently verifies host RuntimeRecipe, helper, libraries,
kernel/initramfs and root authority; hello neither returns unexplained host paths nor pretends
to measure them. Its guest policy/boot account still requires the independent effective-state
observation below and is not an attestation substitute.

Install accepts only one of spec017's five complete grant operations, never an inverse.
It carries all original capability/owner/ID
fields, including the full recipe; it is not a mutable guest-side lookup. Only the original host
supervisor may issue it, after the corresponding cell prepare is durable and host original
resource authority has been retained. An address is the original per-boot ProxyMap binding
specified below; it is null for every other class. Conflicting reuse of an ID or address refuses.
The shim creates the actual grant-bound filesystem/listener/route attachments with admission
closed (`staged`), before returning installed:true. Broker data is exposed at the reserved guest
Unix path `/.plasmosome/brokers/<hex-encoded UTF-8 name>.sock`; this exposes only its data relay,
never its host control socket. Unrepresentable names/paths and cross-class projection collisions
refuse in preflight; no truncation, unowned path replacement or implicit dependency is allowed.

The host then enables its exact gate and calls activate. The shim enables this existing binding
and the spec017 selector; only independently observed active bindings plus the real host
resources complete `apply`. Install/activate acknowledgements are not this independent proof.
This is per-effect visibility after prepare and before commit, not an atomic workload-access
transaction. Commit atomically publishes desired state, not bytes already acquired. A later
effect failure durably aborts before withdrawing all newly created complete/incomplete
bindings in reverse prepare order; retained holdings remain intact. Spec008's preflight refusal
of a reload that would widen access or disturb retained holdings still applies.

Graceful removal uses a reversible **host-local** admission pause until the original host
monotonic deadline. Every 4091 admission check treats that pause as expired at that deadline;
restoration needs neither a timer callback nor a guest RPC. The existing guest binding and
selector remain active during this reversible phase. The host admits no new actual IO while
paused and waits for already-admitted host work. `drain` only observes/waits for the guest
requests pending when that control request arrived; it neither closes a guest gate nor destroys,
rebinds or forgets any resource. Later attempts cannot perform host effects through the paused
gate. Guest drain and host drain share the one remaining budget.

**Lifetime fence (spec 028).** The supervisor gets each grant's bound and idle limit with its
apply (spec 001 §4.1), before activation. It takes an earlier bound at any time, and a later one
only from a published DesiredCell (spec 028 section 8). From B (spec 028 section 1) on, the host
gate of every grant in the attachment admits no request, on any reference, opened before B or
after. The gate reads the supervisor's clock at every admission, as spec 001 §4.2's graceful
pause does, so the fence needs no timer and no controller. It refuses before any OS effect, with
spec 001's `grant_inactive`. It caps each admitted request's work at B + D, and when B moves
earlier, work already admitted is cut to the new B + D.

For the idle limit, the gate keeps the continuous time of the last request it admitted for the
attachment; a request still in flight is not use, and sleep counts as idle. It fences once that time
plus `idle_ms` has passed, checked at each admission and each observation, and the supervisor's
observation reports the idle deadline. With equal peers, fresh use goes to the smallest admitting
`GrantId` (spec 001), so the other can go idle.

The fence is permanent: no restored pause, late drain reply, renewal, publication or use reopens
it. `grant.select` treats a fenced grant as closed, so an equal peer of another attachment serves
fresh lookups (spec 017). The fence withdraws and finishes nothing, so it is not a removal path.

If the deadline expires before destructive progress, restore the local pause and return
DrainTimedOut with original objects, handles, guest admission and peers unchanged. A delayed
drain reply cannot authorize removal or change admission after that deadline. Do not close a
healthy stream merely because this observation timed out. The outstanding drain retains its
request ID until its eventual reply is discarded; later normal-lane mutations wait for that
request to settle or return a bounded refusal without effects. `exec_status` and `exec_output` are
not mutations; they and `workload_stop` never wait for one, and `exec` waits within its deadline and
then refuses as busy (spec 024). Later normal-lane mutations do not block Force's independent host
transition or the withdrawal lane. Actual stream loss is the separate incomplete transition above,
not an unproved remote-restoration result relabelled as a timeout. This preserves spec017 A4 and
spec008 R7 rather than weakening their pure-timeout rule.

Only while the deadline is still live and both drains have completed may the host irreversibly
close that selected grant's gate and send remove with force:false. This begins destructive
withdrawal: the guest verifies UniverseRemoval and the separate CellOwner against the retained
installed UniverseOp, closes its selected admission and destroys only those original guest
attachments. A timeout/lost reply after this point is incomplete, not a preserved timeout.
Force authority is checked/durably recorded by the host under spec008. Without waiting for
any normal-lane request, the supervisor atomically makes ONLY the selected operation
withdrawing/incomplete and closes its host gate permanently. A graceful-pause expiry, delayed
drain reply or cancelled callback cannot restore a non-standing operation. Peers keep their
existing gates, handles and selectors. Send the exact force:true removal on the independent
withdrawal lane; its guest handler closes selected admission and releases cancellable original
attachments without pretending uncancellable work is terminal. Even if that lane is delayed
or busy, the local Force transition has already denied selected access: return the selected
IncompleteEffect under the caller's bound and retain all cleanup authority, never a no-effect
busy refusal with the selected capability still active. Do not close a healthy shared lane
just to escape an unanswered request. Retried cleanup uses that same original association.
Withdrawal-lane observe/remove requests may not hold a peer's gate while waiting; pending work
cannot prevent another locally authorized Force from closing its selected host gate.
For either form, the shim returns guest_removed:true only after fresh observation proves the
selected guest bindings absent. A `revoked` Mount binding counts as absent (spec 024). This
guest-only acknowledgement never attests host cleanup.
The host's `apply_removal` succeeds only after fresh independent observation proves both those
guest bindings and every original host effect of the selected operation absent, including its
holding on shared resources. Equal peers and shared resources still required by them remain
intact; absence concerns this selected holding and its owned effects, not another grant's resources.
A guest acknowledgement, closed gate or unlinked listener alone cannot discharge the operation.
Any remaining effect or unconfirmed cleanup returns `BackendError::IncompleteEffect`, retaining
the original operation, associations, incomplete marker and remaining cleanup authority; never
reacquire by path or ID. Retire the marker and any issued record only under spec017's complete
absence rule; recovery must also observe absence from both standing and incomplete accounts
before finish or publication. A matching already-installed
or active operation may be acknowledged only after actual inspection and only while still
standing; no retry creates another attachment, rebinds an old handle or promotes an incomplete
operation. Install/activate refuse an address made incomplete by a lost stream or other failure.
Shutdown requests orderly guest exit; PID1 first stops the workload as `workload_stop` does, leaving
time to sync the root disk (spec 024); only actual original-child terminal/reap observation
establishes completion. Lost channels or handshakes are faults, not assumed graceful shutdown.

**Proxy projection.** The recorded `host` is a lower-case ASCII DNS name (nonempty1..63-byte
labels, total at most253bytes, letters/digits/hyphen with no leading/trailing hyphen); invalid
forms and numeric IP literals refuse before prepare without normalizing the capability.
`route` is an opaque exact
operator label for its complete ProxyRecipe, not a guest-supplied CIDR, HTTP path or lookup key.
After prepare, the original host supervisor allocates one address per distinct host from
198.18.0.1 through198.19.255.254 in numeric order, retaining the binding through this boot and
never assigning it to a different
host or recycling it after removal. Exhaustion refuses materialization with normal rollback.
Install carries that address and the full operation to the shim. Equal-host grants share the
host/address mapping but retain independent effective rules. Conflicting operator-supplied
routes in the controlled guest namespace are rejected in preflight; an unrelated host default
route is not such a collision. The binding is original runtime authority, not a journal field
or a recovery guess.

On the first installed ProxyMap the shim creates the shared DNS listener (UDP/TCP127.0.0.53:53)
and synthetic-prefix TUN/route in the controlled guest namespace, initially admitting no grant.
The trusted image supplies that resolver address and no fallback. These are actual attachments
of the installed grant; later grants add their own independently gated bindings. It answers A
queries for effectively active exact hosts (ASCII DNS query case is ignored) with their assigned
address, once the host supervisor has selected a grant for the queried name's `proxy_host` target
through 4091 `grant.select` (below). DNS selection is per host: a query names no destination port or
transport, so a host has an answer while any of its ProxyMap grants, on any port and transport, is
eligible. AAAA has no data and unknown/inactive hosts return NXDOMAIN. No external DNS forwarding,
fallback route or arbitrary destination service exists. At a new TCP connection/UDP flow, the shim
maps the destination address to its bound host and calls 4091 `grant.select` with the `proxy_map`
target of that host and the flow's destination port and transport. The host supervisor returns the
smallest currently eligible GrantId among the holdings at that target (below). So a host held on two
ports, or over TCP and UDP on one port, serves each flow from the grant for its own port and
transport, and a flow on a port or transport that no eligible grant covers refuses with
`no_active_grant`. The shim still checks the flow against the selected recipe. A mismatch refuses
that flow alone, as after `no_active_grant`, and never falls through to another rule; the shim keeps
serving every other flow and stream and never exits over it. The mismatch is a named observation
fault on the selected grant: like a mismatched binding (below), it fails complete observation
instead of being hidden. A stale locally active grant is not an authority to bypass this host
selection. It binds the actual guest flow and 4091 stream/datagram handle to that original grant.
Existing flows never follow a later selector change. An old holding and the activated replacement of
a reload in progress (spec008) at one host, port and transport can both be eligible, and their
routes and recipes can differ; nothing here assumes them equal. Selection does not read generations:
a new flow binds to the smallest eligible GrantId at its target, as at any other time, which may be
the old holding, the replacement or a peer at that target. Spec008's reload preflight still applies.
After commit the old holding stays eligible until its own retirement, and fresh selection excludes
it only while its host gate admits no new IO: a graceful pause that times out returns it, with its
possibly wider recipe, to selection, and the reload has not succeeded until finish. After an abort
the replacement likewise stays eligible until its own withdrawal. A flow already bound to any of
these grants stays bound to it and closes with that grant's drain or removal; it is never moved to a
survivor. Each UDP five-tuple is one flow until closed;
drain/remove closes that grant's flows, not a peer's. Last active-host removal withdraws its DNS
answer and forwarding rule; stale cached addresses refuse rather than reaching a reused host.
The shared resolver/TUN/route persist while peer bindings exist; last ProxyMap removal destroys
them and verifies their absence. Each actual DNS listener, TUN, route, per-grant rule and flow is
independently inventoried, including a rule with no flows. Workload edits of resolver
configuration grant no bypass.

4091 carries bounded selection and data operations. Every params object contains `deadline_ms`.
Except for grant.select, it also contains `grant:GrantId`, plus the fields in the table.
A host-issued `handle:u64` is nonzero,
connection-local, never recycled on that connection and bound to the exact original grant.
Exhaustion refuses a new open. Amounts/offsets are unsigned64-bit values checked for overflow;
one byte payload/read is at most65,536bytes, also subject to the enclosing frame limit.

| Method | Additional params | Result |
| --- | --- | --- |
| `grant.select` | `{target:SelectionTarget}` (no grant parameter) | `{grant:GrantId}` |
| `file.open` | `{components:[String], access:FileAccess}` | `{handle}` |
| `file.read` | `{handle, offset, length}` | `{bytes:[u8], eof:bool}` |
| `file.write` | `{handle, offset, bytes:[u8]}` | `{written:u64}` |
| `file.close` | `{handle}` | `{closed:true}` |
| `file.stat` | `{handle}` | `FileMetadata` |
| `file.truncate` | `{handle, length}` | `{length}` |
| `file.sync` | `{handle}` | `{synced:true}` |
| `fs.lookup` | `{components:[String]}` | `FileMetadata` |
| `fs.create` | `{components:[String]}` | `{handle}` |
| `fs.mkdir` | `{components:[String]}` | `FileMetadata` |
| `fs.remove` | `{components:[String], directory:bool}` | `{removed:true}` |
| `fs.rename` | `{source:[String], target:[String], replace:bool}` | `{renamed:true}` |
| `directory.open` | `{components:[String]}` | `{handle}` |
| `directory.read` | `{handle, cursor:u64, limit:u32}` | `{entries:[DirectoryEntry], cursor:u64, eof:bool}` |
| `directory.close` | `{handle}` | `{closed:true}` |
| `stream.open` | `{}` | `{handle}` |
| `stream.read` | `{handle, length}` | `{bytes:[u8], eof:bool}` |
| `stream.write` | `{handle, bytes:[u8]}` | `{written:u64}` |
| `stream.close` | `{handle}` | `{closed:true}` |
| `datagram.open` | `{}` | `{handle}` |
| `datagram.send` | `{handle, bytes:[u8]}` | `{written:u64}` |
| `datagram.receive` | `{handle, length}` | `{bytes:[u8], truncated:bool}` |
| `datagram.close` | `{handle}` | `{closed:true}` |

`SelectionTarget` is a closed record whose fields follow its kind. A `proxy_map` target is exactly
`{kind:"proxy_map", key:String, port:u16, transport:ProxyTransport}`; a target of any other kind is
exactly `{kind:SelectionKind, key:String}`. A `port` or `transport` field on another kind refuses
like any unknown field, and a `proxy_map` target missing either refuses like any missing field.
SelectionKind is exactly `session_file | uds_socket | proxy_map | proxy_host | broker | mount`; key
is respectively the recorded guest_path, guest_path, host, host, name or target. `port` is nonzero
and ProxyTransport is spec017's `tcp | udp`. Strings retain their exact recorded identity and
validation rules. A holding is at a target when its class is the kind's class (`proxy_map` and
`proxy_host` both name ProxyMap) and the recorded field listed above for that kind equals key;
spec017's diagnostic `key()` plays no part. A ProxyMap holding is at a `proxy_map` target only when
its recipe's `port` and `transport` also equal the target's, and at a `proxy_host` target whatever
its port and transport. A `proxy_host` selection serves only the DNS answer above; its result binds
no flow or data handle. Selection searches only this authenticated cell's original standing holdings
whose host gate currently admits new IO, and returns the lexicographically smallest full GrantId
among the holdings at that target. It never creates a holding or selects an incomplete, closed,
staged or foreign object. If no grant is eligible, return code105 with
`detail:{kind:"no_active_grant",target:SelectionTarget}`, never an invented ID.

The trusted shim must make this host selection before each fresh unqualified path lookup,
DNS answer or new flow, before local recipe/permission/transport checks. A guest-side eligibility
cache or pre-opened peer handle cannot substitute for it. The returned ID must match an actual
local binding at that target; a missing/mismatched binding is an observation fault, not permission
to invent one. Thus Force's local gate transition immediately excludes A from fresh selection
even while its guest cleanup is delayed, and a surviving eligible B still serves fresh requests.

Selection does not reserve future admission. If a selected grant becomes non-admitting before
the following operation is admitted, the host may return code105 with the closed
`detail:{kind:"grant_inactive",grant:GrantId}`. This refusal is issued only before admission or
any OS effect. Only a fresh unqualified operation whose concrete binding has not yet been
exposed may repeat grant.select under the original request deadline, never retrying an already
tried ID. Exhaustion/deadline or no_active_grant refuses. Recipe/transport/permission mismatch,
IO failure, unknown/foreign handles and any post-effect result do not authorize fallback or
replay. An already-bound inode, file/directory handle or flow propagates refusal instead of
switching grants; a stale pathname resolution must be revalidated as a new lookup, never by
rebinding the old inode. This is bounded selection, not another lifecycle or recovery API.

`FileMetadata` is `{inode:String, kind:FileKind, size:u64, modified_ns:i64}`;
FileKind is `file | directory`. The opaque inode identity remains bound to this grant and
original object while references survive; it is not a raw host inode reused across roots.
DirectoryEntry is `{name:String, metadata:FileMetadata}`. Directory handles retain the original
opened directory; cursor0 starts enumeration, later cursors are only those returned on that
handle. Limit is1..256 and reply bytes must still fit the frame. Unsupported/non-UTF-8 names
or enumeration failure return an error, never an apparently complete listing with missing rows.
Concurrent authorized directory edits may affect later pages; this is not a content snapshot.

Components are relative to the declared Mount root; each is nonempty, is neither `.` nor `..`,
and contains no slash, backslash or NUL. SessionFile requires an empty list for file.open and
selects its exact inode; filesystem namespace/directory methods require Mount. Symlinks and
non-file/non-directory objects refuse. Namespace creation is exclusive (file0600/directory0700);
removal and rename cannot address the root, cross grants or follow a substituted parent.
Rename with replace:false refuses an existing target. No method changes host ownership or
creates device nodes, hardlinks or symlinks. Mutating methods require read_write; requested file
access cannot exceed the recipe. Sync acknowledges actual synchronization of that original file,
not journal settlement. Stream/datagram opens use only the declared upstream or
ProxyRecipe with matching transport, never a supplied host destination. Wrong class, unknown
or foreign handles, closed grants, invalid lengths and overflow refuse before an OS action.
Returned byte/write counts describe actual IO, including short operations. Datagram truncation
is explicit; an empty timed-out read is not an invented EOF.

The 4091 data verbs grant no capability-lifecycle or recovery authority. Handles cannot transfer across channels or
rebind to a replacement grant. The host checks the effective grant on every request, bounds
admitted work and propagates real errors. Channel loss closes its original data handles but
does not manufacture grant withdrawal or clear remaining incomplete IO. Neither a service
acknowledgement nor a guest claim replaces independent resource cleanup observation.

`GuestObservation` is the strict record
`{boot:String, policy:String, fs_mounts:[GuestProjection], fs_handles:[GuestProjection],
socket_listeners:[GuestProjection], socket_streams:[SocketStreamProjection],
network:GuestNetworkObservation, software:SoftwareObservation}`. Every field and nested
array is required even when independently observed empty; omission of an otherwise-empty kind refuses
the entire observation. The nonempty boot token identifies this original supervisor/guest
association and is never reused or reconstructed from journal contents. Policy is the SHA256 of
the exact effectively loaded guest-policy artifact, not requested configuration.
GuestProjection is `{id:String, bindings:[GuestBinding]}`; its containing array fixes its kind.
GuestNetworkObservation is the strict record `{dns_listeners:[GuestProjection],
tuns:[GuestProjection], routes:[GuestProjection], proxy_rules:[GuestProjection],
tcp_flows:[GuestProjection], udp_flows:[GuestProjection]}`. All six arrays are required, including
empty arrays; coverage of one network kind cannot stand in for another. SoftwareObservation is
the strict record `{trees:[SoftwareTree], held:[TreeReference]}` (spec 027). SoftwareTree names
one tree visible in the workload namespace, with its digest and every name placed for it; its
field names follow spec 027's journal record. TreeReference is `{path:String, pid:u64,
comm:String, exec_id:String|null}`, one per reference into a detached tree that the guest's scan
sees. Both arrays are required even when empty. A tree or name with no committed or pending
journal effect, or a committed effect with no tree or name, fails observation. A `held` row is
not a binding or residue, and never makes a removal incomplete. DNS listeners appear
only in network, not again in socket_listeners; the latter contains UdsSocket/Broker data
listeners. Network infrastructure first created by install and shared by several grants carries
each actual binding; it cannot persist with an empty binding list after last removal.
`socket_streams` independently inventories accepted UdsSocket and Broker data streams, not
just their listeners. `SocketStreamProjection` is `{id:String, grant:GrantId, ends:[SocketStreamEnd]}`;
SocketStreamEnd is `{side:StreamSide, id:String}`, with StreamSide exactly `accepted | relay`.
The ends array contains each still-present actual guest accepted endpoint and its original
per-stream guest bridge binding, at most one of each and at least one. These are original
resource identities, not a pathname, reused FD number or the shared 4091 channel mistaken for
one stream. A half-cleaned pair remains with its surviving end; it disappears only after both
are freshly verified absent. A missing required socket_streams array is not an empty inventory.
The supervisor independently joins that original bridge binding to its retained host relay
endpoint and inspects the actual host end too; the guest cannot attest to host cleanup.
Unknown or lost associations fail observation instead of inventing IDs or omitting the pair.
Host gate closure, listener unlink or disappearance of only one end cannot complete withdrawal:
the operation remains incomplete until this guest pair and its original host access resources
are all freshly absent. Equal-peer streams keep their own grants and cannot be retargeted.
GuestBinding is `{grant:GrantId, admission:GuestAdmission}` and GuestAdmission is exactly `staged |
active | draining | closed | revoked`. `revoked` is a Mount binding whose backing is revoked under
spec 024: the guest serves nothing for its grant and the host side is closed, while workload
references to it may remain, and every use of them fails. A `revoked` binding accounts for its own
physical object, the aborted connection or the withdrawn grant's inodes on a shared connection, so
the inventory rules below still apply to that object. Once its removal has completed, it is exempt
only from host-authority association, residue, drift and readiness: it needs no retained host
authority, and it never blocks readiness or a later attach at the same target. The host checks,
against its own inventory keyed by GrantId (gates, source roots, handles, listeners and flows), that
it holds nothing for that grant, as withdrawal's absence check requires; a holding found there is an
unknown holding under spec 008. Admission describes the actually inspected gate, not the
last requested transition. IDs identify actual objects within this boot/namespace lifetime, not
logical capability addresses. Each binding is associated with independently verified original
host authority and the proper cell. Empty binding lists, duplicate object IDs within a kind,
duplicate bindings on an object, unknown objects, missing enforcement policy or an unverifiable
association are named observation faults, not rows invented from desired state. Actual
mountinfo, filesystem connection/handle state, listeners, rules and flows must agree with this
inventory. Closed/staged attachments remain visible until physically removed; a `revoked` binding
also remains visible until its last reference goes, and counts as absent for removal; a rule is
inventoried even with no active flow. A compromised workload cannot write this account; the host
separately verifies effective resource gates. Unknown or unattributable managed objects fail
complete observation rather than disappearing from stop/recovery.
The observational drain leaves guest admission active; `draining` is reserved for an already
irreversible withdrawal with admission closed and original work still pending.
Physical projections do not create new logical granted classes or authorize cleanup by ID alone.

**Platform admission.** Darwin uses Apple Silicon/macOS14+ with Hypervisor.framework, a correctly
signed VMM helper carrying `com.apple.security.hypervisor`, and a deny-default seatbelt policy
applied before guest execution. The allowlist is restricted to the chosen artifacts, private
per-cell writable image, required hypervisor operations and the two declared bridge endpoints.
Close all other inherited descriptors first: a pathname policy is not proof about existing FDs.
An actual Darwin25.6 unprivileged witness established allowlisted file/UDS use and EPERM on
fresh forbidden file/UDS/TCP access, but inherited file and connected sockets remained usable.
Removing those inherited FDs restored denial; an allow-default mutant exposed all forbidden
resources. This establishes that narrow mechanism, not an HVF/libkrun-compatible profile.
`sandbox_init` is deprecated and its raw-profile interface is not a supported portable API
guarantee. The publisher must supply and prove a compatible signed helper/policy on the supported
host; an unavailable or incompatible confinement refuses launch, never falls back unrestricted.

Linux requires matching KVM hardware/kernel and operator-delegated `/dev/kvm` access, not blanket
root for the workload. Confine the helper with separate host UID/namespaces, an allowlisted
read-only launch root, only owned writable images/endpoints and no ambient host network access.
The publisher supplies the matching Linux guest kernel/initramfs, effective filesystem/LSM
policy and userspace shim; the operator supplies the supported host and authorized fixture
roots/upstreams. No available Docker CLI, compiled kernel option or entitlement alone proves
this runtime. Missing artifacts, unavailable sandbox/KVM/LSM or a failed confinement test refuse
launch and are reported as concrete prerequisites, never as a simulated ready cell.

Acceptance requires actual hardware guests on both platforms, all five real adapters, denied
private-control access and ambient host file/network access, retained original authority across
controller restart, and spec017's managed-mmap/copy-execution witnesses. Disable TSI protection,
omit confinement, leak a privileged FD/loader environment or bypass host grant gating in
disposable mutants: the corresponding real unauthorized-access witness must fail. Check actual
loaded dependencies, protected initramfs-policy binding and both control-lane boot identities;
neither a guest echo of host paths nor post-main environment cleanup supplies that evidence.
Verify the declared host-policy hash, original installation binding and effective confinement at
readiness and recovery. Substituting another policy, even one called deny-default, must fail.
A small sandbox/mmap/loader/priority-lane probe establishes only the mechanism it exercised,
not libkrun/HVF compatibility or whole-cell isolation.

## 5. What this spec deliberately leaves undecided

- The plasmid WIT world (SDK surface) — deferred by design; `plasmid-sdk` is a reserved crate
  with a placeholder world.
- `cell.clone` / `cell.save` / `cell.load` / `freeze` (D1c tiers 2–3), genome
  `new/show/lint/test/export` details beyond D1's one-line definitions, and exec output streaming.
- Remaining VMM/shim and broker lifecycle verbs beyond §4.2's selected launch, observation, data and
  shutdown contract and spec 024's exec and workload verbs. Their concrete implementation remains
  required; the selected runtime and bounded bridge schemas are no longer undecided substitutes for
  an actual cell.
- Multi-instance brokers, remote orchestration, multi-tenancy — out of scope per 90.

## 6. How much of this is delivered

Item by item, what is true of the tree today. An item that is not yet true says so. None of them
is a claim that the text above may not be corrected.

1. Every verb above has a passing round-trip test against the real controller (ndjson in,
   typed result out) — **true of one verb**. `plasmosomed` exists: it reads a config naming its
   control socket and its instance, serves the §1 envelope on that socket, and answers
   `plasmosome.status` (§3.3) from an empty cell registry at ledger generation 0. Every other
   verb in §3 is **not yet** — nothing serves them. The socket path in §1 is not yet the one it
   binds either: the daemon takes the path from its config, and the
   `~/.plasmosome/instances/<name>/` convention arrives with `plasmosome.start`.
   The §1 control-socket lifecycle is **delivered** by both this daemon and `membraned`
   (§4): refuse-occupied paths, then no-follow device-and-inode best-effort cleanup of the
   bound socket only, with all stated host conditions and race limits. Measured reason for the
   correction: the delivered daemons' teardown used to unlink whatever entry sat at the
   configured pathname — a caller-settled regular file left there after renaming the original
   socket was deleted on ordinary `SIGTERM` shutdown in both daemons — contradicting the
   earlier unconditional claim that cleanup never removes a path the daemon did not create.
   That claim was wrong as written and is corrected in §1; the correction is the reviewed
   reason this spec changes here, per spec012's rule that a spec is corrected with the reason
   when reality shows its design wrong.
2. The error code table is closed and every code has a structured-field spec — **delivered**
   (§1).
3. The controller-side wire types are serde and share no memory — **delivered**, and true of the
   tree today, held by review rather than by a test: the guards that asserted it were removed
   with spec 013, which says why.
4. The D2 mock-mode field appears in every plasmid-carrying response — **delivered** (§3).
5. Ambiguity-as-error with candidate lists is the only selection semantics. Instance selection
   (spec 025 §5) is **delivered** by `plasmosome status`, the one command the `plasmosome`
   binary serves: exactly one instance is the default, and more than one, reachable or not, is
   refused with `ambiguous_instance` and the candidate list. Cell selection (§2) is **not yet**:
   code `100` and its `candidates` field exist, but no served verb takes a cell, so nothing
   resolves one. Amended on 2026-10-21, when spec 025's amendment A gave instance selection to
   the command line (plasmosome-6yev). Until then this item read
   "Ambiguity-as-error with candidate lists is the only selection semantics — **delivered** (§2)."
6. Ledger replayable-from-log and residue-empty as standing rows — ledger property green
   today (rule 3); the D4 residue row re-points at the membrane in P1 step 2.
7. Spec008's recovery journal, typed cell ownership, instance-root startup, recovery diagnostics
   and exact-operation/observation messages in §4.1 are **not yet implemented**. Their acceptance
   requires actual controller restart and independently surviving supervisor observation,
   separately from portable journal/model evidence. Accepting the contract does not turn the
   existing status-only daemons into recovery-capable daemons.
8. Spec017's PID-free BrokerLaunch and single-plugin format3 cutover are **delivered** in the
   exact-ID model and the ledger, and its caller-prepared fallible grant API is **delivered** in
   the exact-ID model. Its complete fallible snapshot, typed incomplete effects and drain-aware
   exact cleanup are **not yet implemented**.
   Current exact-ID model code carries recipe-bearing capabilities, a PID-free broker launch and
   format3 records. Its grant takes the operation and kind its caller prepared, keeps that
   address, mints no identity and can fail; every failure it has today comes before any effect.
   No product code calls grant yet. In tests, the testkit's attach-detach fixture writes the
   inverse of every prepared grant to a format3 log before the first grant. The new contract permits
   complete prepare/inverse before fork and defines cleanup after partial launch; it does not
   deliver an independent observer, workload confinement or all five real adapters.
   Amended on 2026-10-20, when plasmosome-8pr4 shipped the launch and the format3 cutover. Until
   then this item listed both as not yet implemented, and said the model code carried
   PID-bearing capabilities and format2 records.
   Amended on 2026-10-21, when plasmosome-kpm shipped the caller-prepared fallible grant. Until
   then this item listed it as not yet implemented, and said the model code still had
   infallible backend-minted grants.
9. The selected hardware runtime and physical guest projection account in §4.2/spec017 are
   **not yet implemented**. The exact-ID model and the ledger carry spec017's complete resource
   recipes as values, but nothing yet resolves a need into one or materializes one. Actual guest
   artifacts, platform confinement and strict managed-file policy witnesses remain
   deployment/admission obligations. Accepting their specification neither changes the
   status-only daemon nor completes spec008. Amended on 2026-10-20, when plasmosome-8pr4 shipped
   those recipe values. Until then this item listed fully resolved resource recipes as not yet
   implemented.
