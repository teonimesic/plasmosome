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
component host. The component's own HTTP requests leave through the cell's egress relay, which
makes the TLS connection outside the component host, and only to hosts the plasmid itself
declared.

This is the sibling spec 011 leaves open: "Freezing the world is its sibling spec, and it is the
larger of the two." (`011:645-646`). Spec 001 defers the same interface: "The plasmid WIT world
(SDK surface) — deferred by design" (`001:1621`). Spec 011 promises that "A plasmid is reported
active only when every tool its declaration names resolves." (`011:453`), and spec 020's use
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
component host alone, on Darwin, with fake peers), the **membrane** (`membraned`'s launcher, gates
and egress relay, on Darwin, with a fake guest bridge), the **model** (controller, journal, fake
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
child, shim, and brokers as **its** children" (`001:812-813`). It is a spec 017 Broker holding, so
spec 017's Broker rules apply: "use the original owned child and answered private control
readiness" (`017:408`), and "The first equal holding may launch; the last shuts down/reaps the
original owned child" (`017:411-412`).

Why the host and not the guest:

- PID1 stays outside the workload's control group "and runs no helper process" (`024:263`), and
  "There is no PID namespace, on purpose" (`024:227-228`). A runtime in the guest would be a process
  the workload can see, which spec 027 counts as "A hint, a name or value the kernel wrote"
  (`027:159`).
- Spec 017 already gives the Broker class what a component needs: a launch only after a durable
  prepare, answered readiness, a per-grant relay that can be withdrawn alone, and reaping by the
  last holding. Spec 024 already says what the workload sees when a Broker stream is refused.
- The credential grammar pairs a component with a handle: "`consumer` pairs with it
  (`handle⇔wasm`" (`001:616-617`). The credential proxy is on the host: "The credential vsock
  proxy remains RESERVED: port4041 terminates at the membrane" (`001:827`). So a credential
  never has to enter the guest, or the component host: the egress relay attaches it (W7).
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
  trusted certificate roots in PEM form, for tests (§10). The roots are the egress relay's, never
  the component host's: the relay makes every TLS connection (G4), so the confined process holds
  no certificate store and has no path to one. Each is an Artifact as spec 001 §4.2 defines it, and
  spec 023's rule applies: "Spec 001 §4.2's immutability rule for the helper applies to
  `membraned`" (`023:78`). Without `component_runtime`, an attach of a plasmid with `[impl].wasm`
  is refused before prepare with 103, `capability` `unsupported:component`. The names are
  provisional.
- **The engine.** The component host runs the component with wasmtime, built with only the
  features this spec needs. The default compiler is Cranelift, run inside the confined process.
  The reason, read from wasmtime's `docs/stability-tiers.md` at commit a0613097 (2026-10-09) and
  not measured: Cranelift is a tier 1 compiler, the Pulley interpreter is tier 2, and only tier 1
  promises that "CVEs and security releases will be performed as necessary". The same document
  puts `aarch64-apple-darwin` in tier 2, so, by inference from it, no tier 1 configuration exists
  on the only runtime host. If the pinned policy cannot allow Cranelift's executable memory
  narrowly, the fallback is Pulley, which writes no native code. That is owner question Q2.

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
  WASI repository's releases; the first consumer may choose it instead (W1), which also changes
  the network import, since 0.3 replaces 0.2's outgoing handler. The expected set is `wasi:io`,
  `wasi:clocks`, `wasi:random`, `wasi:cli`'s environment, exit and standard streams,
  `wasi:filesystem`'s preopens with an empty list, and `wasi:http`'s outgoing handler (W2). It is
  not measured. Task F7 writes this set into the world text above, and W1 corrects it from what
  the first consumer's build actually imports. In G1, the imports the world names are the world as
  F7 writes it.
- **Changes.** While the world is `0.x`, it changes in place by an edit to this spec, with a minor
  version bump each time. The component host accepts exactly the versions this spec lists, today
  only 0.1.0. Spec 011's scaffold rule stands: it "says that it will not until the plasmid
  interface is frozen, and exits `2`" (`011:483-484`). Only the owner records a freeze (Q7).

### 4. The harm guards this spec fixes now

These hold whatever the world grows into. Each is a rule with an acceptance item. Each says what
holds outside the component host, and what holds only while the WebAssembly sandbox holds: a
component that escapes the sandbox runs as the component host process, under G3's policy alone.

- **G1. No authority by importing.** The component host satisfies exactly the imports the world
  names (§3). Each one either carries no authority or is backed by one of this plasmid's own grants
  and enforced outside the component host. The ones with no authority are empty, not inherited:
  clocks, randomness, an empty environment and argument list, no preopened directory, an empty
  stdin, and output that nobody reads. A component that imports anything else fails its launch,
  and the attach with it. `plasmid-sdk`'s working notes say the same: nothing should let a plasmid
  acquire authority by importing something.
- **G2. Never reachable.** A component can never reach: the host's files, processes and signals;
  the host network, except through the cell's egress relay under this plasmid's own grants; the
  controller's or `membraned`'s sockets; another plasmid's grants, component or memory; another
  cell; a credential's value; the workload's processes and memory; the guest's control and data
  channels; the host's environment and arguments. A future workspace import goes through a Mount
  grant of the plasmid's own declaration.
- **G3. Two confinements.** The component runs inside the WebAssembly sandbox, and the component
  host process is confined too. It applies the pinned policy before it reads one byte of the
  launch record or the component. The policy denies by default. It allows no file open, no
  outbound Unix-socket connect, no `process-exec` or `process-fork`, no signal to another process,
  no `mach-lookup`, no process information about another process, and `sysctl` reads only under
  listed names. Its allow rules are listed and reviewed in task F8c's record; whether the
  process needs any other operation is not measured, and task F8c measures whether Seatbelt
  filters `kern.procargs2` by name. Every channel the process uses is a
  descriptor `membraned` made and passed to it. It opens no path, and creates, binds or connects no
  socket. `membraned` starts it as spec 023 starts every process: "Every other descriptor is closed
  in the new process before exec, by `posix_spawn` with `POSIX_SPAWN_CLOEXEC_DEFAULT`"
  (`023:169-170`). So, as for the VMM helper, "No other descriptor survives." (`001:1135`), and
  descriptors 0 to 2 are `/dev/null`. Before readiness and at every observation, `membraned` checks
  the policy bytes against `component_runtime.policy`'s SHA-256 and checks confinement, as spec 001
  asks of the VMM helper: "Before readiness and during every recovery observation, verify that
  retained binding" (`001:1158-1159`). The check is the membrane's confinement observation. It
  shows that the required operations answer denied, not which profile is installed (owner gate O-3
  on task plasmosome-qep), and it cannot judge Unix connects: "The qualified host policy allows
  outbound connections to Unix sockets and answers `network-outbound` denied."
  (`plasmosome-membrane`'s `confinement.rs`, lines 4 to 7). Spec 023 records the same gap in the
  helper's own policy: "the qualified bundle's policy allows any Unix socket" (`023:460`). So for
  Unix connects, signals, exec, Mach lookups and reads of other processes the reviewed policy text
  is the proof, and an acceptance item probes each.
- **G4. Enforcement outside the component host.** Every check here is made by the cell's egress
  relay, outside the confined process, so a component host that skipped its own check still
  reaches nothing undeclared. The relay takes the owner `{cell, plugin}` from the egress channel a
  request arrives on, never from the request's bytes, as spec 017 says of the workload's relay:
  "The host derives the cell from that channel" (`017:397`). It checks every request against that
  owner's own ProxyMap holdings. It resolves only the recorded destination: "A DNS name is resolved
  only for this authorized destination." (`017:119-120`). It makes the TLS connection, verifies the
  certificate against a pinned root set and `component_runtime.roots`, and attaches any credential
  (W7). The component host sends and receives only HTTP requests and responses. It does no DNS
  lookup and holds no certificate store, root or credential, so a component cannot add a root or
  turn off a certificate check.
- **G5. What a caller reads comes from the declaration.** Tool names and sentences in `tools/list`
  are the declaration's. The component cannot change them, so reviewing the declaration reviews
  what a model is shown. The component host serves `tools/list` from its launch record, so this
  holds only while the WebAssembly sandbox holds: a component that escaped it could change what a
  caller reads.
- **G6. No say in teardown.** Spec 011 holds: "A plasmid implements no revocation hook, gets no
  chance to refuse a detach" (`011:84-85`). The world exports nothing the kernel calls at attach,
  detach, reload or expiry, except `names` at launch. These hold outside the component host. Detach
  and expiry never wait on the component: a drain cuts a call still running at its deadline D less a
  reserve, so a graceful removal never answers 106 because of a call. At B, `membraned` closes the
  holding's host-side connections, so no admitted call sends anything after B. Each withdrawal
  reaps its own component host (§8). Two parts hold only while the WebAssembly sandbox holds: the
  component host drops a call within 1 s once its connection closes, and exits within 1 s once it
  reads end of file on its lifeline pipe from `membraned`. A component that escaped could keep
  running past either, but its connections and egress channel are closed, so it can send nothing.
  The controller stops an orphan left by a SIGKILLed `membraned` (§8).
- **G7. Bounds.** Every call has a deadline, a memory cap and an output cap, and the launch has a
  deadline and a memory cap (§8). Past a call's bound, the call fails with a fixed tool error, its
  instance is dropped, and the component host keeps serving. Every parser of bytes a component, an
  author or a workload writes states its bound and refuses past it, as spec 026 requires: "A parser
  whose input a less-trusted party can write states a bound." (`026:393`). Only the launch deadline
  holds outside the component host; the other bounds hold only while the WebAssembly sandbox
  holds. Nothing outside bounds the process's memory or CPU on the host: Seatbelt bounds neither,
  and a resource limit for it is not measured.
- **G8. Nothing shared.** One component host per attachment, which `membraned` enforces outside the
  process. It is never shared between attachments, plasmids or cells, which narrows spec 017's
  rule that "equal launches may share one original owned broker" (`017:513`). Each call runs in a
  fresh instance, which holds only while the WebAssembly sandbox holds.
- **G9. The bytes that run are the bytes attach pinned.** The component's SHA-256 is recorded at
  prepare. The component host reads the component through its size bound from a descriptor
  `membraned` opened without following a link, and refuses bytes whose digest differs.
- **G10. Invisible.** Nothing in the guest names the component host: no process, mount,
  environment variable or file, except the plasmid's socket. Its path and every reply name only
  the plasmid and its version, never a kernel name.
- **G11. Denials are missing declarations.** A request of the component refused at the boundary is
  reported as spec 011 says: "The report names what was denied and the declaration field it belongs
  in" (`011:409-410`), so it names the host and `network.hosts`, or the port and `network.ports`,
  and nothing that was granted. The relay knows which plasmid's code sent a request, so the report
  names that plasmid and is judged by that plasmid's grants alone (the proposed spec 011 change). A
  host that only another plasmid in the cell declared is reported to the requesting plasmid as
  `network.hosts`. Naming it shows the component nothing it did not send.

### 5. How a tool is found

- **One socket per component plasmid.** Each attached component plasmid has one Unix stream socket
  in the workload's mount namespace, at `/run/tools/<plasmid>.sock`. The path is provisional (Q3).
  It is the guest side of the plasmid's Broker holding, at the target whose key is the Broker's
  name: "key is respectively the recorded guest_path, guest_path, host, host, name or target"
  (`001:1461-1462`). The Broker's name is the plasmid ID.
- **The path is recorded, not derived.** Today spec 001 says "Broker data is exposed at the
  reserved guest Unix path" (`001:1277-1278`), a path under `/.plasmosome/` built from the name. A
  name with `plasmosome` in it is a hint under spec 027. So the Broker capability records its
  guest path, as UdsSocket's recipe does, and nothing derives it. That is spec 017's rule for
  launch paths too: "Paths and arguments are exact payload, not a recipe to be guessed from a
  broker name" (`017:500-501`).
- **The directory exists before any attach.** PID1 makes `/run/tools` before hello, as it makes
  spec 027's directories: "Before hello, PID1 makes the guest's software directories"
  (`027:58`). It is owned by root, mode 0755, empty at readiness, with a mount source, type and
  options that name nothing about Plasmosome. So a plasmid attached later has somewhere to appear,
  as spec 011 promises: "a cell is created already arranged so that what is attached later can be
  reached from inside it" (`011:181-182`). PID1 refuses readiness if the root image has an entry
  there. Spec 027 counts file names under `/run` as hints too, so its reference lanes make the same
  directory and serve the same socket (the proposed spec 027 change).
- **The listener stays while any holding is at it.** As "The shared resolver/TUN/route persist
  while peer bindings exist" (`001:1418`), the socket at a target stays while any Broker holding
  is at it, so a reload's two generations hand over without a moment with no socket.
- **An ID that cannot be a path.** A component plasmid's ID must be one path component, not `.`
  or `..`, and at most 91 bytes, so that `/run/tools/<id>.sock` fits Linux's 108-byte `sun_path`
  with its terminating NUL. Spec 001 refuses such a name only in the membrane's preflight:
  "Unrepresentable names/paths and cross-class projection collisions refuse in preflight"
  (`001:1279-1280`). This spec refuses it before prepare, with 108 at field `id` and a `fix` that
  is the line `id = "<name>"`. The name is the ID with each `/` and NUL replaced by `-`, cut to 91
  bytes at a character boundary, and given the prefix `plasmid-` if the result is `.` or `..`
  (the proposed spec 011 change).
- **What resolves means.** A tool **resolves** when a connection made from inside the workload
  namespace reaches the plasmid's component host, and that host lists the tool. Tool names need be
  unique only within one plasmid, so two plasmids never collide.

### 6. How a tool is called

The socket speaks MCP's tool methods as newline-delimited JSON-RPC 2.0, one message per line. MCP's
stdio transport uses the same framing, and its text says the framing "works unchanged over Unix
domain sockets", read from the revision below and not measured here. So a stdio-to-socket bridge
such as `socat` makes the socket an ordinary stdio MCP server with no Plasmosome client.

- **Two revisions.** The component host serves MCP revision `2026-07-28`, the current one when
  this spec was written, and the previous one, `2025-11-25`, since an unchanged harness may speak
  either (Q3). It does what the `2026-07-28` versioning page asks of a dual-era server, read from
  that page and not measured. A connection whose first request is `initialize` is served under
  `2025-11-25`, handshake and `ping` included, for the rest of that connection. Every other request
  is served statelessly under `2026-07-28`, which carries the version in each request's `_meta`.
  Which revision a given harness speaks is not measured.
- **Methods.** Under `2026-07-28` it serves `server/discover`, `tools/list` and `tools/call`; under
  `2025-11-25`, `initialize`, `notifications/initialized`, `ping`, `tools/list` and `tools/call`;
  under both, `notifications/cancelled`. A `2026-07-28` request missing a required `_meta` field is
  `-32602`, as that revision says of a malformed request. A request naming another revision gets
  `-32022`, with `data.supported` exactly `["2026-07-28", "2025-11-25"]` and `data.requested` the
  revision it named. An `initialize` naming an older revision is answered with `2025-11-25`, as
  that revision's negotiation allows.
- **What a caller sees.** `tools/list` comes from the declaration (G5): each tool's name, its
  sentence as `description`, and `inputSchema` exactly `{"type": "object"}` (W3), in byte order of
  the names, with no pagination. Every `2026-07-28` result carries `resultType` `"complete"`. The
  server's identity in each result's `_meta` is the plasmid's ID and version, nothing else (G10).
  The values of the caching fields that revision requires are the first consumer's to choose
  (W10).
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
  them. The cell's egress relay serves them only to this plasmid's component host, over a private
  egress channel, selected by target and by the owner `{cell, plugin}` it takes from that channel
  (G4). They are never installed in the guest's resolver or network device, so the workload cannot
  reach those hosts.
- **Why.** The workload that calls a tool stands where a requirer stands, and "A requirer gains its
  provider's tools, never its provider's grants." (`011:186-187`). Intent 012 says nothing arrives
  that was not needed. The recipe records the reaching side because recovery and observation read
  only recordings, never a declaration. A plasmid that wants the workload to reach a host declares
  no component, as today. This is owner question Q4.
- **Other sections.** With a component, `[workspace]` is refused before prepare with 103,
  `capability` `unsupported:component:workspace`, until a consumer needs a filesystem import.
  `[commands]`, `[secrets]` and `[model]` stay refused as spec 022 says: "`[commands]` with any
  command: 103" (`022:343`) and "`[secrets]` with any reference, or `[model]`" (`022:345`).
- **Declared tools need a component.** A declaration that names a tool and no `[impl].wasm` is
  refused before prepare with 108 at field `impl`, with the `fix` spec 001 already shows: "a
  section such as `[impl]` followed by `wasm = "component.wasm"`" (`001:115-116`). Nothing could
  serve such a tool, so it could never be reported active, and spec 020 forbids the way around it:
  "do not invent a callable implementation to avoid the SDK/runtime prerequisite"
  (`020:514-515`).

### 8. Launch, lifetime and failure

- **The launch record.** The Broker capability of a component carries its complete launch record:
  the component's absolute path, size and SHA-256; the declared tools, names and sentences, in
  byte order; the guest path; the call and launch limits (W10); and the policy and roots
  Artifacts. Its `BrokerLaunch` names the component host executable in `command`, and its two
  endpoints are ones `membraned` binds itself (the proposed spec 017 change). It is recorded at
  prepare, so recovery and observation never read a declaration, and it fits the frame limit:
  "Complete serialized operations/inverses and enclosing messages must fit spec001's existing
  frame limit" (`017:110-111`). It is the contract spec 017 asks for: "Additional launch inputs
  require an accepted contract" (`017:489-490`). The field names are provisional.
- **How it reaches the process.** `membraned` starts the component host as G3 says, by direct exec:
  "The membrane uses direct exec, not a shell" (`001:1131`), with an empty environment. Its
  descriptors, besides 0 to 2, are exactly these: the policy bytes, the launch record, the
  component bytes from a file opened without following a link, the control and data listeners
  `membraned` bound at the holding's endpoints, the egress channel and the lifeline pipe. Before it
  writes the launch record, `membraned` syncs a pid record `{pid, start_time}` of the new process
  in the cell's `runtime/` directory, so no component code runs before that record exists. Whether
  the pinned policy lets the process accept on a passed listener is not measured. If accepting
  needs `network-inbound`, the membrane's observation reads the process unconfined and readiness
  never comes. Then `membraned` passes each accepted connection as a descriptor instead; task F8c
  measures this and chooses.
- **Launch checks.** In order: apply the policy; read and check the launch record; read the
  component through its bound and check its digest (G9); check that it is a component of a listed
  world version; instantiate it with only the world's imports (G1); call `names`, and check that
  its answer equals the declared tool set. Then answer readiness on the control endpoint, as a
  Broker does (spec 001 §4). The launch has a deadline and an instance memory cap (W10), as spec
  026 asks of a parser of less-trusted bytes: "The bound is a maximum size and, where it applies, a
  maximum nesting depth and a time budget." (`026:393-394`). `membraned` enforces the deadline from
  outside: a component host that has not answered readiness by then is killed, and the attach
  aborts. The memory cap is the limit wasmtime applies at instantiation. How to bound the
  compiler's own memory on Darwin is not measured; task F8a measures the peak for a 64 MiB
  component.
- **Drain.** A graceful withdrawal of a component holding pauses its streams, as spec 024's pause
  column says, and lets each call in flight run until the drain deadline D less a release
  reserve: the smaller of 1,000 ms and half of D, a provisional default, as spec 024's reserves
  are. Then `membraned` closes that holding's remaining host-side connections, the component host
  drops their calls, and the withdrawal goes on. The component host's exit, its reaping and the
  guest's removal of the socket run in the reserve, so the withdrawal ends by D. For a release
  that does not, spec 001's rule stands: "A deadline after partial release is incomplete_effect"
  (`001:1000`). So a call never makes the drain time out, and a graceful `plasmid.remove`
  never answers 106 because of a call (the proposed spec 017 change). Without this, spec 017's
  "Before destructive release, timeout restores that holding's admission" (`017:425-426`) would let
  a running call make a detach fail.
- **Lifetime.** A component plasmid holds a Broker grant, so spec 028 applies: "Every plasmid that
  holds a capability grant has a lifetime" (`028:10`). At B the gate closes new connections, as
  spec 024 says: "A new connection is closed the same way as soon as it is accepted." (`024:382`).
  At the same B, and at an earlier B whenever it moves, `membraned` closes the holding's open
  host-side connections by the supervisor's clock (the proposed spec 028 change). That close is
  needed because a call in flight sends nothing until it finishes, so no admission comes to fence
  it, and spec 028's gate has no timer: "so the fence needs no timer and no controller"
  (`028:125-126`). So no admitted call sends anything after B, and while the WebAssembly sandbox
  holds, the component host drops each call within 1 s of its connection closing (§6), so admitted
  work ends within 1 s of B. Its egress holdings are fenced at the same B. The removal
  that follows starts "within 2 s of B, or of its own next run" (`028:116-117`).
- **Order and reaping.** Within a plasmid, the Broker capability comes after its egress ProxyMap
  capabilities (the proposed spec 022 change), and spec 008 retires effects "in reverse original
  grant order" (`008:303`), so a removal withdraws the Broker before them. Each holding's own
  withdrawal shuts its own component host down and reaps it. On a reload the old holding's process
  goes with the old holding, though a new holding then stands at the same target. When the
  Broker's withdrawal is the removal's first, the process is gone by "H = B + 2 s + D" (`028:62`).
  In a removal that withdraws other holdings first, such as a closure's requirers, "n withdrawals
  can take up to n times D" (`028:158-159`), so the process may go later, though its calls ended
  within 1 s of B. This meets spec 028's ask: "the spec that gives Broker a source must bound its
  accepted work by H" (`028:213-214`). While the controller is down, the close at B still ends
  every call, but the idle process waits for the removal.
- **A component host that dies.** It is reaped and never restarted. Its plasmid's `state` becomes
  `failed`, its socket closes every new connection at once, and `plasmid.remove` or
  `plasmid.reload` clears it. Until then the cell's `membrane.status` is not ready, since a
  membrane is ready only when "every broker answers ready" (`001:723`), and it names the holder:
  "`broker` — the one that held the set back" (`001:727`). A trap inside a call is not a death:
  the instance is dropped and the host keeps serving.
- **An orphan.** The component host holds the read end of a lifeline pipe whose only write end
  `membraned` holds, and a native thread outside WebAssembly exits the host when the lifeline
  reads end of file. So a host whose supervisor died, even by SIGKILL, stops by itself with no
  signal. That exit holds only while the
  WebAssembly sandbox holds (G6), so the controller's signal is the backstop for a host that
  outlived its lifeline. When the controller's supervisor check (spec 023 §8) finds a cell whose
  supervisor is gone, it reads that cell's pid records and stops each process whose pid and start
  time both still match: SIGKILL, then a read that shows the pid gone or its start time changed. A
  record whose start time differs names another process, which is left alone. The record only
  names what to stop; no cell state is derived from it (the proposed spec 023 change). Spec 017
  says "a new process using the same number cannot be signalled or adopted" (`017:522-523`), so
  the proposed spec 017 and spec 001 changes make this stop the one exception. A pid freed and
  reused between the read and the signal is not excluded. Spec 026
  lists the VMM helper's orphan as not covered: "The helper and VM keep running." (`026:95`). A
  component host that outlived its lifeline is stopped at the controller's next supervisor check
  of that cell, which runs at startup and, while serving, "whenever a query to a cell's membrane
  fails" (`023:388`).

### 9. What a failure looks like on each wire

**The control socket** (`plasmid.add`, `plasmid.reload`). Before prepare:

- tools declared and no component: 108 at field `impl`, with the `fix` in §7;
- a component plasmid with `[workspace]`: 103 `unsupported:component:workspace`; no
  `component_runtime`: 103 `unsupported:component`;
- a component plasmid's `id` that cannot be a path: 108 at field `id`, with the `fix` in §5;
- an `[impl].wasm` path with a `..` component or a leading `/`, a missing file, or one outside its
  package: 108 at field `impl.wasm`, with `fix` `wasm = "component.wasm"`;
- a component file over 64 MiB: 108 at field `impl.wasm`, with `detail` naming the bound and `fix`
  exactly `remove this entry`;
- a component file that fails spec 022's other trust checks: 108 with `path` the file, and no field
  or `fix`, as spec 001 says of "a fault in the file rather than in what it declares"
  (`001:135-136`).

After prepare, a failed launch check aborts the transaction, and the reply is 108. Spec 001 says
"a mid-commit failure rolls the prefix back and replies `CommitFailed`-shaped code `103`/`109`"
(`001:612-613`); this is the case it gains.

- a declared tool the component lacks: 108 at field `provides."<key>".tools."<name>"`, with `fix`
  exactly `remove this entry`;
- a **component fault**: a file that is not a component, a world version this spec does not list,
  an import the world does not name, a launch past its deadline or memory cap, or a tool the
  component exports that its declaration does not name. It is 108 with `path` the component,
  `detail` naming the fault, and no field or `fix`. It is a fault in the component, not in what
  the declaration says. A fix that wrote an exported name into the declaration could write a blank
  tool name, which spec 011 refuses (task plasmosome-f324). Spec 001's count, "Exactly three
  refusals an author can cause name no declaration field" (`001:127`), counts refusals of a
  declaration. The proposed spec 001 and spec 011 changes write "refusal of a declaration" there
  and say that a component fault is not one;
- a digest mismatch: 108 with `path` the component and no field or `fix`, since "A 108 the author
  did not cause carries no `fix` either" (`001:135`).

**The tool socket.** Protocol faults are JSON-RPC errors, under either revision. A line that is not
JSON is `-32700`, and the conversation goes on. A line over 1 MiB is `-32600`, and the connection
closes. An unknown method is `-32601`. An unknown tool name, or `arguments` that is not an object,
is `-32602` with `data: {"tool": <name>}`. Everything about the tool itself is a result with
`isError: true` and one text item: the component's own message, or one of four fixed sentences in
plain words:

| Cause | Text |
| --- | --- |
| a trap | `The tool stopped because of an internal error.` |
| the call deadline | `The tool did not finish in time.` |
| the memory cap | `The tool ran out of memory.` |
| the output cap | `The tool's result was too large.` |

A call to a detached or fenced plasmid gets no reply: the connection closes, as for any Broker
stream. A new connection to a removed plasmid fails with `ENOENT`, since its path is gone.

**The component's own requests.** Withheld must look like absent. Spec 027 names the tell:
"`ECONNREFUSED` for a withheld host where an unknown host fails name resolution" (`027:157-158`).
So the relay answers an undeclared host as a name that does not resolve, which the component sees
as the network import's DNS error. A declared host on an undeclared port fails as connection
refused, as spec 024 says of a workload's flow: "A new TCP connect fails with `ECONNREFUSED` when
no other eligible grant covers its host, port and transport" (`024:383`). A certificate that no
trusted root signed fails as the network import's certificate error, and no request byte is sent.
A request cut by a fence or a removal fails as the network import's `connection-terminated`.

### 10. Reported active, and the use witness

**Reported active.** A component plasmid is active only after all of these: the bytes match the
pinned digest; the component instantiates with only imports the world names;
`names` equals the declared tool set; and a probe from inside the workload namespace connected to
the socket, reached the component host, and read the declared tools from `tools/list`. Spec 027's
attach rule is the model: "Attach is not complete until the kernel has probed from inside the
workload namespace" (`027:87`). Any failure aborts the attach and leaves nothing, since "Attach is
all-or-nothing." (`011:451`). After attach, `state` is `active` only while `membraned` observes the
process alive, confined and answering readiness; otherwise it is `failed` (§8). The journal's
Broker recording carries the tool names, so `cell.status` can send spec 001 §3.6's `tools` field
from the journal, and `active` rests on observation, not on the journal alone. This meets
spec 011's acceptance: "A plasmid one of whose declared tools does not resolve is not reported
active." (`011:594`).

**The use witness.** A workload process started by `cell.exec` in a real cell connects to
`/run/tools/github-pr.sock`, calls `pr.read`, and gets the answer of a fixture server. The operator
file maps `api.github.com:443` to that server with `allow_private`, which spec 017 requires for a
private address: "loopback, link-local, private, multicast and unspecified addresses refuse unless
the operator explicitly set `allow_private`" (`017:121-122`). The egress relay trusts the
fixture's certificate through `component_runtime.roots`. The call crosses the guest bridge to the
host and leaves through the egress relay, so it uses "a real supported backend and workload
process, not `ToolRegistry` alone" (`020:515-516`). After `plasmid.remove`, the socket is gone, an
open connection reads end of file, and the component host is reaped. An unmodified MCP client also
calls the tool through `socat`. Follow-up task F13 runs both. Spec 020's two plasmid rows also need
the registry (task plasmosome-zjo).

### 11. What waits for the first consumer

**The first consumer** is `github-pr`, curated, with one tool, `pr.read`: "Read a pull request's
title, body and review state." (`011:344`). Every fixture already names it. Reading a public
repository's pull request needs no token, so it can attach before credential custody exists. It
declares `[network]` `api.github.com` on 443 and a `[mock]` for `simulate`. `pr.comment` and its
credential handle wait for task plasmosome-3g2c. Follow-up task F9 builds it, and its PR edits this
spec to settle W1 to W4 and W10. Until then this spec fixes nothing about these shapes:

- **W1.** The exact WASI imports that carry no authority, and their versions, measured from the
  first consumer's build with the pinned toolchain; WASI 0.2 or 0.3, which also settles W2's
  interface.
- **W2.** The network import. Default: `wasi:http`'s outgoing handler. The component host forwards
  each request's method, URL, headers and body over the egress channel, and the relay makes the
  TLS connection, checks the destination, attaches any credential and returns the response. Only
  `https` URLs are sent. So TLS, certificates and credentials stay outside the component host (G4).
  A raw socket import would put TLS, and so any credential, inside the component. The relay's TLS
  client and HTTP response parser read bytes from a declared host, which the plasmid's author may
  run. They run in `membraned`, outside both confinements, so each is a hostile parser with a
  bound and a fuzz target. The relay follows no redirect, since spec 017's host names are "not
  implicit permission to run a program, follow a redirect or use ambient DNS" (`017:124`). It
  returns a redirect response to the component, and a request to the new location is checked like
  any other.
- **W3.** Input schemas. Default: `{"type": "object"}` plus the declaration's sentence. A schema
  in the declaration is a spec 011 grammar change. A schema from the component would be text a
  model reads that no declaration review covered (G5).
- **W4.** Typed exports, one function per tool, instead of `names` and `call` on JSON text.
- **W5.** Streaming results, progress, and the detail of cancellation.
- **W6.** State across calls. Default: none.
- **W7.** Credential handles: task plasmosome-3g2c's spec, with `pr.comment` as the consumer.
  Default: the relay attaches a credential to a request outside the component host, and no
  credential value ever enters that process (G2).
- **W8.** Workspace access, the model endpoint, and calling a provider's tools from a component.
- **W9.** Component logs. Default: standard output and standard error are discarded.
- **W10.** The limit values and the caching fields of `tools/list`. Default limits: a 30 s call
  deadline, though a call still ends at B (§8); 64 MiB of memory per instance; 1 MiB per line in
  either direction, which is also the output cap; 4 calls at once per plasmid, where a call past
  that waits within its own deadline; a 10 s launch deadline. None is measured.
- **W11.** One socket for the whole cell, for a harness that wants a single server. That is a
  program in the guest image, or later a plasmid's software tree (spec 027), not kernel code.

## Changes proposed to accepted specs

These are proposed text. The PR that adds this spec edits no other document. Each change is
applied by a later reviewed change to the document it amends, before any task that relies on it:
F1 to F5 in this PR's follow-up list, none of which starts while this spec is a draft. Each item
quotes the text it changes with its lines at main 18ea0177.

**Spec 001** (F1).

- §5: replace "The plasmid WIT world (SDK surface) — deferred by design; `plasmid-sdk` is a
  reserved crate with a placeholder world." (`001:1621-1622`) with "The plasmid WIT world is
  provisional, at version 0.1.0, as spec 031 defines it. It is not frozen."
- §1: replace "Exactly three refusals an author can cause" (`001:127`) with "Exactly three refusals
  of a declaration that an author can cause", and "Every other refusal an author can cause"
  (`001:134`) with "Every other refusal of a declaration that an author can cause", so the count
  stays true.
- §1: after "Every other refusal an author can cause carries a field and a `fix`."
  (`001:134-135`), add "A spec 031 component fault is not a refusal of the declaration. A fault in
  the component a declaration names, such as an import its world does not name or a tool the
  declaration does not name, is 108 with `path` the component and no field or `fix`."
- §3.9: after "Absent declarations mean `passthrough`." (`001:576`), add "`state` is `failed`
  while a spec 031 component host of the plasmid has exited and the plasmid is not yet removed or
  reloaded. A `failed` plasmid is never `active`."
- §3.10: after "a mid-commit failure rolls the prefix back and replies `CommitFailed`-shaped code
  `103`/`109` with the rolled-back list." (`001:612-613`), add "A spec 031 launch check that fails
  after prepare aborts the transaction the same way, and the reply is 108 as spec 031 §9 says. An
  attach of a component plasmid is complete only after spec 031 §10's probe from inside the
  workload namespace."
- §4.1: after "`SpawnBroker` carries `{id,name,launch,owner}`" (`001:956`) and the rest of that
  sentence, add "A spec 031 component's Broker capability also carries its `guest_path` and its
  component launch record (spec 017)."
- §4.1: after "Graceful timeout retains the exact holding and its peers." (`001:982`), add "A spec
  031 component Broker's drain does not time out on a running call (spec 017)."
- §4.1: after "it never signals a numeric PID copied from a request or old record."
  (`001:983-984`), add "Spec 031's orphan stop is the one exception, only for a host that outlived
  its lifeline: after the supervisor is gone, the controller signals a component host named by its
  pid record only when the pid and the start time both still match (spec 017). A pid freed and
  reused between that read and the signal is not excluded."
- §4.2: replace "Broker data is exposed at the reserved guest Unix path
  `/.plasmosome/brokers/<hex-encoded UTF-8 name>.sock`;" (`001:1277-1278`) with "Broker data is
  exposed at the capability's recorded `guest_path`, under `/run/tools`, which PID1 makes before
  hello (spec 024 §5). One listener stands at each Broker target while any holding is at it, so a
  reload hands over with no gap;".
- §4.2: after "so the fence needs no timer and no controller." (`001:1305`), add "A spec 031
  component holding adds one timer (spec 028 §4)."
- §4.2: after "Do not close a healthy stream merely because this observation timed out."
  (`001:1321-1322`), add "A spec 031 component Broker is the exception: at the deadline less its
  release reserve, `membraned` closes its remaining host-side connections and the withdrawal goes
  on (spec 017)."
- §4.2: after "Workload edits of resolver configuration grant no bypass." (`001:1420-1421`), add
  "A ProxyMap holding whose recipe says a spec 031 component reaches it is never installed in the
  guest. It adds no DNS answer, route or rule here. The cell's egress relay serves it only to that
  component's host, and takes the owner from the egress channel (spec 017)."

**Spec 024** (F1).

- §5: after the bullet that begins "The network namespace holds only loopback until a ProxyMap
  is installed." (`024:242`), add "- Before hello, PID1 makes `/run/tools`, owned by root, mode
  0755, empty at readiness, with a mount source, type and options that name nothing about
  Plasmosome, as spec 027 §2 makes its software directories. A spec 031 tool socket appears there
  without a restart. PID1 refuses readiness if the root image has an entry there."
- §9: after "Every error in this table is expected, not yet measured" (`024:376`) and the rest of
  that sentence, add "A spec 031 tool socket is a Broker stream in this table."

**Spec 011** (F2).

- Out of scope: replace "Freezing the world is its sibling spec, and it is the larger of the
  two." (`011:645-646`) with "Spec 031 is that sibling. It defines a provisional world, version
  0.1.0, and does not freeze it."
- Behavior and Contract: replace "So every refusal an author can cause" (`011:113`) with "So
  every refusal of a declaration that an author can cause", and "Every refusal an author can
  cause carries" (`011:365`) with "Every refusal of a declaration that an author can cause
  carries", so the count stays true.
- Behavior: after "are the only exceptions (Contract)." (`011:118`), add "A fault in a plasmid's
  component, spec 031's component fault, names the component instead: it is not a refusal of the
  declaration."
- Contract: after "The three name no field and carry no `fix`." (`011:371`), add "A spec 031
  component fault names the component's path instead, and no field or `fix`: it is a fault in the
  component a declaration names, not a refusal of the declaration."
- Contract: after "A plasmid is reported active only when every tool its declaration names
  resolves." (`011:453`), add these two bullets.
  - "**A declared tool needs a component.** A declaration that names a tool and no `[impl].wasm` is
    refused at attach with 108 at field `impl`, and `fix` `[impl]` followed by the line
    `wasm = "component.wasm"`, since nothing could serve the tool (spec 031)."
  - "**A component plasmid's `id` names a socket.** With `[impl].wasm`, the `id` must be one path
    component, not `.` or `..`, of at most 91 bytes, so that `/run/tools/<id>.sock` fits Linux's
    108-byte `sun_path` with its NUL. Another is refused at attach with 108 at field `id`, and
    `fix` the line `id = "<name>"`: the `id` with each `/` and NUL replaced by `-`, cut to 91 bytes
    at a character boundary, and given the prefix `plasmid-` if the result is `.` or `..` (spec
    031 §5)."
- Contract, *Whom*: after "no `network.hosts` denial reaches a plasmid's `denials`."
  (`011:432`), add "Spec 031's component egress relay is such a boundary: it takes the plasmid
  from the egress channel a request came on. For a component's own request, *What* reads that
  plasmid's grants, not the cell's, and the report names that plasmid. A host that only another
  plasmid holds is not granted to it, so its report may name that host."

**Spec 020** (F2).

- Behavior: replace "It does not freeze the component WIT world" (`020:40`) with "Spec 031
  defines the component WIT world as provisional; this spec does not freeze it".
- Delivery boundaries: replace "The existing spec011 gate question and component interface remain
  outside this spec" (`020:597-598`) with "The existing spec011 gate question remains outside this
  spec, and the component interface is spec 031's".

**Spec 017** (F3).

- The capability table: replace the row "| `Broker` | `name`, `launch: BrokerLaunch` |"
  (`017:72`) and the rest of that row with "| `Broker` | `name`, `launch: BrokerLaunch`,
  `guest_path`, `component` | `broker/{name}` |".
- After "`BrokerLaunch` is the strict serde record" (`017:478`) and the paragraph it begins, add
  "`guest_path` is the absolute guest path of the holding's data socket, recorded and never
  derived. `component` is null, or a spec 031 component launch record: the component's path, size
  and SHA-256, its declared tools, its limits, and the policy and roots Artifacts. For a component,
  `command` is exactly the component host's absolute path, from `component_runtime.host`, so the
  executable is named once. `control_socket` and `data_socket` are endpoints `membraned` binds in
  the cell's private directory, under spec 001 §4.1's rules, and passes to the component host as
  listening descriptors, so the component host binds no socket. The controller names each holding's
  endpoints by its `GrantId`, so a reload's two holdings at one target never share one." That keeps
  "A different launch description cannot silently take over either occupied endpoint"
  (`017:497-498`) true.
- Resolved recipes: replace "Broker, UdsSocket and SessionFile have no declaration source yet."
  (`017:131-132`) with "Broker's declaration source is a declaration's `[impl].wasm` (spec 031);
  UdsSocket and SessionFile have none yet."
- ProxyMap: after "equal-host selector changes, as specified by spec001 §4.2." (`017:407`), add
  "`ProxyRecipe` gains `reached_by`, exactly `workload | component`. A `component` holding is never
  installed in the guest. The relay serves it only to its own plasmid's component host, over a
  private egress channel, and takes the owner `{cell, plugin}` from that channel, never from a
  request's bytes (spec 031)."
- Drain: after "Before destructive release, timeout restores that holding's admission and
  preserves its full authority and peers." (`017:425-426`), add "A spec 031 component Broker's
  drain does not time out on a running call. At the deadline less a release reserve, the smaller
  of 1,000 ms and half the deadline, `membraned` closes the holding's remaining host-side
  connections, which ends their calls, and the withdrawal goes on. The component host's exit, its
  reaping and the guest's removal of the socket run in the reserve, so the withdrawal ends by the
  deadline; one that does not is `incomplete_effect` (spec 001 §4.1). The reserve is a provisional
  default. So a component's call never makes a graceful removal answer 106."
- Versions: replace "The single-plugin `LogRecord` now requires `format: 3`." (`017:562`) with
  "The single-plugin `LogRecord` requires `format: 3` or `format: 4`.", and "Writers and readers
  accept only version3." (`017:565`) with "Readers accept only version3 and version4, and a writer
  that implements spec 031 writes only version4."
- Versions: after "so any later incompatible change to these records requires another explicit
  version transition, not a silent change inside format3." (`017:571-572`), add "Spec 031's
  `ProxyRecipe.reached_by` and Broker `guest_path` and `component` are such a change, so they come
  with format4, which requires them and gives them no default. A format4 reader also reads format3
  records as what they meant, since no component holding existed before format4: each ProxyMap as
  `reached_by` `workload`, and each Broker with `component` null and `guest_path` the path spec 001
  §4.2 then derived from its name, under `/.plasmosome`, the one exception to spec 001 §4.2's
  `/run/tools`. One file may hold format3 records followed by format4 records, which is what a
  format4 writer's append to an existing log produces. A format3 record after a format4 record
  refuses. Each record validates under its own format. A format4 writer writes only format4." A
  version, and not an optional field, because a missing field read as `workload` would install a
  component's holding
  in the guest, the wider reading, and spec 008 says "Required fields have no defaults" (`008:89`).
- Append and open: in "append validates all existing complete LF-terminated version3 records"
  (`017:578-579`) and "must parse completely with version3" (`017:584`), write "version3 or
  version4" for "version3": each record is checked under its own format, and no version3 record
  may follow a version4 one.
- Broker launch: after "equal launches may share one original owned broker" (`017:513`) and the
  rest of that paragraph, add "A launch with a spec 031 `component` record is never shared: each
  holding has its own component host."
- Process authority: after "a new process using the same number cannot be signalled or adopted."
  (`017:522-523`), add "Spec 031's orphan stop is the one exception, only for a host that outlived
  its lifeline: after the supervisor is gone, the controller signals a component host named by its
  pid record only when the pid and the start time both still match. A pid freed and reused
  between that read and the signal is not excluded."

**Spec 008** (F3).

- After "is the format the baseline writes and reads." (`008:84-85`), add "Spec 031 adds spec017's
  single-plugin version4. A writer that implements spec 031 writes version4, and its reader also
  reads version3."
- Replace "For brokers the capability is `{name, launch:BrokerLaunch}`, never a PID." (`008:131`)
  with "For brokers the capability is `{name, launch:BrokerLaunch, guest_path, component}` (spec
  017 and spec 031), never a PID."
- In "single-plugin logs, the implemented version3 and the refused version2" (`008:135-136`),
  write "version3, spec 031's version4 and the refused version2".
- After "This registry-source addition is part of the still-unimplemented per-cell format1 before
  its first writer ships." (`008:157-158`), add "So are spec 031's `ProxyRecipe.reached_by` and
  Broker `guest_path` and `component`, required with no default. Spec 017's single-plugin format4
  adds the same fields and still reads format3."
- Replace "The single-plugin version3 contract, the refusal of version2, and the refusal to migrate
  either into cell histories are unchanged." (`008:158-160`) with "The single-plugin version3 and
  version4 contracts, the refusal of version2, and the refusal to migrate any of them into cell
  histories are unchanged."

**Spec 022** (F3).

- Behavior: replace "Broker, UdsSocket, SessionFile and credentials are owner questions."
  (`022:35`) with "Spec 031 gives Broker a source; UdsSocket, SessionFile and credentials are owner
  questions."
- §5: after "A declaration that fails §3, or whose `id` differs: 108 with `path` the declaration."
  (`022:177`), add "- A component file that `[impl].wasm` names is opened under §3 too, relative
  to its declaration's directory, with a size bound of 64 MiB instead of rule 4's. A `wasm` path
  with a `..` component or a leading `/`, a missing file, or, for a registry member, a file its
  package does not list, is 108 at field `impl.wasm` with `fix` `wasm = "component.wasm"`. A file
  over 64 MiB is 108 at `impl.wasm`, with `detail` naming the bound and `fix` exactly
  `remove this entry`. One that fails another trust check is 108 with `path` the component, and no
  field or `fix`."
- §6: replace "`[model].endpoint`, `wasm` and `[provides]` add no capability." (`022:351`) with
  "`[model].endpoint` and `[provides]` add no capability. `wasm` yields one Broker capability for
  the plasmid's component (spec 031), and that plasmid's `[network]` needs yield ProxyMap
  capabilities reached by the component. With `wasm`, `[workspace]` is 103, `capability`
  `unsupported:component:workspace`. Without the configuration's `component_runtime`, `wasm` is
  103, `capability` `unsupported:component`."
- §7: after "Each member yields complete capabilities, each with a fresh `GrantId` chosen before
  prepare and no reference back to the operator file." (`022:356-357`), add "A member with `wasm`
  yields its ProxyMap capabilities before its Broker capability, so a removal, which retires in
  reverse grant order (spec 008), withdraws the Broker first (spec 031 §8)."
- §8: after "`read_write` where the held Mount was `read_only`." (`022:375`), add two items: "- a
  ProxyMap reached by `workload` at a host, port and transport the plasmid held reached by
  `component`;" and "- a Broker it did not hold, or a declaration that drops the held Broker's
  `[impl].wasm` while it keeps any `[network]` need (spec 031)." Spec
  022's acceptance item 7 gains the case of a reload that drops `[impl].wasm` and keeps `[network]`
  and an empty tools table.
- Open question 1: after "No declaration section selects them." (`022:545`), add "For Broker,
  spec 031 answers: `[impl].wasm` is its source, and no author-written Broker section exists."

**Spec 023** (F3).

- §1: after "Invalid configuration refuses startup, as spec 001 §4.1 says." (`023:74`) and the
  rest of that paragraph, add "The configuration also takes an optional `component_runtime`, spec
  031's deployment input. `membraned` never reads it: the controller puts its Artifacts in each
  component's Broker capability."
- §1: replace "Spec 001 §4.2's immutability rule for the helper applies to `membraned` and to every
  Artifact in the template." (`023:78-79`) with "Spec 001 §4.2's immutability rule for the helper
  applies to `membraned`, to every Artifact in the template and to every Artifact in
  `component_runtime`."
- §5: after "That is owner decision O-10; this spec makes such a cell visible and does not clean
  it." (`023:226-227`), add "A spec 031 component host of such a cell is the one exception: it
  exits when its lifeline reads end of file, and the supervisor check stops one that outlived its
  lifeline (§8). The helper and guest stay as this paragraph says."
- §8: after "It never touches an entry spec 008 does not validate." (`023:395`), add "When the check
  finds a cell whose supervisor is gone, it also stops that cell's spec 031 component hosts. It
  reads each pid record `{pid, start_time}` that `membraned` wrote under the cell's `runtime/`
  directory, through the same no-follow opens. It sends SIGKILL to a process only when its pid and
  start time both still match, then confirms by a read that the pid is gone or its start time
  changed. A record whose start time differs names another process, which it leaves alone. The
  record names only what to stop; no cell state is derived from it."
- Open question 1, O-10: after "This spec sets that cell aside and does not clean it."
  (`023:674-675`), add "Spec 031's component hosts are the exception: each exits on its lifeline,
  and §8 stops one that outlived it. O-10 asks about the helper and the guest only."

**Spec 026** (F4).

- Threat model: after "A plasmid's manifest and its broker's readiness replies are written by its
  author" (`026:74-75`) and the rest of that item, add "So is a spec 031 component, which runs on
  the host inside a confined component host. The standard defends against its code as well as its
  bytes."
- Not covered yet: after "An orphan after `SIGKILL` of `membraned`" (`026:95`) and the rest of
  that item, add "A spec 031 component host exits when its lifeline reads end of file. One that
  outlived its lifeline is stopped at the controller's next supervisor check of that cell, by its
  pid and start time (spec 023 §8)."
- Not covered yet: replace "A broker is such a process: it runs unconfined" (`026:102-103`) and
  the rest of that sentence and the next with "A broker is such a process, behind spec 001's and
  spec 017's peer-UID checks. A spec 031 component host is confined by its own pinned policy and
  the WebAssembly sandbox, so for it the standard defends against code as well as bytes. Any other
  broker runs unconfined, and the standard defends against its bytes, not its code."
- The parser rows land with each parser, as "A new parser lands in the same PR as its row below and
  its bound." (`026:402`) requires. That is tasks F8a, F8b and F8d, for the component host's
  reading of the component file and of its launch record, its line reader, and its reader of the
  relay's responses, and task F11b, for three parsers of the relay, which runs in `membraned`,
  outside both confinements. The component file is a plasmid author's bytes and the line reader's
  are the workload's, so those two rows also name a fuzz target. The relay's parser of a
  component's egress requests has the component host as its writer, and so possibly a component
  that escaped the sandbox. The relay's TLS client and HTTP response parser read bytes from a
  declared host, which the plasmid's author may run. So each is hostile and gets a fuzz target
  too: "A parser whose writer is hostile also has a fuzz
  target." (`026:400`).

**Spec 027** (F5).

- §6: after "When the cell lane attaches or detaches during a run, both reference lanes install or
  uninstall the same files by hand at the same moment" (`027:197-198`) and the rest of that
  bullet, add "- A spec 031 component plasmid attached in the cell lane is matched in both
  reference lanes by an ordinary tool server at the same socket path, serving the same tools and
  reaching a fixture for each host the component declares. Those hosts are outside the workload's
  G. All three lanes make `/run/tools` the same way."
- §7: remove the item "Spec 001 §4.2's brokers at `/.plasmosome/brokers/`, a path that names the
  membrane." (`027:321`). Under spec 031 no broker path is under `/.plasmosome`, and every lane
  makes `/run/tools` the same way.
- Item 13: replace "a second entry beside `/.plasmosome`" (`027:536-537`) with "an entry under
  `/run/tools` in one lane only", since nothing in the guest is under `/.plasmosome` once spec 001
  §4.2 records each Broker's guest path.

**Spec 028** (F5).

- §4: after "so the fence needs no timer and no controller." (`028:125-126`), add "A spec 031
  component holding adds one timer: at B, and at an earlier B whenever it moves, `membraned` closes
  that holding's host-side connections by the supervisor's clock. A call in flight sends nothing
  until it finishes, so it meets no admission to fence it."
- §5: replace "Spec 022's version 1 declares no Broker; the spec that gives Broker a source must
  bound its accepted work by H." (`028:213-214`) with "Spec 031 gives Broker a source and bounds a
  component host's accepted work: at B `membraned` closes its connections, so no admitted call
  sends anything after B, and while the WebAssembly sandbox holds its calls end within 1 s. Each
  holding's withdrawal reaps its own process. Any other Broker still has no source."
- §7: replace "Admitted work stops by B + D; a Broker's does not" (`028:245`) with "Admitted work
  stops by B + D. For a spec 031 component host, at B `membraned` closes its connections, so no
  admitted call sends anything after B, and while the WebAssembly sandbox holds its calls end
  within 1 s. Any other Broker's work does not stop by B + D".

## Open questions

Each has the default this spec uses until the owner answers.

1. **Q1. Where plasmid code runs.** Default: (a), on the host, as §1 says. The alternative, (b),
   runs it in the guest beside the workload. (b) keeps foreign code behind the hardware boundary. It
   needs a guest process the workload can see, or a PID namespace; a helper process in PID1, which
   spec 024 forbids today; one network namespace per plasmid, so that the plasmid's flows can be
   told apart from the workload's; and a credential handle honoured inside the guest.
2. **Q2. The engine, and what it brings.** Default: wasmtime with the fewest features, compiling
   with Cranelift inside the confined process (§2). Neither Cranelift nor Pulley gives a tier 1
   configuration on `aarch64-apple-darwin`, which wasmtime puts in tier 2. That is an inference
   from wasmtime's tiers document; its vulnerability policy counts a bug in a tier 1 feature, so a
   Cranelift bug may still count on a tier 2 target. Cranelift writes native code from plasmid
   bytes on the host. Whether the pinned policy can allow that memory while it denies everything
   else is not measured, and G3 rests on it. Task F8c measures it first. It also records whether
   the component host is signed with the hardened runtime's `com.apple.security.cs.allow-jit`
   entitlement, which Apple documents that `MAP_JIT` needs under that runtime. If the policy cannot
   allow the memory narrowly, the fallback is Pulley, which writes no native code, at a cost in
   speed that is not measured. The dependency closure is large: wasmtime and its WASI and HTTP
   crates in the component host, and rustls and an HTTP client in the relay. Each direct dependency
   needs a spec 026 record and review. Is that set, on a tier 2 target, acceptable?
3. **Q3. The in-cell wire and its path.** Default: MCP revisions `2026-07-28` and `2025-11-25`,
   served as a dual-era server, over newline-delimited JSON-RPC at `/run/tools/<plasmid>.sock`,
   one socket per component plasmid (§5, §6), since an unchanged harness may speak either.
   Alternatives: serve only `2026-07-28`, one protocol to keep, though a harness that speaks only
   an older revision cannot then use the socket, as MCP's versioning page says: "legacy clients
   have no fall-forward mechanism"; a Plasmosome JSON-RPC with an MCP bridge later; or one socket
   for the whole cell, which needs routing by tool name on the host.
4. **Q4. Whose reach a component's needs are.** Default: its component's only (§7). Alternative:
   the workload's too, which gives a caller grants it never declared.
5. **Q5. Spec 022's question 1, for Broker.** Default: `[impl].wasm` is Broker's only declaration
   source, and no author-written Broker section exists. UdsSocket and SessionFile stay without a
   source.
6. **Q6. The gate, now that a plasmid's code runs on the host.** Default: nothing new gates
   attach. A component plasmid enters a cell the two ways any plasmid does, an operator-file
   listing or a spec 020 import. Its code reaches only what its closure's declarations and the
   operator file allow, as spec 011 says: "Declarations are enough to bound reach" (`011:293`). For
   a component on the host, that holds only while G3 holds, on a tier 2 target. If G3 fails, the
   worst case is what the host user can reach: the user's files, and through any same-UID deputy
   the controller's socket, which "proves a UID, not a person" (`028:281-282`). Should a
   user-population component be refused until the gate of intent 010 exists? That would block
   spec 020's user plasmid row, whose publisher "publishes its own package without curator
   authority" (`020:509`).
7. **Q7. When the world is frozen.** Default: not by this spec. It stays `0.x` and changes by edits
   to this spec. `plasmid new` keeps refusing a component, and spec 007's first blocker stays open
   until the owner records a freeze.

## Acceptance

Each item names its level, the guards it proves, and the broken runtime it catches: the mutants its
test must kill. Host and membrane items use components written as WAT text, so they need no wasm
target.

1. **Tools come from the declaration (host; G5).** A component exporting exactly the declared
   tools lists each with exactly its declaration sentence and `{"type": "object"}`, in byte order.
   Catches: sentences taken from the component; a list built before the `names` check.
2. **A missing tool fails the launch (host, then model).** A component lacking a declared tool
   fails launch, and the attach replies 108 at that tool with `fix` `remove this entry`, leaving
   nothing. Catches: a check of exported against declared in one direction only; `active` without
   the check.
3. **An extra tool is a component fault (host, then model).** A tool the component exports and the
   declaration lacks fails launch with 108, `path` the component and `detail` naming the tool, and
   no field or `fix`. A component that exports a tool with an empty name fails the same way.
   Catches: the other direction checked alone; a fix that writes an exported name, even a blank
   one, into the declaration.
4. **An undeclared import fails the launch (host; G1).** A component importing
   `wasi:sockets/tcp` fails launch with 108 naming the component and the import, and no field or
   `fix`. A component importing only the world's interfaces, as F7 writes them, launches. Catches:
   a host that stubs an unknown import, or traps only when it is called.
5. **Imports with no authority are empty (host; G1, G2).** A component that reads its environment,
   its arguments and its preopened directories gets three empty lists, and its standard input reads
   end of file at once. Catches: wasmtime's inherit defaults; a host that passes its own arguments,
   which name its path, or its environment to the component.
6. **Refusals before prepare (model).** A declaration with a tool and no `[impl].wasm` is refused
   with 108 at `impl` and the `[impl]` fix. Without `component_runtime`, a component plasmid is
   103 `unsupported:component`; with `[workspace]`, 103 `unsupported:component:workspace`. An `id`
   of `a/b` is 108 at `id` with the line `id = "a-b"`; an `id` of 91 bytes is accepted and one of
   92 is refused. A `wasm` of `../x.wasm`, of `/x.wasm` or of a missing file is 108 at `impl.wasm`
   with `wasm = "component.wasm"`, and a file of 64 MiB and one byte is 108 at `impl.wasm` with
   `remove this entry`. Nothing is journaled in any case. Catches: an attach that reports active a
   plasmid nothing serves; a refusal found only after prepare; an author refusal with no field; an
   ID bound off by one.
7. **The pinned bytes are the bytes that run (host; G9).** Bytes changed after the digest was
   recorded fail launch, and so does a component path that is a symbolic link. Catches: hashing a
   path and then opening it again.
8. **The process is confined before it reads (host; G2, G3).** A test build of the component host
   tries, after launch: to open a host file; to open a TCP socket; to connect to a Unix socket that
   the test binds as the same user, outside the passed descriptors; to signal another process of
   the same user; to exec `/bin/true`; to look up a Mach service, such as the security daemon's;
   to read another same-user process's arguments and environment through `sysctl` with
   `KERN_PROCARGS2`; and to read that process's `proc_pidinfo`. Each fails, and the membrane's
   observation reads confined. The test feeds the
   component through a pipe and writes its first byte only after the observation reads confined,
   so a host that reads before it confines is seen unconfined while it waits. Catches: confinement
   after parsing; a policy that allows file, network, Unix-socket, signal, exec, Mach or
   process-information access, or `sysctl` reads under every name, including one the observation
   reads as confined.
9. **Only the passed descriptors survive (membrane; G3).** The test reads the component host's
   descriptor table from outside, with `proc_pidinfo` and `PROC_PIDLISTFDS`, and finds exactly the
   passed descriptors and 0 to 2 on `/dev/null`. A mutant `membraned` that leaves its 4090
   control-lane descriptor, or another plasmid's egress channel, open across exec fails the item.
   Catches: a close-on-exec flag set after creation; an inherited descriptor, which no pathname
   policy denies.
10. **The policy and the confinement are checked from outside (membrane, then model; G3).** A test
    build of the component host that skips its policy never reaches readiness, and the attach
    aborts and leaves nothing. Policy bytes whose SHA-256 differs from `component_runtime.policy`'s
    refuse the launch before exec, as for the helper: "a different generic deny-default policy is
    not equivalent" (`001:1156`). Catches: a `membraned` that trusts the component host's own
    report; a policy checked by path, not by bytes.
11. **The component host sends only HTTP (host; G2, G4).** With a fake relay, a call reaching the
    declared host gets the fixture's answer, and the fake relay receives an HTTP request over the
    egress channel, never a DNS query, a TLS handshake or a credential. When the fake relay answers
    as §9 says, an undeclared host fails as an unresolved name, and a declared host on an
    undeclared port fails as refused. Catches: a component host that resolves names, makes TLS
    itself or holds a root; a denial shown as a policy error instead of an absent host.
12. **The relay enforces by owner (membrane, then pinned runtime; G4).** A test program standing in
    for a component host sends `membraned`'s egress relay, over one plasmid's component egress
    channel, three requests: one for a host its plasmid did not declare, one for a host only
    another plasmid in the cell declared, and one whose own bytes name the other plasmid as its
    owner. All three are refused as §9 says, and no connection opens. Catches: enforcement only
    inside the component host; selection by target alone; an owner read from the request.
13. **The relay checks certificates (membrane; G4).** A declared host whose fixture presents a
    certificate that no trusted root signed fails as the network import's certificate error, and
    the fixture receives no request byte. Catches: a relay that skips verification; roots taken
    from the request.
14. **Denials name what was missing (model; G11).** The denials of items 11 and 12 appear in the
    session log and in the requesting plasmid's `denials` with `network.hosts` or `network.ports`.
    The host only another plasmid in the cell declared is reported to the requesting plasmid as
    `network.hosts`, and no host the requesting plasmid holds appears anywhere. Catches: a report
    that lists what was granted; a report judged by the cell's grants, which drops that host or
    names no plasmid.
15. **The deadline (host; G7).** An infinite loop returns `isError` with the deadline sentence
    within the deadline plus 1 s, and the next call succeeds. Catches: no interruption; a poisoned
    shared instance.
16. **The memory cap (host; G7).** Memory growth past the cap returns `isError` with the memory
    sentence, and the host keeps serving. Catches: no cap, or a host that dies with the instance.
17. **The output cap (host; G7).** A result over the output cap returns `isError` with the output
    sentence, never a truncated success. Catches: truncation.
18. **The launch is bounded (host, then membrane; G7).** A component built to compile slowly is
    killed at the launch deadline, and the attach aborts with a component fault naming the
    deadline. A component whose initial memory is over the cap fails instantiation the same way.
    Catches: an unbounded compile; a deadline that only the component host enforces.
19. **Protocol faults and tool faults stay apart (host; G7).** Under each revision, a non-JSON line
    is `-32700` and the next request is answered; a line over 1 MiB is `-32600` and the connection
    closes; an unknown method is `-32601`; an unknown tool is `-32602` with `data.tool`. Catches:
    protocol faults returned as tool results, or the reverse; an unbounded line reader.
20. **Both revisions, and the version errors (host).** An `initialize` naming `2025-11-25` gets
    that revision's handshake, and `tools/list` and `tools/call` on that connection follow it. A
    `2026-07-28` request with its `_meta` works with no handshake. One missing a required `_meta`
    field is `-32602`, and one naming `1900-01-01` is `-32022`, with `supported` exactly
    `["2026-07-28", "2025-11-25"]` and `requested` `1900-01-01`. Catches: a host that ignores the
    version; one revision served where two are claimed; legacy state carried to another connection.
21. **A fresh instance per call (host; G8).** A component that counts its calls returns 1 every
    time. Catches: instance reuse.
22. **A closed connection drops its call (host; G6).** With the limit of 4 calls at once per
    plasmid, four calls into an infinite loop are started and their connections closed. A fifth
    call then starts within 1 s and succeeds. Catches: a call that keeps running after its caller
    is gone, holding a slot and the CPU until its own deadline.
23. **Attach probes from inside (pinned runtime).** A process started before the attach sees the
    socket appear without a restart, and the attach returns only after the probe from inside the
    workload namespace. With the probe made to fail, the attach aborts and leaves nothing.
    Catches: a probe from the host side only; a socket that needs a restart.
24. **Removal leaves nothing running (pinned runtime; G6).** After `plasmid.remove` answers
    success, a new connection fails with `ENOENT`, an open connection reads end of file, a call in
    flight has ended, the component host is gone, and its egress flows are closed. The process's
    absence is checked with its start time, since `sandbox_check` answers denied for a dead pid.
    Catches: asking the component to stop; a socket removed but a process alive; a process killed
    but a socket left.
25. **Detach never waits on the component (membrane, then pinned runtime; G6).** A component in an
    infinite loop, under a call deadline longer than D, is detached by a graceful `plasmid.remove`
    whose Broker withdrawal answers success within D of its own call, never 106, and the call's
    connection is closed at D less the release reserve. Catches: a drain that waits on the call and
    times out, which would let a plasmid refuse a detach; a call cut before D less the reserve; a
    release run past D.
26. **The lifetime bound (membrane, then pinned runtime; G6).** At B new connections close at once
    and `membraned` closes the open ones, with no request arriving after B. A call admitted before
    B ends within 1 s of B; the egress holdings are fenced; the Broker holding is the removal's
    first withdrawal; and the process is gone by H. With the controller stopped, the call still
    ends within 1 s of B. Catches: Broker work past H; a fence that waits for an admission; a fence
    that only removal provides; a Broker withdrawn after its egress holdings.
27. **The lifeline (host; G6).** With a call running an infinite loop, SIGKILL of the process
    holding the pipe's write end makes the component host exit within 1 s. Catches: an orphaned
    component host; a lifeline read on the thread that runs the call.
28. **An orphan is stopped (model; G6).** After `membraned` is SIGKILLed, a component host built to
    ignore its lifeline is stopped by the controller's supervisor check, through its pid record,
    and is confirmed gone by its start time. A record whose pid now belongs to another process,
    with a different start time, leaves that process running. Catches: an orphan nothing stops; a
    kill by pid alone, which could stop an unrelated process.
29. **A dead component host is `failed` (pinned runtime).** A component host that aborts leaves
    its plasmid `failed` in `plasmid.list`; new connections close at once;
    `membrane.status` names its broker; nothing restarts it; and `plasmid.remove` completes.
    Catches: `active` after the tools stopped resolving; a silent restart.
30. **Two cells, two processes (pinned runtime; G8).** Two cells with the same plasmid have two
    component hosts, and detaching one leaves the other's calls working. After a reload, the old
    generation's process is gone, checked with its start time, and the new one serves. Two
    component plasmids in one cell each reach only their own component host through their own
    socket, so `tools/list` on each shows only that plasmid's tools. Catches: sharing across
    cells, plasmids or holdings; a socket routed to another plasmid's host; a component host
    reaped only at the last withdrawal at its target.
31. **A reload cannot move a component's reach (model; G4).** A plasmid attached with
    `[impl].wasm`, a tool and `[network]` `api.github.com:443` is reloaded with the component and
    its tool removed and `[network]` kept: once with no tools table and once with an empty one.
    Each is refused with 109 before prepare, and the workload never reaches the host. Catches: a
    widening check that compares hosts but not who reaches them.
32. **Nothing names the membrane (pinned runtime; G10).** The guest shows no process, mount,
    variable or file of the component host. The socket's path and replies match the ordinary tool
    server of spec 027's reference lanes. Task F14 runs it. Catches: kernel names leaking into the
    guest.
33. **An unchanged MCP client calls a tool (pinned runtime).** An MCP client at a pinned version,
    unmodified and named by task F13, lists and calls `pr.read` through `socat` from inside the
    workload, under each revision it speaks. Catches: a socket no real client can use; a default
    revision no harness speaks.
34. **A controller restart changes nothing (model).** After a controller restart the component
    host is still running and its calls work, and recovery opens no declaration. Catches: recovery
    that reads declarations again or launches a second process.
35. **A host stops itself when its supervisor dies (model; G6).** With a call running an infinite
    loop, `membraned` is SIGKILLed. The component host exits within 1 s with no signal sent to it,
    and a supervisor check of that cell run after that second finds no host to stop and sends no
    signal. Catches: a host that ignores its lifeline; a lifeline read on the thread that runs the
    call.
36. **The journal's formats (model).** A format4 ProxyMap record without `reached_by`, or a Broker
    without `component`, refuses. A format3 ProxyMap reads as `reached_by` `workload`, and a
    format3 Broker as `component` null with its `guest_path` under `/.plasmosome`. A file of
    format3 records followed by format4 records opens, each record checked under its own format.
    A format3 record after a format4 record refuses. Catches: `reached_by` read as optional with a
    default; a format4 reader that refuses format3; a reader that takes format3 after format4.

## Out of scope

- Freezing the world, publishing `plasmid-sdk`, and a versioning policy: spec 007 and the owner.
- Credential custody and a credential import: task plasmosome-3g2c's spec.
- The approval gate of intent 010, which spec 011 leaves to the owner: "Where the gate sits, who
  holds it" (`011:319-320`).
- A component skeleton from `plasmid new`, which stays refused while the world is not frozen.
- Workspace, model-endpoint and provider-tool imports, streaming, and state kept across calls.
- A cell-wide tool aggregator for an unchanged harness (W11).
- A Linux host, and the Linux mechanism for the component host's confinement.
