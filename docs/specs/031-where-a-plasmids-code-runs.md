---
id: 031
title: Where a plasmid's code runs, and how its tools are called
status: draft
intents: [010, 011, 012, 006]
---

## Behavior

A plasmid's declaration may name a component: `[impl].wasm`, one WebAssembly component file. This
spec says what runs that component, where it runs, and how a program inside a cell calls the tools
the declaration names. In short: the cell's `membraned` runs one **component host** for each
attached component plasmid. It is a host process that `membraned` owns as a Broker holding. The
component runs inside the WebAssembly sandbox, and the process around it is confined by a pinned
deny-default policy. Inside the cell, each such plasmid has one Unix socket,
`/run/tools/<plasmid>.sock`, that speaks MCP's tool methods. A call crosses the guest bridge to the
component host. The component's own network requests leave through the cell's egress relay, and
only to hosts the plasmid itself declared.

This is the sibling spec 011 leaves open: "Freezing the world is its sibling spec, and it is the
larger of the two." (`011:537-538`). Spec 001 defers the same interface: "The plasmid WIT world
(SDK surface) — deferred by design" (`001:1609`). Spec 011 promises that "A plasmid is reported
active only when every tool its declaration names resolves." (`011:392`), and spec 020's use
witness is to "attach it to a real cell and call its declared tool" (`020:508`). Neither can be
met while nothing runs a component. It serves four intents. Intent 010: anyone may write a
plasmid, so its code is untrusted and runs confined. Intent 011: an unchanged harness calls tools
through MCP and sees no kernel names. Intent 012: the component reaches only what its own
declaration names, and only while attached. Intent 006: two confinements, bounded parsers and
dependency records.

What is fixed and what is not. The **harm guards** in §4, what a plasmid can reach and what it can
never reach, are fixed now, and each has an acceptance item. The **world** is provisional, at
version 0.1.0, and is not frozen: `plasmid new` keeps refusing a component, and spec 007's first
blocker, "The `plasmid-sdk` interface freeze." (`007:90`), stays open. Every shape that only a
real plasmid can teach waits for the first consumer, `github-pr` with one tool, `pr.read` (§11).
This spec edits no other spec; it proposes changes below. Its status is `draft` because owner
question Q1 decides where foreign code runs.

**Levels.** Each acceptance item names the lowest level that proves it: the **host** (the
component host alone, on Darwin, with fake peers), the **model** (controller, journal, fake
backend), or the **pinned runtime** (a booted repo-built cell). As in spec 028, "That waits on
owner decisions O-1" (`028:34`), O-6 and O-7. Pinned-runtime items are reported not run until
then, never passed.

**Platform.** macOS first: "Darwin arm64 is the only runtime host" (`024:42`). The component host
is confined by a Seatbelt policy. A Linux host is deferred; its mechanism would be Landlock and
seccomp, and this spec makes no claim about it.

## Contract

### 1. Where the code runs

The component runs **on the host**, in one component host process per attachment. The cell's
`membraned` launches it as its own child, as spec 001 says of brokers: "the membrane owns the VMM
child, shim, and brokers as **its** children" (`001:800-801`). It is a spec 017 Broker holding, so
spec 017's Broker rules apply: "use the original owned child and answered private control
readiness" (`017:402`), and "The first equal holding may launch; the last shuts down/reaps the
original owned child" (`017:405-406`).

Why the host and not the guest:

- PID1 stays outside the workload's control group "and runs no helper process" (`024:249`), and
  "There is no PID namespace, on purpose" (`024:222-223`). A runtime in the guest would be a process
  the workload can see, which spec 027 counts as "A hint, a name or value the kernel wrote"
  (`027:156`).
- Spec 017 already gives the Broker class what a component needs: a launch only after a durable
  prepare, answered readiness, a per-grant relay that can be withdrawn alone, and reaping by the
  last holding. Spec 024 already says what the workload sees when a Broker stream is refused.
