---
id: 001
title: Control protocol
status: accepted
intents: []
---

# Plasmosome control protocol — the P1 freeze

**Scope:** the P1 contract freeze (91 plan step 1). Everything binding below is
traceable to a decided item: D1/D1a/D1b/D1c/D2/D2b/D3/D4 in `91-p1-plan.md`, the frozen
credential grammar in `80-adr-credential-delivery.md` (D2 as confirmed by E4b, `[commands]` as
frozen by E13/E13b), the six must-not-bake-in rules in `86-kernel-process-topology.md` §4, and
F9's measured readiness rule. Undecided items are marked RESERVED and do not freeze here.

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
| 105 | `illegal_state` | `from`, `to` |
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

## 3. Verb schemas (frozen v1 set)

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
- RESERVED (not frozen): `cell.clone` (tier-2: state + plasmids, fresh brain), `cell.save` /
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

`mock` ∈ `simulate | capture | passthrough` — the frozen D2 vocabulary, closed. Absent
declarations mean `passthrough`.

### 3.10 `plasmid.add`

```json
{"id": 12, "method": "plasmid.add",
 "params": {"kernel": "work", "cell": "cell-1", "plasmid": "github-pr", "mock": "capture"}}
```

```json
{"id": 12, "result": {"plasmid": "github-pr", "mock": "capture", "generation": 4,
  "propagated": {"mode": "capture", "closure": ["github-pr", "mock-github"]},
  "attach": {"attach_to_first_allowed_ms": 57}}}
```

- `mock` optional; absent = the cell's inherited default (`passthrough` when nothing is
  declared). Setting a mode propagates across the plasmid's **whole dependency closure
  transitively** (D2b rule 1) — the reply reports what was propagated.
- Inherited levels yield to the new explicit declaration (D2b rule 2). An explicit-vs-explicit
  conflict on the same node at different modes → code `104` with
  `resolutions: ["--force-simulate", "--force-passthrough", "remove plasmid mock-github"]`
  (D2b rule 3 — never last-write-wins).
- Attach is the two-phase transaction over the Track B seam: validate the whole subgraph,
  then commit; a mid-commit failure rolls the prefix back and replies `CommitFailed`-shaped
  code `103`/`109` with the rolled-back list. Attach receipts carry the ledger generation so
  the reconciler converges on replay (86 §4 rule 2).
- Credential refs in the manifest freeze per ADR 80: `delivery` is an ordered non-empty list
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

## 4. Controller ⇄ membrane (the supervisor side of the freeze)

The controller drives each cell's `membraned` over a second, private ndjson-UDS
(`<instance>/cells/<cell>/membrane.uds`). Same envelope as §1. The frozen subset:

- `membrane.status` — the F9 readiness probe. Reply `{"ready": true, "state": "serving"}`.
  Readiness = the socket **answers**; accept-without-answer is the half-alive broker and is
  reported not-ready (measured in F9; implemented in `plasmosome-membrane::readiness`).
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

## 5. What this draft deliberately does not freeze

- The plasmid WIT world (SDK surface) — deferred by design; `plasmid-sdk` is a reserved crate
  with a placeholder world.
- `cell.clone` / `cell.save` / `cell.load` / `freeze` (D1c tiers 2–3), genome
  `new/show/lint/test/export` details beyond D1's one-line definitions, and exec output
  streaming.
- The membrane's VMM/shim/broker verb set (P1 step 2 owns it; §4 bounds its shape).
- Multi-instance brokers, remote orchestration, multi-tenancy — out of scope per 90.

## 6. Freeze checklist (what makes this "frozen")

1. Every verb above has a passing round-trip test against the real controller (ndjson in,
   typed result out) — not yet: the controller daemon is P1 step 3.
2. The error code table is closed and every code has a structured-field spec — done (§1).
3. The controller-side wire types are serde and share no memory — enforced by
   `plasmosome-freeze-checks` (86 §4 rules 1–3 green today).
4. The D2 mock-mode field appears in every plasmid-carrying response — done (§3).
5. Ambiguity-as-error with candidate lists is the only selection semantics — done (§2).
6. Ledger replayable-from-log and residue-empty as standing rows — ledger property green
   today (rule 3); the D4 residue row re-points at the membrane in P1 step 2.
