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
  has no `genome` key (§3.3, §3.6).

Request envelope:

```json
{"id": 7, "method": "cell.status", "params": {"kernel": "work", "cell": "cell-1"}}
```

- `id` — client-chosen, echoed verbatim in the response. Any JSON value.
- `method` — `<noun>.<verb>`, dotted, lowercase.
- `params` — always an object (never omitted; empty object when the verb takes nothing).

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
| 104 | `mock_mode_conflict` | `node`, `modes`, `plasmids`, `resolutions` — per D2b rule 3 |
| 105 | `illegal_state` | `from`, `to`; private recovery methods additionally carry the typed `recovery` refusal in §4.1 |
| 106 | `drain_timeout` | `handle`, `deadline_ms` |
| 107 | `not_running` | `target` — the named instance is not up |
| 108 | `manifest_invalid` | `detail`, `path` |
| 109 | `widening_forbidden` | `plasmid` |
| 110 | `attestation_required` | `verb` — the E13b residual: subject spawn needs host-side attestation |

## 2. Naming and addressing (D1a/D1b/D1c)

- A **kernel instance** has a name (`plasmosome start --name work`); its state lives at
  `~/.plasmosome/instances/<name>/` (D1b). Verbs in the `plasmosome.*` group address the kernel
  organism; verbs in the `plasmid.*` group address plugin attachments. **No plasmid verb can
  start the plasmosome** (D1a): if the addressed instance is not running, `plasmid.*` fails with
  code `107`, it never boots one.
- A **cell** is a running session: one plasmosome + its attached plasmids + the workload (D1c).
  Its declarative definition is a **genome**, `~/.plasmosome`-relative to the project:
  `.plasmosome/genomes/<name>.toml`; the share/export form is `*.genome.toml`.
- Addressing chains down `(kernel, cell, plasmid)`. `--kernel` / `--cell` are optional **only
  when unambiguous**. One running instance → default; otherwise code `100` with the candidate
  list. The server resolves; the client never guesses.

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
     "plasmids": ["github-pr [mock:simulate]", "model-provider [real]"]},
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
supervisor state; replay does not prove readiness. After successful recovery startup, `ready`
is false while the account contains drift, quarantine or unmatched effects. Quarantined cells
are absent from `cells` and remain named in `plasmosome.recovery`. A recovered unknown genome
is omitted, as §1 requires.

### 3.3a `plasmosome.recovery`

Read-only diagnostics for the most recent completed recovery observation:

```json
{"id": 30, "method": "plasmosome.recovery", "params": {"name": "work"}}
```

The result is `{name, ledger_generation, desired, tombstones, observed_cells, expected, drift,
unmatched, pending, quarantined}`. `desired`, exact objects, pending transactions and quarantine
reports use spec008's typed records with this wire projection: absent genome/cell/line,
operation and force fields are omitted; a journal change whose replacement is null becomes
`{plugin, remove: true}`, while a replacement change is `{plugin, replacement: ...}`.
These are response encodings, not changes to the required nullable journal fields.
`drift` retains `Diff { added, removed }`: added objects were observed but not desired; removed
objects were desired but absent. IDs, complete capabilities and cell/plugin owners are
preserved. This is not a canonical-equivalence comparison. Empty collections are `[]` or their
empty map, not an omitted failure. Quarantine raw names and paths are arrays of unsigned byte
values; human messages escape them and are not machine selectors. The same optional-field
projection applies to the DesiredCell sent to a membrane.

The call neither forces cleanup nor claims to refresh a failed observation. Name resolution
and wrong-name errors are the same as `plasmosome.status`. Recovery startup with incomplete
observation or blocked cleanup does not serve this or any other method: it exits with the
structured stderr diagnostic described in §4.1. Quarantine alone can coexist with a serving
controller when all live observations are complete; readiness remains false.

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

### 3.5 `plasmosome.cell.new` — wire method `cell.new`