- The credential grammar pairs a component with a handle: "`consumer` pairs with it
  (`handle⇔wasm`" (`001:616-617`). The credential proxy is on the host: "The credential vsock
  proxy remains RESERVED: port4041 terminates at the membrane" (`001:815-816`). So a future
  credential handle never has to enter the guest or the component.
- The guest's trusted code does not grow.

The cost is that code anyone can write runs on the host, where spec 026 says of a broker "A broker
is such a process: it runs unconfined" (`026:102-103`). So §4's G3 confines the process, and this
spec proposes the matching spec 026 change. This is owner question Q1, which states the guest
alternative and its costs.

### 2. What a component is, and what runs it

- **The file.** A component is one WebAssembly component (the component model) in the file
  `[impl].wasm` names, which spec 011 calls "the component the declaration names" (`011:46-47`).
  In a package, "If `[impl].wasm` is present, it names a listed package-relative file."
  (`020:111-112`). A local component path is relative to its declaration's directory, with no
  `..` component and no leading `/`. A component file is at most 64 MiB, as spec 020 allows "each
  file 64 MiB" (`020:155`).
- **How it is built.** It targets the world `plasmosome:plasmid/plasmid@0.1.0` (§3). A Rust author
  builds for `wasm32-wasip2` with `wit-bindgen`. Any language with component tooling works, which
  keeps intent 010's promise that "Nothing on the generated path is a shortcut I am locked out of"
  (`011:35-36`).
- **The deployment input.** The controller's strict JSON configuration gains an optional key,
  `component_runtime: {host: Artifact, policy: Artifact, roots: Artifact | null}`. `host` is the
  component host executable, `policy` its pinned Seatbelt policy, and `roots` optional extra
  trusted certificate roots in PEM form, for tests (§10). Each is an Artifact as spec 001 §4.2
  defines it, and spec 023's rule applies: "Spec 001 §4.2's immutability rule for the helper applies
  to `membraned`" (`023:78`). Without `component_runtime`, an attach of a plasmid with
  `[impl].wasm` is refused before prepare with 103, `capability` `unsupported:component`. The
  names are provisional.
- **The engine.** The component host runs the component with wasmtime, built with only the
  features this spec needs. The default compiler is Cranelift, run inside the confined process.
  The reason, read from wasmtime's `docs/stability-tiers.md` at commit a0613097 (2026-10-09) and
  not measured: Cranelift is a tier 1 compiler, the Pulley interpreter is tier 2, and only tier 1
  promises that "CVEs and security releases will be performed as necessary". The same document
  puts `aarch64-apple-darwin` in tier 2, so no tier 1 configuration exists on the only runtime
  host. That is owner question Q2.

### 3. The world, provisional version 0.1.0

The world replaces today's empty placeholder in `crates/plasmid-sdk/wit/plasmid.wit`. Its export
is this text; its imports follow the rule below, and W1 fixes their exact list.

```wit
package plasmosome:plasmid@0.1.0;

interface tools {
  variant failure {
    invalid-input(string),
    failed(string),
  }
  names: func() -> list<string>;
  call: func(name: string, input: string) -> result<string, failure>;
}

world plasmid {
  export tools;
}
```

- `names` is called once, at launch. Its answer is checked against the declaration (§8).
- `call` gets the call's arguments as JSON text and returns the text the caller reads.
  `invalid-input` means the caller should change its arguments; `failed` means the work failed.
- **Imports.** The world imports only WASI interfaces that carry no authority, and one network
  interface served only through the plasmid's own grants (§4, G1). The default WASI release is
  0.2, since `wasm32-wasip2` targets it. WASI 0.3.0 was published on 2026-06-11, read from the
  WASI repository's releases; the first consumer may choose it instead (W1). The expected set is
  `wasi:io`, `wasi:clocks`, `wasi:random`, `wasi:cli`'s environment, exit and standard streams,
  `wasi:filesystem`'s preopens with an empty list, and `wasi:http`'s outgoing handler (W2). It is
  not measured: W1 records what the first consumer's build actually imports.
- **Changes.** While the world is `0.x`, it changes in place by an edit to this spec, with a minor
  version bump each time. The component host accepts exactly the versions this spec lists, today
  only 0.1.0. Spec 011's scaffold rule stands: it "says that it will not until the plasmid
  interface is frozen, and exits `2`" (`011:422-423`). Only the owner records a freeze (Q7).

### 4. The harm guards this spec fixes now

These hold whatever the world grows into. Each is a rule with an acceptance item.

- **G1. No authority by importing.** The component host satisfies exactly the imports the world
  names. Each one either carries no authority (clocks, randomness, an empty environment and
  argument list, no preopened directory, an empty stdin, output that nobody reads) or is backed by
  one of this plasmid's own grants and enforced outside the component host. A component that
  imports anything else fails its launch, and the attach with it. `plasmid-sdk`'s working notes
  say the same: nothing should let a plasmid acquire authority by importing something.
- **G2. Never reachable.** A component can never reach: the host's files, processes and signals;
  the host network, except through the cell's egress relay under this plasmid's own grants; the
  controller's or `membraned`'s sockets; another plasmid's grants, component or memory; another
  cell; a credential's value; the workload's processes and memory; the guest's control and data
  channels; the host's environment and arguments. A future workspace import goes through a Mount
  grant of the plasmid's own declaration.
- **G3. Two confinements.** The component runs inside the WebAssembly sandbox, and the component
  host process is confined too. It applies the pinned policy before it reads one byte of the
  launch record or the component. Every channel it uses is a descriptor `membraned` made and
  passed to it at exec; it opens no path and creates, binds or connects no socket. `membraned`
  verifies the policy before readiness and at every observation, as spec 001 asks of the VMM
  helper: "Before readiness and during every recovery observation, verify that retained binding"
  (`001:1146-1147`). The check is the membrane's existing confinement observation, with its known
  limit: it shows that the required operations answer denied, not which profile is installed
  (owner gate O-3 on task plasmosome-qep). `membraned` forks and at once execs the component host;
  no confinement happens between fork and exec.
- **G4. Enforcement outside the component host.** The egress relay checks every connection
  against this plasmid's own ProxyMap holdings, so a component host that skipped its own check
  still reaches nothing undeclared. The component host does no DNS lookup; the relay resolves only
  the recorded destination, as spec 017 says: "A DNS name is resolved only for this authorized
  destination." (`017:113-114`). A component cannot add a certificate root or turn off
  certificate checks.
- **G5. What a caller reads comes from the declaration.** Tool names and sentences in `tools/list`
  are the declaration's. The component cannot change them, so reviewing the declaration reviews
  what a model is shown.
- **G6. No say in teardown.** Spec 011 holds: "A plasmid implements no revocation hook, gets no
  chance to refuse a detach" (`011:84-85`). The world exports nothing the kernel calls at attach,
  detach, reload or expiry, except `names` at launch. Detach and expiry never wait on the
  component. The component host holds the read end of a pipe from `membraned` and exits within
  1 s once it reads end of file, so a SIGKILLed `membraned` leaves no component running. No
  admitted call runs more than 1 s past B, and the process is gone by H (§8).
- **G7. Bounds.** Every call has a deadline, a memory cap and an output cap. Past one, the call
  fails with a fixed tool error, its instance is dropped, and the component host keeps serving.
  Every parser of bytes a component, an author or a workload writes states its bound and refuses
  past it, as spec 026 requires: "A parser whose input a less-trusted party can write states a
  bound." (`026:393`).
- **G8. Nothing shared.** One component host per attachment. It is never shared between
  attachments, plasmids or cells, which narrows spec 017's rule that "equal launches may share one
  original owned broker" (`017:507`). Each call runs in a fresh instance.
- **G9. The bytes that run are the bytes attach pinned.** The component's SHA-256 is recorded at
  prepare. The component host reads the component through its size bound from a descriptor
  `membraned` opened without following a link, and refuses bytes whose digest differs.
- **G10. Invisible.** Nothing in the guest names the component host: no process, mount,
  environment variable or file, except the plasmid's socket. Its path and every reply name only
  the plasmid and its version, never a kernel name.
- **G11. Denials are missing declarations.** A request of the component refused at the boundary is
  reported as spec 011 says: "The report names what was denied and the declaration field it belongs
  in" (`011:385-386`), so it names the host and `network.hosts`, or the port and `network.ports`,
  and nothing that was granted.

### 5. How a tool is found

- **One socket per component plasmid.** Each attached component plasmid has one Unix stream socket
  in the workload's mount namespace, at `/run/tools/<plasmid>.sock`. The path is provisional (Q3).
  It is the guest side of the plasmid's Broker holding, at the target whose key is the Broker's
  name: "key is respectively the recorded guest_path, guest_path, host, host, name or target"
  (`001:1449-1450`). The Broker's name is the plasmid ID.
- **The path is recorded, not derived.** Today spec 001 says "Broker data is exposed at the
  reserved guest Unix path" (`001:1265-1266`), a path under `/.plasmosome/` built from the name. A
  name with `plasmosome` in it is a hint under spec 027. So the Broker capability records its
  guest path, as UdsSocket's recipe does, and nothing derives it. That is spec 017's rule for
  launch paths too: "Paths and arguments are exact payload, not a recipe to be guessed from a
  broker name" (`017:494-495`).
- **The directory exists before any attach.** PID1 makes `/run/tools` before hello, as it makes
  spec 027's directories: "Before hello, PID1 makes the guest's software directories"
  (`027:58`). It is owned by root, mode 0755, empty at readiness, with a mount source, type and
  options that name nothing about Plasmosome. So a plasmid attached later has somewhere to appear,
  as spec 011 promises: "a cell is created already arranged so that what is attached later can be
  reached from inside it" (`011:181-182`). PID1 refuses readiness if the root image has an entry
  there. Spec 027 counts file names under `/run` as hints too, so its reference lanes make the same
  directory and serve the same socket (the proposed spec 027 change).
- **The listener stays while any holding is at it.** As "The shared resolver/TUN/route persist
  while peer bindings exist" (`001:1406`), the socket at a target stays while any Broker holding
  is at it, so a reload's two generations hand over without a moment with no socket.
- **An ID that cannot be a path.** An ID that is not one path component, or that makes a path
  longer than Linux's 108-byte `sun_path`, is refused, as spec 001 says: "Unrepresentable
  names/paths and cross-class projection collisions refuse in preflight" (`001:1267-1268`). It is
  refused before prepare with 108 at field `id`, whose `fix` is an `id` line with a name that
  fits.
- **What resolves means.** A tool **resolves** when a connection made from inside the workload
  namespace reaches the plasmid's component host, and that host lists the tool. Tool names need be
  unique only within one plasmid, so two plasmids never collide.

### 6. How a tool is called

The socket speaks MCP's tool methods as newline-delimited JSON-RPC 2.0, one message per line. MCP's
stdio transport uses the same framing, and its text says the framing "works unchanged over Unix
domain sockets", read from the revision below and not measured here. So a stdio-to-socket bridge
such as `socat` makes the socket an ordinary stdio MCP server with no Plasmosome client.

- **The revision.** The component host serves exactly MCP revision `2026-07-28`, the current one
  when this spec was written. That revision has no `initialize` handshake: every request carries
  its version in `_meta`. A client that speaks only `2025-11-25` or earlier fails against it.
  Whether a harness speaks `2026-07-28` is not measured (Q3).
- **Methods.** It serves `server/discover`, `tools/list` and `tools/call`, and the notification
  `notifications/cancelled`. A request whose `_meta` lacks the revision's required fields is
  `-32602`. A request that names another revision gets `-32022`, the revision's
  unsupported-version error, listing `["2026-07-28"]`. An `initialize` request is `-32601`, and
  its message names `2026-07-28`.
- **What a caller sees.** `tools/list` comes from the declaration (G5): each tool's name, its
  sentence as `description`, and `inputSchema` exactly `{"type": "object"}` (W3), in byte order of
  the names, with no pagination. Every result carries `resultType` `"complete"`. The server's
  identity in each result's `_meta` is the plasmid's ID and version, nothing else (G10). The
  values of the caching fields the revision requires are the first consumer's to choose (W10).
- **A call.** Each `tools/call` runs in a fresh component instance. Its `arguments` object, or
  `{}` when absent, is passed to `call` as JSON text. `ok(text)` is a result with one text item and
  `isError: false`. `invalid-input(message)` and `failed(message)` are a result with one text
  item, the message, and `isError: true`.
- **Cancellation.** On `notifications/cancelled` for a call in flight, the component host drops
  its instance and sends nothing more for it. A call whose connection closes is dropped the same
  way, within 1 s.

### 7. Whose reach a component's needs are

- **Its component's only.** A declaration with `[impl].wasm` gives its `[network]` needs to its
  component alone. They become ProxyMap holdings whose recipe records that the component reaches
  them. The cell's egress relay serves them only to this plasmid's component host, selected by
  owner `{cell, plugin}` and target, over a private egress channel. They are never installed in
  the guest's resolver or network device, so the workload cannot reach those hosts.
- **Why.** The workload that calls a tool stands where a requirer stands, and "A requirer gains its
  provider's tools, never its provider's grants." (`011:186-187`). Intent 012 says nothing arrives
  that was not needed. The recipe records the reaching side because recovery and observation read
  only recordings, never a declaration. A plasmid that wants the workload to reach a host declares
  no component, as today. This is owner question Q4.
- **Other sections.** With a component, `[workspace]` is refused before prepare with 103,
  `capability` `unsupported:component:workspace`, until a consumer needs a filesystem import.
  `[commands]`, `[secrets]` and `[model]` stay refused as spec 022 says: "`[commands]` with any
  command: 103" (`022:329`) and "`[secrets]` with any reference, or `[model]`" (`022:331`).
- **Declared tools need a component.** A declaration that names a tool and no `[impl].wasm` is
  refused before prepare with 108 at field `impl`, with the `fix` spec 001 already shows: "a
  section such as `[impl]` followed by `wasm = "component.wasm"`" (`001:115-116`). Nothing could
  serve such a tool, so it could never be reported active, and spec 020 forbids the way around it:
  "do not invent a callable implementation to avoid the SDK/runtime prerequisite"
  (`020:514-515`).

### 8. Launch, lifetime and failure

- **The launch record.** The Broker capability of a component carries its complete launch record:
  the component's absolute path, size and SHA-256; the declared tools, names and sentences, in
  byte order; the guest path; the call limits (W10); and the component host and policy Artifacts.
  It is recorded at prepare, so recovery and observation never read a declaration, and it fits
  the frame limit: "Complete serialized operations/inverses and enclosing messages must fit
  spec001's existing frame limit" (`017:104-105`). It is the contract spec 017 asks for:
  "Additional launch inputs require an accepted contract" (`017:483-484`). The field names are
  provisional.
- **How it reaches the process.** `membraned` launches the component host the way spec 001 launches
  the VMM helper: "The membrane uses direct exec, not a shell" (`001:1119`), with an empty
  environment. It passes, each on its own descriptor, the policy bytes, the launch record, the
  component bytes from a file opened without following a link, the control and data endpoints, the
  egress channel and the lifeline pipe (G6). Whether the pinned policy lets the process accept
  connections on a passed listener is not measured. If it does not, `membraned` passes each
  accepted connection as a descriptor instead; task F8c measures it and chooses.
- **Launch checks.** In order: apply the policy; read and check the launch record; read the
  component through its bound and check its digest (G9); check that it is a component of a listed
  world version; instantiate it with only the world's imports (G1); call `names`, and check that
  its answer equals the declared tool set. Then answer readiness on the control endpoint, as a
  Broker does (spec 001 §4).
- **Lifetime.** A component plasmid holds a Broker grant, so spec 028 applies: "Every plasmid that
  holds a capability grant has a lifetime" (`028:10`). At B, the gate closes the holding's streams
  and new connections, as spec 024 says: "A new connection is closed the same way as soon as it is
  accepted." (`024:353`). Its egress holdings are fenced at the same B. Each call in flight sees its
  connection close and is dropped within 1 s (§6), so admitted work ends within 1 s of B. The
  removal that follows starts "within 2 s of B, or of its own next run" (`028:116-117`). The Broker
  holding is prepared after its plasmid's egress holdings, so the removal, which withdraws in
  reverse order, withdraws it first, with drain budget D. That withdrawal is the last one at its
  target, so it shuts the component host down and reaps it, and the process is gone by
  "H = B + 2 s + D" (`028:62`). This is the bound spec 028 asks for: "the spec that gives Broker a
  source must bound its accepted work by H" (`028:213-214`). While the controller is down, the
  fence still ends every call, but the idle process waits for the removal.
- **A component host that dies.** It is reaped and never restarted. Its plasmid's `state` becomes
  `failed`, its socket closes every new connection at once, and `plasmid.remove` or
  `plasmid.reload` clears it. Until then the cell's `membrane.status` is not ready, since a
  membrane is ready only when "every broker answers ready" (`001:723`), and it names the holder:
  "`broker` — the one that held the set back" (`001:727`). A trap inside a call is not a death:
  the instance is dropped and the host keeps serving.

### 9. What a failure looks like on each wire

**The control socket** (`plasmid.add`, `plasmid.reload`). Before prepare:

- tools declared and no component: 108 at field `impl`, with the `fix` in §7;
- a component plasmid with `[workspace]`: 103 `unsupported:component:workspace`; no
  `component_runtime`: 103 `unsupported:component`;
- a component file that is missing, over 64 MiB, outside its package, or fails spec 022's trust
  checks: 108 with `path` the file, no field and no `fix`.

After prepare, a failed launch check aborts the transaction, and the reply is 108. Spec 001 says
"a mid-commit failure rolls the prefix back and replies `CommitFailed`-shaped code `103`/`109`"
(`001:612-613`); this is the case it gains.

- digest mismatch, a file that is not a component, a world version this spec does not list, or an
  import the world does not name: 108 with `path` the component and `detail` naming what failed,
  no field and no `fix`. The last three are a fourth refusal with no field, beside the three
  where spec 001 says "Exactly three refusals an author can cause name no declaration field"
  (`001:127`). A digest mismatch is not the author's doing, and already carries none, as "A 108
  the author did not cause carries no `fix` either" (`001:135`);
- a declared tool the component lacks: 108 at field `provides."<key>".tools."<name>"`, with `fix`
  exactly `remove this entry`;
- a tool the component exports and the declaration lacks: 108 at field `provides."<key>".tools`,
  with `fix` `"<name>" = "Describe what this tool does."`. The key is the declaration's only
  binding, or the first in byte order.

**The tool socket.** Protocol faults are JSON-RPC errors. A line that is not JSON is `-32700`, and
the conversation goes on. A line over 1 MiB is `-32600`, and the connection closes. An unknown
method is `-32601`. An unknown tool name, or `arguments` that is not an object, is `-32602` with
`data: {"tool": <name>}`. Everything about the tool itself is a result with `isError: true` and one
text item: the component's own message, or one of four fixed sentences in plain words:

| Cause | Text |
| --- | --- |
| a trap | `The tool stopped because of an internal error.` |
| the call deadline | `The tool did not finish in time.` |
| the memory cap | `The tool ran out of memory.` |
| the output cap | `The tool's result was too large.` |

A call to a detached or fenced plasmid gets no reply: the connection closes, as for any Broker
stream. A new connection to a removed plasmid fails with `ENOENT`, since its path is gone.

**The component's own requests.** Withheld must look like absent. Spec 027 names the tell:
"`ECONNREFUSED` for a withheld host where an unknown host fails name resolution" (`027:154-155`).
So an undeclared host fails as a name that does not resolve, the network import's DNS error. A
declared host on an undeclared port fails as connection refused, as spec 024 says of a workload's
flow: "A new TCP connect fails with `ECONNREFUSED` when no other eligible grant covers its host,
port and transport" (`024:354`). A flow cut by a fence or a removal fails as a reset.

### 10. Reported active, and the use witness

**Reported active.** A component plasmid is active only after all of these: the bytes match the
pinned digest; the component instantiates with only imports the world names;
`names` equals the declared tool set; and a probe from inside the workload namespace connected to
the socket, reached the component host, and read the declared tools from `tools/list`. Spec 027's
attach rule is the model: "Attach is not complete until the kernel has probed from inside the
workload namespace" (`027:84`). Any failure aborts the attach and leaves nothing, since "Attach is
all-or-nothing." (`011:390`). After attach, `state` is `active` only while `membraned` observes the
process alive, confined and answering readiness; otherwise it is `failed` (§8). The journal's
Broker recording carries the tool names, so `cell.status` can send spec 001 §3.6's `tools` field
from the journal, and `active` rests on observation, not on the journal alone. This meets
spec 011's acceptance: "A plasmid one of whose declared tools does not resolve is not reported
active." (`011:486`).

**The use witness.** A workload process started by `cell.exec` in a real cell connects to
`/run/tools/github-pr.sock`, calls `pr.read`, and gets the answer of a fixture server. The operator
file maps `api.github.com:443` to that server with `allow_private`, which spec 017 requires for a
private address: "loopback, link-local, private, multicast and unspecified addresses refuse unless
the operator explicitly set `allow_private`" (`017:115-116`). The component host trusts the
fixture's certificate through `component_runtime.roots`. The call crosses the guest bridge to the
host and leaves through the egress relay, so it uses "a real supported backend and workload
process, not `ToolRegistry` alone" (`020:515-516`). After `plasmid.remove`, the socket is gone, an
open connection reads end of file, and the component host is reaped. Follow-up task F13 runs it.
Spec 020's two plasmid rows also need the registry (task plasmosome-zjo).

### 11. What waits for the first consumer

**The first consumer** is `github-pr`, curated, with one tool, `pr.read`: "Read a pull request's
title, body and review state." (`011:333`). Every fixture already names it. Reading a public
repository's pull request needs no token, so it can attach before credential custody exists. It
declares `[network]` `api.github.com` on 443 and a `[mock]` for `simulate`. `pr.comment` and its
credential handle wait for task plasmosome-3g2c. Follow-up task F9 builds it, and its PR edits this
spec to settle W1 to W4 and W10. Until then this spec fixes nothing about these shapes:

- **W1.** The exact WASI imports that carry no authority, and their versions, measured from the
  first consumer's build with the pinned toolchain; WASI 0.2 or 0.3.
- **W2.** The network import. Default: `wasi:http`'s outgoing handler, so TLS and a future
  credential handle stay in the component host. A raw socket import would put a credential inside
  the component.
- **W3.** Input schemas. Default: `{"type": "object"}` plus the declaration's sentence. A schema
  in the declaration is a spec 011 grammar change. A schema from the component would be text a
  model reads that no declaration review covered (G5).
- **W4.** Typed exports, one function per tool, instead of `names` and `call` on JSON text.
- **W5.** Streaming results, progress, and the detail of cancellation.
- **W6.** State across calls. Default: none.
- **W7.** Credential handles: task plasmosome-3g2c's spec, with `pr.comment` as the consumer.
- **W8.** Workspace access, the model endpoint, and calling a provider's tools from a component.
- **W9.** Component logs. Default: standard output and standard error are discarded.
- **W10.** The limit values and the caching fields of `tools/list`. Default limits: a 30 s call
  deadline, though a call still ends at B (§8); 64 MiB of memory per instance; 1 MiB per line in
  either direction, which is also the output cap; 4 calls at once per plasmid, where a call past
  that waits within its own deadline. None is measured.
- **W11.** One socket for the whole cell, for a harness that wants a single server. That is a
  program in the guest image, or later a plasmid's software tree (spec 027), not kernel code.

## Changes proposed to accepted specs

These are proposed text. The PR that adds this spec edits no other document. Each change is
applied by a later reviewed change to the document it amends, before any task that relies on it:
F1 to F5 in this PR's follow-up list. Each item quotes the text it changes with its lines at the
merge base.

**Spec 001** (F1).

- §5: replace "The plasmid WIT world (SDK surface) — deferred by design; `plasmid-sdk` is a
  reserved crate with a placeholder world." (`001:1609-1610`) with "The plasmid WIT world is
  provisional, at version 0.1.0, as spec 031 defines it. It is not frozen."
- §1: replace "Exactly three refusals an author can cause name no declaration field" (`001:127`)
  with "Exactly four refusals an author can cause name no declaration field", and after the item "a
  declaration nested more than 32 levels deep, counted as spec 011 says." (`001:131`) and its
  amendment note, add "4. a component that spec 031's launch checks refuse because it is not a
  component, targets a world version spec 031 does not list, or imports what that world does not
  name."
- §3.9: after "Absent declarations mean `passthrough`." (`001:576`), add "`state` is `failed`
  while a spec 031 component host of the plasmid has exited and the plasmid is not yet removed or
  reloaded. A `failed` plasmid is never `active`."
- §3.10: after "a mid-commit failure rolls the prefix back and replies `CommitFailed`-shaped code
  `103`/`109` with the rolled-back list." (`001:612-613`), add "A spec 031 launch check that fails
  after prepare aborts the transaction the same way, and the reply is 108 as spec 031 §9 says. An
  attach of a component plasmid is complete only after spec 031 §10's probe from inside the
  workload namespace."
- §4.1: after "`SpawnBroker` carries `{id,name,launch,owner}`" (`001:944`) and the rest of that
  sentence, add "A spec 031 component's Broker capability also carries its `guest_path` and its
  component launch record (spec 017)."
- §4.2: replace "Broker data is exposed at the reserved guest Unix path
  `/.plasmosome/brokers/<hex-encoded UTF-8 name>.sock`;" (`001:1265-1266`) with "Broker data is
  exposed at the capability's recorded `guest_path`, under `/run/tools`, which PID1 makes before
  hello (spec 024 §5). One listener stands at each Broker target while any holding is at it, so a
  reload hands over with no gap;".
- §4.2: after "Workload edits of resolver configuration grant no bypass." (`001:1408-1409`), add
  "A ProxyMap holding whose recipe says a spec 031 component reaches it is never installed in the
  guest. It adds no DNS answer, route or rule here, and the cell's egress relay serves it only to
  that component's host (spec 017)."

**Spec 024** (F1).

- §5: after the bullet that begins "The network namespace holds only loopback until a ProxyMap
  is installed." (`024:237`), add "- Before hello, PID1 makes `/run/tools`, owned by root, mode
  0755, empty at readiness, with a mount source, type and options that name nothing about
  Plasmosome, as spec 027 §2 makes its software directories. A spec 031 tool socket appears there
  without a restart. PID1 refuses readiness if the root image has an entry there."
- §9: after "Every error in this table is expected, not yet measured" (`024:347`) and the rest of
  that sentence, add "A spec 031 tool socket is a Broker stream in this table."

**Spec 011** (F2).

- Out of scope: replace "Freezing the world is its sibling spec, and it is the larger of the
  two." (`011:537-538`) with "Spec 031 is that sibling. It defines a provisional world, version
  0.1.0, and does not freeze it."
- Contract: after "The three name no field and carry no `fix`." (`011:360-361`), add "Spec 031
  adds one more that names no field: a component its launch checks refuse because it is not a
  component, targets a world version spec 031 does not list, or imports what that world does not
  name. It names the component's path and carries no `fix`." In the Behavior, replace "The three
  refusals of the file as a whole" (`011:116-117`) with "The three refusals of the file as a
  whole, and spec 031's refusal of a component's world,".
- Contract: after "A plasmid is reported active only when every tool its declaration names
  resolves." (`011:392`), add "- **A declared tool needs a component.** A declaration that names a
  tool and no `[impl].wasm` is refused at attach with 108 at field `impl`, and `fix` `[impl]`
  followed by `wasm = "component.wasm"`, since nothing could serve the tool (spec 031)."

**Spec 020** (F2).

- Behavior: replace "It does not freeze the component WIT world" (`020:40`) with "Spec 031
  defines the component WIT world as provisional; this spec does not freeze it".
- Delivery boundaries: replace "The existing spec011 gate question and component interface remain
  outside this spec" (`020:597-598`) with "The existing spec011 gate question remains outside this
  spec, and the component interface is spec 031's".

**Spec 017** (F3).

- The capability table: replace the row "| `Broker` | `name`, `launch: BrokerLaunch` |"
  (`017:66`) and the rest of that row with "| `Broker` | `name`, `launch: BrokerLaunch`,
  `guest_path`, `component` | `broker/{name}` |".
- After "`BrokerLaunch` is the strict serde record" (`017:472`) and the paragraph it begins, add
  "`guest_path` is the absolute guest path of the holding's data socket, recorded and never
  derived. `component` is null, or a spec 031 component launch record: the component's path, size
  and SHA-256, its declared tools, its limits, and the component host and policy Artifacts."
- Resolved recipes: replace "Broker, UdsSocket and SessionFile have no declaration source yet."
  (`017:125-126`) with "Broker's declaration source is a declaration's `[impl].wasm` (spec 031);
  UdsSocket and SessionFile have none yet."
- ProxyMap: after "equal-host selector changes, as specified by spec001 §4.2." (`017:401`), add
  "`ProxyRecipe` gains `reached_by`, exactly `workload | component`. A `component` holding is never
  installed in the guest. The relay serves it only to its own plasmid's component host over a
  private egress channel, selecting by owner `{cell, plugin}` and target (spec 031)."
- Broker launch: after "equal launches may share one original owned broker" (`017:507`) and the
  rest of that paragraph, add "A launch with a spec 031 `component` record is never shared: each
  holding has its own component host."

**Spec 022** (F3).

- Behavior: replace "Broker, UdsSocket, SessionFile and credentials are owner questions."
  (`022:35`) with "Spec 031 gives Broker a source; UdsSocket, SessionFile and credentials are owner
  questions."
- §5: after "A declaration that fails §3, or whose `id` differs: 108 with `path` the declaration."
  (`022:168`), add "- A component file that `[impl].wasm` names is opened under §3 too, with a size
  bound of 64 MiB instead of rule 4's, relative to its declaration's directory. One that fails is
  108 with `path` the component."
- §6: replace "`[model].endpoint`, `wasm` and `[provides]` add no capability." (`022:337`) with
  "`[model].endpoint` and `[provides]` add no capability. `wasm` yields one Broker capability for
  the plasmid's component (spec 031), and that plasmid's `[network]` needs yield ProxyMap
  capabilities reached by the component. With `wasm`, `[workspace]` is 103, `capability`
  `unsupported:component:workspace`."
- Open question 1: after "No declaration section selects them." (`022:506-507`), add "For Broker,
  spec 031 answers: `[impl].wasm` is its source, and no author-written Broker section exists."

**Spec 023** (F3).

- §1: after "Invalid configuration refuses startup, as spec 001 §4.1 says." (`023:74`) and the
  rest of that paragraph, add "The configuration also takes an optional `component_runtime`, spec
  031's deployment input. `membraned` never reads it: the controller puts its Artifacts in each
  component's launch record."
- §1: replace "Spec 001 §4.2's immutability rule for the helper applies to `membraned` and to every
  Artifact in the template." (`023:78-79`) with "Spec 001 §4.2's immutability rule for the helper
  applies to `membraned`, to every Artifact in the template and to every Artifact in
  `component_runtime`."

**Spec 026** (F4).

- Threat model: after "A plasmid's manifest and its broker's readiness replies are written by its
  author" (`026:74-75`) and the rest of that item, add "So is a spec 031 component, which runs on
  the host inside a confined component host. The standard defends against its code as well as its
  bytes."
- Not covered yet: replace "A broker is such a process: it runs unconfined" (`026:102-103`) and
  the rest of that sentence and the next with "A broker is such a process, behind spec 001's and
  spec 017's peer-UID checks. A spec 031 component host is confined by its own pinned policy and
  the WebAssembly sandbox, so for it the standard defends against code as well as bytes. Any other
  broker runs unconfined, and the standard defends against its bytes, not its code." The parser
  rows land with each parser, as "A new parser lands in the same PR as its row below and its
  bound." (`026:402`) requires: in F7 and F8.

**Spec 027** (F5).

- §6: after "When the cell lane attaches or detaches during a run, both reference lanes install or
  uninstall the same files by hand at the same moment" (`027:194-195`) and the rest of that
  bullet, add "- A spec 031 component plasmid attached in the cell lane is matched in both
  reference lanes by an ordinary tool server at the same socket path, serving the same tools and
  reaching a fixture for each host the component declares. Those hosts are outside the workload's
  G. All three lanes make `/run/tools` the same way."

**Spec 028** (F5).

- §5: replace "Spec 022's version 1 declares no Broker; the spec that gives Broker a source must
  bound its accepted work by H." (`028:213-214`) with "Spec 031 gives Broker a source and bounds a
  component host's accepted work by H: at B its streams close, its calls end within 1 s, and its
  removal reaps it by H. Any other Broker still has no source."
- §7: replace "Admitted work stops by B + D; a Broker's does not" (`028:245`) with "Admitted work
  stops by B + D, and a spec 031 component host's within 1 s of B; any other Broker's does not".

## Open questions

Each has the default this spec uses until the owner answers.

1. **Q1. Where plasmid code runs.** Default: (a), on the host, as §1 says. The alternative, (b),
   runs it in the guest beside the workload. (b) keeps foreign code behind the hardware boundary. It
   needs a guest process the workload can see, or a PID namespace; a helper process in PID1, which
   spec 024 forbids today; one network namespace per plasmid, so that the plasmid's flows can be
   told apart from the workload's; and a credential handle honoured inside the guest.
2. **Q2. The engine, and what it brings.** Default: wasmtime with the fewest features, compiling
   with Cranelift inside the confined process (§2). Neither Cranelift nor Pulley gives a tier 1
   configuration on `aarch64-apple-darwin`, which wasmtime puts in tier 2. Cranelift also writes
   native code from plasmid bytes on the host; whether the pinned policy can allow that memory
   while it denies everything else is not measured. The dependency closure is large (wasmtime, its
   WASI and HTTP crates, rustls, hyper), and each direct dependency needs a spec 026 record and
   review. Is that set, on a tier 2 target, acceptable?
3. **Q3. The in-cell wire and its path.** Default: MCP revision `2026-07-28`'s tool methods over
   newline-delimited JSON-RPC, at `/run/tools/<plasmid>.sock`, one socket per component plasmid
   (§5, §6). Alternatives: also serve `2025-11-25`, so that a client of the older, handshake
   revision works, at the cost of a second protocol to keep; a Plasmosome JSON-RPC with an MCP
   bridge later; or one socket for the whole cell, which needs routing by tool name on the host.
4. **Q4. Whose reach a component's needs are.** Default: its component's only (§7). Alternative:
   the workload's too, which gives a caller grants it never declared.
5. **Q5. Spec 022's question 1, for Broker.** Default: `[impl].wasm` is Broker's only declaration
   source, and no author-written Broker section exists. UdsSocket and SessionFile stay without a
   source.
6. **Q6. The gate, now that a plasmid's code runs on the host.** Default: nothing new gates
   attach. A component plasmid enters a cell the two ways any plasmid does, an operator-file
   listing or a spec 020 import. Its code reaches only what its closure's declarations and the
   operator file allow, as spec 011 says: "Declarations are enough to bound reach" (`011:289`).
   Should a user-population component be refused until the gate of intent 010 exists?
7. **Q7. When the world is frozen.** Default: not by this spec. It stays `0.x` and changes by edits
   to this spec. `plasmid new` keeps refusing a component, and spec 007's first blocker stays open
   until the owner records a freeze.

## Acceptance

Each item names its level, the guards it proves, and the broken runtime it catches: the mutants its
test must kill. Host items use components written as WAT text, so they need no wasm target.

1. **Tools come from the declaration (host; G5).** A component exporting exactly the declared
   tools lists each with exactly its declaration sentence and `{"type": "object"}`, in byte order.
   Catches: sentences taken from the component; a list built before the `names` check.
2. **A missing tool fails the launch (host, then model).** A component lacking a declared tool
   fails launch, and the attach replies 108 at that tool with `fix` `remove this entry`, leaving
   nothing. Catches: a check of exported against declared in one direction only; `active` without
   the check.
3. **An extra tool fails the launch (host, then model).** A tool the component exports and the
   declaration lacks fails launch, with the field and the line to add. Catches: the other
   direction checked alone.
4. **An undeclared import fails the launch (host; G1).** A component importing
   `wasi:sockets/tcp` fails launch with 108 naming the component and the import, no field and no
   `fix`. A component importing only the world's interfaces launches. Catches: a host that stubs an
   unknown import, or traps only when it is called.
5. **Refusals before prepare (model).** A declaration with a tool and no `[impl].wasm` is refused
   with 108 at `impl` and the `[impl]` fix. Without `component_runtime`, a component plasmid is
   103 `unsupported:component`; with `[workspace]`, 103 `unsupported:component:workspace`; with an
   `id` holding a `/`, 108 at `id`. Nothing is journaled in any case. Catches: an attach that
   reports active a plasmid nothing serves; a refusal found only after prepare.
6. **The pinned bytes are the bytes that run (host; G9).** Bytes changed after the digest was
   recorded fail launch, and so does a component path that is a symbolic link. Catches: hashing a
   path and then opening it again.
7. **The process is confined before it reads (host; G2, G3).** A test build of the component
   host that tries, after launch, to open a host file and a TCP socket fails both, and the
   membrane's observation reads confined. The test feeds the component through a pipe and writes
   its first byte only after the observation reads confined, so a host that reads before it
   confines is seen unconfined while it waits. Catches: confinement after parsing; a policy that
   allows network or file access.
8. **Egress reaches declared hosts only (host; G2, G4).** With a fake relay, a call reaching the
   declared host gets the fixture's answer. An undeclared host fails as an unresolved name and the
   fixture sees no connection; a declared host on an undeclared port fails as refused. Catches: a
   denial that shows as a policy error instead of an absent host.
9. **The relay enforces by owner (pinned runtime; G4).** A test program standing in for a component
   host asks the cell's real egress relay, over the component egress channel, for a host its
   plasmid did not declare, and for a host only another plasmid in the cell declared. Both are
   refused, and no flow opens. Catches: enforcement only inside the component host; selection by
   target alone instead of owner and target.
10. **Denials name what was missing (model; G11).** The denials of items 8 and 9 appear in the
    session log and in `denials` with `network.hosts` or `network.ports`, and no granted host
    appears anywhere.
    Catches: a report that lists what was granted.
11. **The deadline (host; G7).** An infinite loop returns `isError` with the deadline sentence
    within the deadline plus 1 s, and the next call succeeds. Catches: no interruption; a poisoned
    shared instance.
12. **The memory cap (host; G7).** Memory growth past the cap returns `isError` with the memory
    sentence, and the host keeps serving. Catches: no cap, or a host that dies with the instance.
13. **The output cap (host; G7).** A result over the output cap returns `isError` with the output
    sentence, never a truncated success. Catches: truncation.
14. **Protocol faults and tool faults stay apart (host; G7).** A non-JSON line is `-32700` and the
    next request is answered; a line over 1 MiB is `-32600` and the connection closes; an unknown
    method is `-32601`; an unknown tool is `-32602` with `data.tool`; `initialize` is `-32601`
    naming `2026-07-28`. Catches: protocol faults returned as tool results, or the reverse; an
    unbounded line reader.
15. **A fresh instance per call (host; G8).** A component that counts its calls returns 1 every
    time. Catches: instance reuse.
16. **Attach probes from inside (pinned runtime).** A process started before the attach sees the
    socket appear without a restart, and the attach returns only after the probe from inside the
    workload namespace. With the probe made to fail, the attach aborts and leaves nothing.
    Catches: a probe from the host side only; a socket that needs a restart.
17. **Removal leaves nothing running (pinned runtime; G6).** After `plasmid.remove`, a new
    connection fails with `ENOENT`, an open connection reads end of file, a call in flight ends by
    the removal's deadline, the component host is gone, and its egress flows are closed. The
    process's absence is checked with its start time, since `sandbox_check` answers denied for a
    dead pid. Catches: asking the component to stop; a socket removed but a process alive; a
    process killed but a socket left.
18. **Detach never waits on the component (pinned runtime; G6).** A component in an infinite loop,
    under a call deadline longer than D, still detaches within D. Catches: detach waiting on a
    call.
19. **The lifetime bound (model, then pinned runtime; G6).** At B new connections close at once,
    a call admitted before B ends within 1 s of B, the egress holdings are fenced, the Broker
    holding is the removal's first withdrawal, and the process is gone by H. With the controller
    stopped, the call still ends within 1 s of B. Catches: Broker work past H; a fence that only
    removal provides; a Broker withdrawn after its egress holdings, whose drains then come first.
20. **The lifeline (host; G6).** SIGKILL of the process holding the pipe makes the component host
    exit within 1 s. Catches: an orphaned component host.
21. **A dead component host is `failed` (pinned runtime).** A component host that aborts leaves
    its plasmid `failed` in `plasmid.list`; new connections close at once;
    `membrane.status` names its broker; nothing restarts it; and `plasmid.remove` completes.
    Catches: `active` after the tools stopped resolving; a silent restart.
22. **Two cells, two processes (pinned runtime; G8).** Two cells with the same plasmid have two
    component hosts, and detaching one leaves the other's calls working. A reload never makes one
    process serve two holdings. Catches: sharing across cells or holdings.
23. **Nothing names the membrane (pinned runtime; G10).** The guest shows no process, mount,
    variable or file of the component host. The socket's path and replies match the ordinary tool
    server of spec 027's reference lanes. Catches: kernel names leaking into the guest.
24. **A controller restart changes nothing (model).** After a controller restart the component
    host is still running and its calls work, and recovery opens no declaration. Catches: recovery
    that reads declarations again or launches a second process.

## Out of scope

- Freezing the world, publishing `plasmid-sdk`, and a versioning policy: spec 007 and the owner.
- Credential custody and a credential import: task plasmosome-3g2c's spec.
- The approval gate of intent 010, which spec 011 leaves to the owner: "Where the gate sits, who
  holds it" (`011:315-316`).
- A component skeleton from `plasmid new`, which stays refused while the world is not frozen.
- Workspace, model-endpoint and provider-tool imports, streaming, and state kept across calls.
- A cell-wide tool aggregator for an unchanged harness (W11).
- A Linux host, and the Linux mechanism for the component host's confinement.
