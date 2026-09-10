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
| 105 | `illegal_state` | `from`, `to`; private recovery effect methods additionally carry the typed `recovery` refusal in §4.1 |
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
  full desired cell record plus `generation: u64`. A membrane that receives an equal-or-older
  generation acks and does nothing (replayed reconciler converges instead of re-firing —
  86 §4 rule 2).
- `membrane.cell.observe` — the supervisor's observed state (cells, broker readiness, VMM
  liveness). This is the only source the controller trusts for liveness; `sessions.status`-style
  requested state is lifecycle, not liveness.
- `membrane.residue.snapshot` — the F9-universe observation taken **from the supervisor/broker
  side** at diff time (86 §4 rule 4). The controller diffs snapshots, never its intentions.
- `membrane.cell.kill` — drain-then-kill with `DrainSpec { deadline, policy }` carried
  verbatim from the seam types; the membrane owns the VMM child, shim, and brokers as **its**
  children — never the controller's (86 §4 rule 5), and per-cell vs per-host brokers is an
  explicit parameter in the desired record.
- RESERVED for P1 step 2: vsock bridge setup, shim lifecycle, broker spawn/supervision verbs,
  and the credential vsock proxy (port 4041 terminates at the membrane and proxies to the
  controller; custody state stays kernel-core).

### 4.1 Recovery startup and exact operation messages

These are the recovery additions specified by spec008, not claims that the existing membrane
daemon already serves them. Its existing `membrane.status` response and error codes remain.
The configured controller gains required absolute `instance_root` and positive integer
`recovery_deadline_ms`, alongside `control_socket` and `name`; no root is inferred from the
socket. Recovery has one monotonic deadline covering discovery, observation and pending
cleanup, not a new full budget per retry. Exhaustion refuses startup; it is never an empty
observation. JSON configuration rejects unknown keys as before.

The controller holds spec008's instance writer lock and discovers validated cell directories
before sending requests to their membrane sockets. A membrane is configured for one validated
cell and rejects a request naming another cell with code101. Each connection uses §1 framing,
request IDs and errors. Responses with a wrong request ID or cell, unknown fields/variants,
malformed exact objects, incomplete frames or timeout are failed observations. The controller
must not operate a quarantine's effects merely because its supervisor answered.

- `membrane.cell.observe` params are `{cell, deadline_ms}`. `deadline_ms` is a positive
  remaining budget capped by the controller's recovery deadline. Result is
  `{cell, state, supervisor, generation}`: `state` is the existing closed cell lifecycle enum
  from actual child/supervisor observation, `supervisor` is the current `membrane.status`
  result, and `generation` is the last completely published desired generation. No PID-file,
  socket-existence or requested-state substitute is permitted.
- `membrane.residue.snapshot` params are `{cell, deadline_ms}`. Result is
  `{cell, state: OsState, grants: [LedgerEntry, ...]}`. `state` is fresh enforcement-side
  observation, including unrequested residue, using spec017 identities and spec008 CellOwner.
  Every object and grant belongs to the addressed cell. `grants` names only original live grant
  records for which this surviving backend still has its drain/withdrawal authority; it is
  not rebuilt by planting observed objects. An unsupported or failed class observation fails
  the whole request with `-32603`, never omits the class or returns a requested-state mirror.
  The result contains the five currently specified capability classes; adding guest classes
  requires their explicit enumeration rather than a claim of coverage without an observer.
- `membrane.effect.apply` params are `{cell, generation, operation, deadline_ms}`.
  `operation` is a fully predeclared spec017 UniverseOp with spec008 owner. The trusted
  controller sends it only after that generation's prepare is durable. Result is
  `{cell, generation, id, applied: true}` after enforcement accepted the exact operation.
  Duplicate application of the same standing operation retains spec017's no-op semantics.
  This result does not replace the independent snapshot used for recovery verification.
- `membrane.effect.withdraw` params are `{cell, generation, removal, owner, drain, deadline_ms}`.
  `removal` is the exact spec017 UniverseRemoval, `owner` is CellOwner, and `drain` is the
  existing DrainSpec serde value. Result is `{cell, generation, id, withdrawn: true}`.
  The backend preserves all neighbours, drain behavior and incarnation checks. Unknown-object
  and unknown-handle refusals are not translated into success; the controller must freshly
  observe exact absence before completing an uncertain obligation.
- For either effect method, backend refusal is code105 with `from: "prepared"` or `"held"`,
  `to: "applied"` or `"withdrawn"`, plus `recovery: {kind, ...}`. The closed kinds and fields
  are `identity_conflict {class,id}`, `unknown_object {class,id,key,owner}`,
  `unknown_handle {handle}`, `drain_timeout {handle,deadline_ms}`, and
  `backend_fault {detail}`. Unsupported enforcement and unverified process incarnation are
  backend_fault, not successful model operations. The human message is never a selector.
  Protocol parse/parameter/internal errors keep §1's existing meanings.
- `membrane.cell.desired` params are `{cell, generation, desired}` with spec008's complete,
  settled DesiredCell. Result is `{cell, generation}` echoing the request generation.
  Equal/older generations acknowledge without changing anything. A newer generation publishes
  the full record only after its individual effects and cleanup have completed; it is not an
  instruction to infer missing operations, allocate replacement IDs or resurrect withdrawn
  holdings. An acknowledgement is not evidence of current readiness or snapshot contents.

Only the instance's trusted controller can use these private sockets; they are not exposed to
the cell workload. The writer lock serializes controllers, not arbitrary clients, and is not
authentication. Supervisor sockets and their directories stay outside cell write authority.
The supervisor retains exact resource associations independently of controller memory and must
verify resource/process incarnation before enforcing a withdrawal. No recovery RPC may create
an in-memory backend in production and report its ledger as an OS observation.

Before serving, the controller completes spec008 discovery, observation and pending recovery
within the deadline. An instance-wide fault emits one LF-terminated JSON object on stderr:
`{recovery_error:{kind,path_bytes,detail}, quarantined:[...]}`. `kind` is one of
`writer_busy`, `discovery`, `identity_conflict`, `observation`, `cleanup`, `deadline`, or `io`;
`path_bytes` is the exact related Unix path as byte values, omitted if no path applies.
Quarantine entries whose observation failed include `observation_error` and omit `found`;
an empty found list is reserved for an actually observed empty set or an invalid entry with
no attributable cell. Error exit removes only this invocation's control socket and releases
the lock; it neither kills the surviving cells nor rewrites their journals.

Spec008's R9/R10 acceptance exercises this actual socket path and independent enforcement-side
observation. A library-only fake or manually constructed Controller cannot demonstrate it.

## 5. What this spec deliberately leaves undecided

- The plasmid WIT world (SDK surface) — deferred by design; `plasmid-sdk` is a reserved crate
  with a placeholder world.
- `cell.clone` / `cell.save` / `cell.load` / `freeze` (D1c tiers 2–3), genome
  `new/show/lint/test/export` details beyond D1's one-line definitions, and exec output
  streaming.
- The membrane's VMM/shim/broker verb set (P1 step 2 owns it; §4 bounds its shape).
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