```json
{"id": 5, "method": "cell.new",
 "params": {"kernel": "work", "genome": "researcher", "mock": "simulate"}}
```

```json
{"id": 5, "result": {"cell": "cell-3", "state": "germinating",
  "plasmids": ["github-pr [mock:simulate]", "workspace [real]", "model-provider [simulate]"]}}
```

- `genome` is optional; without it the cell starts empty (plasmids attach later).
- `mock` is optional; **bare `--mock` ⇒ `simulate`** (D2). When a genome is named, its
  `[plasmids.X] mock = …` table is the default layer; the request-level `mock` overrides it
  per D2's layering (genome table → `plasmid add --mock` → `plasmid reload --mock`).
- `plasmosome germinate <genome>` is the documented alias of `cell.new --genome <name>` (D1c).
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

### 3.7 `cell.kill`

Drain-by-default teardown of one cell; `--now` is the authority-class immediate kill.

```json
{"id": 8, "method": "cell.kill", "params": {"kernel": "work", "cell": "cell-1"}}
{"id": 8, "result": {"cell": "cell-1", "state": "dead", "drained": true, "residue": "empty"}}
```

With `"now": true` the reply carries `"drained": false` and the recorded operator assertion.

### 3.8 `cell.exec`

Run a command inside the cell. Requests an E13-style subject spawn when a `subject` is named.

```json
{"id": 9, "method": "cell.exec",
 "params": {"kernel": "work", "cell": "cell-1", "argv": ["git", "push"], "subject": "git"}}
```

```json
{"id": 9, "result": {"exec_id": "e-11", "state": "running"}}
```

Completion is asynchronous; `exec.status` (companion, same envelope) polls:

```json
{"id": 10, "method": "exec.status", "params": {"kernel": "work", "cell": "cell-1", "exec_id": "e-11"}}
{"id": 10, "result": {"exec_id": "e-11", "state": "exited", "exit_code": 0, "duration_ms": 1823}}
```

- A subject spawn is a **host-side attested** request (the E13b residual: the wire narrows to
  this one verb); an unattested spawn subject is refused with code `110`.
- RESERVED: streaming stdout/stderr frames as ndjson on the same connection.

### 3.9 `plasmid.list`

```json
{"id": 11, "method": "plasmid.list", "params": {"kernel": "work", "cell": "cell-1"}}
```

```json
{"id": 11, "result": {"plasmids": [
  {"plasmid": "github-pr", "mock": "simulate", "generation": 3, "state": "active",
   "label": "github-pr [mock:simulate]"},
  {"plasmid": "model-provider", "mock": "passthrough", "generation": 3, "state": "active",
   "label": "model-provider [real]"}
]}}
```

`mock` ∈ `simulate | capture | passthrough` — the D2 vocabulary, closed; an addition to it is a
contract change. Absent declarations mean `passthrough`.

### 3.10 `plasmid.add`

```json
{"id": 12, "method": "plasmid.add",
 "params": {"kernel": "work", "cell": "cell-1", "plasmid": "github-pr", "mock": "capture"}}
```

```json
{"id": 12, "result": {"plasmid": "github-pr", "mock": "capture", "generation": 4,
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
- Inherited levels yield to the new explicit declaration (D2b rule 2). An explicit-vs-explicit
  conflict on the same node at different modes → code `104` with
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
  downgrade.

### 3.11 `plasmid.remove`

Drains by default (the no-residue rule: LIFO ledger replay over the F9 universe; the plasmid
never punishes detachment). `--now` is the authority-class immediate removal.

```json
{"id": 13, "method": "plasmid.remove", "params": {"kernel": "work", "cell": "cell-1", "plasmid": "github-pr"}}
```

```json
{"id": 13, "result": {"plasmid": "github-pr", "state": "removed", "drained": true,
  "replayed": 2, "delayed_discarded": 1, "residue": "empty"}}
