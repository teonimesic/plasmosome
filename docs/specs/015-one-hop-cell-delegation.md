---
id: 015
title: One-hop typed delegation between isolated cells
status: draft
intents: [016, 011, 012]
---

## Behavior

A caller cell can establish a named, temporary edge to a callee cell and submit a typed operation
through that edge. The first supported route is Linux agent cell to native macOS executor cell.
The callee executes only an operation permitted by its profile and membrane, and returns a
structured result and ledger record. The caller receives no ambient access to the callee or host.

An edge can be detached independently of either cell. Detachment revokes admission of new work
on that edge but does not retroactively erase work already admitted. The admitted operation either
completes and returns its result or is explicitly reported as cancelled/failed; its process and
resources are reaped according to the callee lifecycle. A later operation requires a new edge
with a new generation.

Interactive cells have private input routing and output surfaces. An input directed to one cell
cannot be delivered to another cell, and a frame or window from one cell cannot be observed by a
different cell unless a separately authorized result explicitly exports it.

## Contract

Every edge and request carries these immutable identifiers:

```text
edge_id       = stable edge identifier
edge_generation = monotonically increasing generation for that edge
caller_cell   = cell identity and incarnation
callee_cell   = cell identity and incarnation
operation_id  = unique operation identifier
profile       = named operation profile, such as xcode.test or host.inspect
```

The authority for a request is a typed grant bound to `edge_id`, `edge_generation`, both cell
incarnations, `operation_id`, and the allowed profile. A request with a stale generation, wrong
incarnation, unknown profile, or operation outside the callee membrane is refused before the
provider starts. No request may name an arbitrary executable, path, socket, device, window,
display, keyboard, or mouse target as an unreviewed capability.

Delegation is one-hop in this spec: a callee cannot forward the caller's authority or create a
new edge on the caller's behalf. Shared service cells are reached through separate grants and
separate ledgers; one caller's edge does not disclose another caller's state, credentials,
results, or interactive surface.

Detaching an edge atomically changes it to `detached` for admission checks, records the revocation
time and reason, and invalidates its generation for future requests. In-flight work retains only
the operation-specific authority already admitted. A detach response includes the operation
state (`not_started`, `running`, `completed`, `cancelled`, or `failed`) and a receipt. Reconnect
must mint a new generation and cannot reuse an old receipt or operation id.

Cell identity includes a kind, genome/profile selection, membrane policy, and incarnation. The
kind and membrane are authoritative policy inputs, not labels inferred from the caller. Profiles
describe an allowed operation bundle; they are not genomes. A native profile may permit host
tools or hardware acceleration while still denying unrelated host state and other cells' input.

An interactive operation names a target cell/session and receives an input channel and frame
channel bound to that session. The provider must prove, with concurrent-cell traces, that input
events and frames are not cross-delivered or cross-observable. A shared host compositor or global
input facility is not by itself evidence of isolation.

Each request, admission decision, detach, provider start/stop, process/resource outcome, and
returned result is ledgered with timestamps, identifiers, policy decision, and redacted command
plan. Receipts are immutable and sufficient to distinguish refused, admitted, completed,
cancelled, failed, and unknown-after-disconnect outcomes.

## Acceptance

- A Linux-cell client can submit one typed, profile-bound request to a native-cell provider, and a
  stale edge generation, wrong incarnation, disallowed profile, or malformed operation is refused
  before provider execution.
- Detaching an edge refuses every subsequent request, allows an already admitted request to reach
  a terminal recorded outcome, and requires a new generation for reconnect; tests cover detach
  during queued and running work.
- The provider cannot delegate onward using caller authority, and two callers of a shared service
  cannot read one another's files, credentials, receipts, results, or operation channels.
- Cell records expose distinct identity, kind, genome/profile, membrane, and incarnation values;
  profiles such as `xcode.test`, `docker.build`, and `host.inspect` are operation policy bundles,
  not genomes.
- Two concurrent interactive cells have separate input and frame channels; adversarial tests
  show that keyboard, mouse, focus, window, and frame events addressed to cell A never arrive in
  cell B and vice versa.
- Lifecycle tests prove provider processes and resources are stopped or reaped on completion,
  cancellation, cell crash, and edge detach, with no ambient mount, inherited descriptor, socket,
  credential, or global-input path added to the caller.
- Native-provider, runtime, and interactive-isolation evidence identifies the actual provider and
  host, includes reproducible commands and timestamps, and does not claim stronger isolation than
  the provider proves; unsupported GUI/runtime cases remain explicitly open.