```

Outstanding external effects refuse the safe removal with code `105` carrying the assertion
list; force requires the operator/reason pair exactly as `plasmosome.stop`.

### 3.12 `plasmid.reload`

Generation swap of an attached plasmid (reload = new generation, not remove+add); mock mode may
be changed in the same swap (D2's third layer).

```json
{"id": 14, "method": "plasmid.reload",
 "params": {"kernel": "work", "cell": "cell-1", "plasmid": "github-pr", "mock": "simulate"}}
```

```json
{"id": 14, "result": {"plasmid": "github-pr", "mock": "simulate", "generation": 5, "state": "active"}}
```

## 4. Controller ⇄ membrane (the supervisor side of the contract)

The controller drives each cell's `membraned` over a second, private ndjson-UDS
(`<instance>/cells/<cell>/membrane.uds`). Same envelope as §1. The subset this spec covers:

- `membrane.status` — the F9 readiness probe. Reply `{"ready": true, "state": "serving"}`.
  Readiness = the socket **answers**; accept-without-answer is the half-alive broker and is
  reported not-ready (measured in F9; implemented in `plasmosome-membrane::readiness`).
  A membrane that is **not** ready answers the same `result` object with `"ready": false` and a
  `state` naming which case it is. Every call asks every broker again; no answer is kept.

  | `state` | further fields |
  | --- | --- |
  | `not_serving` | `broker` — the one that held the set back; `reason`; and `broker_state` when, and only when, `reason` is `reported` |
  | `deadline_spent` | `unreached` — the first broker never asked; `asked` — those that spent the budget, in probe order, omitted when the deadline was gone before any broker was asked |
  | `empty` | none — a membrane supervising no brokers, which is never ready |

  `reason` is one of `unreachable`, `timed_out`, `malformed` or `reported`: the socket was not
  there, it did not answer inside the deadline, it answered something that was not a status, or
  it answered that it is not serving and said so in `broker_state`. Delivered in
  `plasmosome-membrane::{control, daemon}` and served by `membraned`.
- `membrane.cell.desired` — desired-state push, **idempotent and generation-numbered**: the
  full desired cell record plus `generation: u64`. An identical equal-generation request is
  a no-op; conflicting content at that generation refuses. An older request is ignored and
  returns the current published record, never an assertion that the older payload was published.
- `membrane.cell.observe` — the supervisor's observed state (cells, broker readiness, VMM
  liveness). This is the only source the controller trusts for liveness; `sessions.status`-style
  requested state is lifecycle, not liveness.
- `membrane.residue.snapshot` — the F9-universe observation taken **from the supervisor/broker
  side** at diff time (86 §4 rule 4). The controller diffs snapshots, never its intentions.
- `membrane.cell.kill` — drain-then-kill with `DrainSpec { deadline, policy }` carried
  verbatim from the seam types; the membrane owns the VMM child, shim, and brokers as **its**
  children — never the controller's (86 §4 rule 5), and per-cell vs per-host brokers is an
  explicit parameter in the desired record.
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
reset on retry. Exhaustion refuses startup or blocks further mutation of that cell; it is never
an empty observation or successful client result. JSON configuration rejects unknown keys as before.

The controller holds spec008's instance writer lock and discovers validated cell directories
before sending requests to their membrane sockets. A membrane is configured for one validated
cell and rejects a request naming another cell with code101. Each connection uses §1 framing,
request IDs and errors. Responses with a wrong request ID or cell, unknown fields/variants,
malformed exact objects, incomplete frames or timeout are failed observations. The controller
must not operate a quarantine's effects merely because its supervisor answered.

- `membrane.cell.observe` params are `{cell, deadline_ms}`. `deadline_ms` is a positive
  remaining budget capped by the controller's recovery deadline. Result is
  `{cell, state, supervisor, generation, desired}`: `state` is the existing closed cell lifecycle
  enum from actual child/supervisor observation, `supervisor` is the current `membrane.status`
  result, and `desired` is the complete last published DesiredCell retained by this supervisor,
  with `generation == desired.generation`. Generation and content are retained atomically.
  A fresh supervisor starts with the empty generation-0 record. Missing retained content is an
  observation failure, not permission to reconstruct it from the requesting controller.
  This publication record is not evidence of liveness or holdings: those still require actual
  supervisor and enforcement-side observation. No PID-file or socket-existence substitute is allowed.
- `membrane.residue.snapshot` params are `{cell, deadline_ms}`. Result is
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
- `membrane.effect.apply` params are `{cell, generation, operation, deadline_ms}`.
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
`writer_busy`, `discovery`, `identity_conflict`, `desired_conflict`, `observation`, `cleanup`, `deadline`, or `io`;
`path_bytes` is the exact related Unix path as byte values, omitted if no path applies.
For a generation observation refusal or equal-generation `desired_conflict`, recovery_error
additionally carries `cell`, `journal_generation` and `membrane_generation` as structured values.
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
`728df8125077d0db44265f6e997c72b81b65c015`. Configuration and `krun_start_enter` run inside that
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
pinned header's KRUN_KERNEL_FORMAT constants0 through5 respectively. The publisher proves the
selected format/architecture combination supported by the pinned library; no format guessing
or hidden command-line override is allowed. `writable_root` is an absolute NUL-free path under
an owned nonreplaceable private per-cell directory. Create that copy exclusively from root_image
before VMM launch and retain its original inode/authority; it is never the immutable source path
or a caller-selected existing file. This path is an explicit launch input, not recovery authority.
Use immutable verified kernel/initramfs/base artifacts and a separately owned RAW per-cell copy
for writes. Verification/opening must preserve the selected inode/bytes through the library's
path-based opens; a replaceable parent or mutable artifact is refused, not trusted after hashing.
A runtime recipe is deployment input retained by the original supervisor, not a mutable recovery
database or permission to recreate a lost VM. Recovery reconnects, not relaunches.
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
then starts the unprivileged workload chrooted into `/workload` in its controlled namespaces.
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

Use `krun_add_vsock_port2(ctx,4090,control_path,false)` and corresponding4091data mapping
before start. They are fixed guest-initiated mappings; logical grants change over continuing
streams without altering the VM's boot mappings or rebooting it. Port4041 is not reused.
Guest CID3 is not cross-cell identity. The host authenticates through the original per-VMM
mapping/connection and owned private endpoint, not guest-supplied cell/owner/UID/PID fields.
Paths must remain under owned nonreplaceable directories; reconnect cannot silently attach a
replacement listener. The host's private recovery socket is not exposed by either mapping.

The trusted guest shim alone owns these channels, its mount/network namespaces and kernel-policy
loader. Workload uid1000 runs without capabilities, with no-new-privs, and without privileged
inherited FDs or authority to open AF_VSOCK, mount/setns, load BPF or ptrace the shim. Establish
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
withdrawal admits only hello, observe and remove. No install, activate, drain or shutdown may
enter the withdrawal lane. Each lane has its own bounded dispatcher and request IDs; one normal
request may be outstanding without blocking withdrawal dispatch. A wait in normal drain must
not hold a lock or executor needed by withdrawal. Readiness requires both hellos and the data
association before workload launch. On4091 the shim is sole caller and the host the responder,
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
| `observe` | `{deadline_ms}` | `GuestObservation` |
| `install` | `{operation:UniverseOp, address:String|null, deadline_ms}` | `{boot, grant, installed:true}` |
| `activate` | `{grant:GrantId, deadline_ms}` | `{boot, grant, active:true}` |
| `drain` | `{grant:GrantId, deadline_ms}` | `{boot, grant, drained:true}` |
| `remove` | `{inverse:UniverseRemoval, owner:CellOwner, force:bool, deadline_ms}` | `{boot, grant, removed:true}` |
| `shutdown` | `{deadline_ms}` | `{boot, shutdown_requested:true}` |

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
monotonic deadline. Every4091admission check treats that pause as expired at that deadline;
restoration needs neither a timer callback nor a guest RPC. The existing guest binding and
selector remain active during this reversible phase. The host admits no new actual IO while
paused and waits for already-admitted host work. `drain` only observes/waits for the guest
requests pending when that control request arrived; it neither closes a guest gate nor destroys,
rebinds or forgets any resource. Later attempts cannot perform host effects through the paused
gate. Guest drain and host drain share the one remaining budget.

If the deadline expires before destructive progress, restore the local pause and return
DrainTimedOut with original objects, handles, guest admission and peers unchanged. A delayed
drain reply cannot authorize removal or change admission after that deadline. Do not close a
healthy stream merely because this observation timed out. The outstanding drain retains its
request ID until its eventual reply is discarded; later normal-lane mutations wait for that
request to settle or return a bounded refusal without effects. They do not block Force's
independent host transition or the withdrawal lane. Actual stream loss is the
separate incomplete transition above, not an unproved remote-restoration result relabelled as
a timeout. This preserves spec017 A4 and spec008 R7 rather than weakening their pure-timeout rule.

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
Neither form returns removed:true until fresh observation proves the selected
guest bindings absent. Host resource cleanup is separately required. Failures preserve original
associations and incomplete state, never reacquire by path or ID. A matching already-installed
or active operation may be acknowledged only after actual inspection and only while still
standing; no retry creates another attachment, rebinds an old handle or promotes an incomplete
operation. Install/activate refuse an address made incomplete by a lost stream or other failure.
Shutdown requests orderly guest exit; only actual original-child terminal/reap observation
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
of the installed grant; later grants add their own independently gated bindings. It answers
A queries for effectively active exact hosts (ASCII DNS query case is ignored) with their
assigned address, after host-authoritative grant.select below. AAAA has no data and
unknown/inactive hosts return NXDOMAIN. No external DNS forwarding, fallback route or arbitrary
destination service exists. At a new TCP connection/UDP flow, the shim obtains that host's
smallest currently eligible GrantId through4091grant.select before checking packet transport
and destination port against the selected recipe. Mismatches refuse, never fall through to
another rule. A stale locally active grant is not an authority to bypass this host selection.
It binds the actual guest flow and4091stream/datagram handle to that original grant. Existing
flows never follow a later selector change. Each UDP five-tuple is one flow until closed;
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

`SelectionTarget` is the closed record `{kind:SelectionKind,key:String}`. SelectionKind is
exactly `session_file | uds_socket | proxy_map | broker | mount`; key is respectively the
recorded guest_path, guest_path, host, name or target. Strings retain their exact recorded
identity and validation rules. Selection searches only this authenticated cell's original
standing holdings whose host gate currently admits new IO, and returns the lexicographically
smallest full GrantId at that target. It never creates a holding or selects an incomplete,
closed, staged or foreign object. If no grant is eligible, return code105 with
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

The4091data verbs grant no capability-lifecycle or recovery authority. Handles cannot transfer across channels or
rebind to a replacement grant. The host checks the effective grant on every request, bounds
admitted work and propagates real errors. Channel loss closes its original data handles but
does not manufacture grant withdrawal or clear remaining incomplete IO. Neither a service
acknowledgement nor a guest claim replaces independent resource cleanup observation.

`GuestObservation` is the strict record
`{boot:String, policy:String, fs_mounts:[GuestProjection], fs_handles:[GuestProjection],
socket_listeners:[GuestProjection], network:GuestNetworkObservation}`. Every field and nested
array is required even when independently observed empty; omission of an otherwise-empty kind refuses
the entire observation. The nonempty boot token identifies this original supervisor/guest
association and is never reused or reconstructed from journal contents. Policy is the SHA256 of
the exact effectively loaded guest-policy artifact, not requested configuration.
GuestProjection is `{id:String, bindings:[GuestBinding]}`; its containing array fixes its kind.
GuestNetworkObservation is the strict record `{dns_listeners:[GuestProjection],
tuns:[GuestProjection], routes:[GuestProjection], proxy_rules:[GuestProjection],
tcp_flows:[GuestProjection], udp_flows:[GuestProjection]}`. All six arrays are required, including
empty arrays; coverage of one network kind cannot stand in for another. DNS listeners appear
only in network, not again in socket_listeners; the latter contains UdsSocket/Broker data
listeners. Network infrastructure first created by install and shared by several grants carries
each actual binding; it cannot persist with an empty binding list after last removal.
GuestBinding is `{grant:GrantId, admission:GuestAdmission}` and GuestAdmission is exactly
`staged | active | draining | closed`. Admission describes the actually inspected gate, not the
last requested transition. IDs identify actual objects within this boot/namespace lifetime, not
logical capability addresses. Each binding is associated with independently verified original
host authority and the proper cell. Empty binding lists, duplicate object IDs within a kind,
duplicate bindings on an object, unknown objects, missing enforcement policy or an unverifiable
association are named observation faults, not rows invented from desired state. Actual
mountinfo, filesystem connection/handle state, listeners, rules and flows must agree with this
inventory. Closed/staged attachments remain visible until physically removed; a rule is
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
A small sandbox/mmap/loader/priority-lane probe establishes only the mechanism it exercised,
not libkrun/HVF compatibility or whole-cell isolation.

## 5. What this spec deliberately leaves undecided

- The plasmid WIT world (SDK surface) — deferred by design; `plasmid-sdk` is a reserved crate
  with a placeholder world.
- `cell.clone` / `cell.save` / `cell.load` / `freeze` (D1c tiers 2–3), genome
  `new/show/lint/test/export` details beyond D1's one-line definitions, and exec output streaming.
- Remaining VMM/shim and broker lifecycle verbs beyond §4.2's selected launch, observation,
  data and shutdown contract. Their concrete implementation remains required; the selected
  runtime and bounded bridge schemas are no longer undecided substitutes for an actual cell.
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
2. The error code table is closed and every code has a structured-field spec — **delivered**
   (§1).
3. The controller-side wire types are serde and share no memory — **delivered**, and true of the
   tree today, held by review rather than by a test: the guards that asserted it were removed
   with spec 013, which says why.
4. The D2 mock-mode field appears in every plasmid-carrying response — **delivered** (§3).
5. Ambiguity-as-error with candidate lists is the only selection semantics — **delivered** (§2).
6. Ledger replayable-from-log and residue-empty as standing rows — ledger property green
   today (rule 3); the D4 residue row re-points at the membrane in P1 step 2.
7. Spec008's recovery journal, typed cell ownership, instance-root startup, recovery diagnostics
   and exact-operation/observation messages in §4.1 are **not yet implemented**. Their acceptance
   requires actual controller restart and independently surviving supervisor observation,
   separately from portable journal/model evidence. Accepting the contract does not turn the
   existing status-only daemons into recovery-capable daemons.
8. Spec017's PID-free BrokerLaunch, caller-prepared fallible grant API, complete fallible snapshot,
   typed incomplete effects, drain-aware exact cleanup and single-plugin format3 cutover are
   **not yet implemented**. Current exact-ID model code still carries PID-bearing capabilities,
   infallible backend-minted grants and format2 records. The new contract permits complete
   prepare/inverse before fork and defines cleanup after partial launch; it does not deliver
   an independent observer, workload confinement or all five real adapters.
9. The selected hardware runtime, fully resolved resource recipes and physical guest projection
   account in §4.2/spec017 are **not yet implemented**. Actual guest artifacts, platform
   confinement and strict managed-file policy witnesses remain deployment/admission obligations.
   Accepting their specification neither changes the status-only daemon nor completes spec008.
