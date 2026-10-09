---
id: 022
title: Trusted operator declarations, and how a manifest becomes exact grants
status: draft
intents: [004, 012, 003]
---

## Behavior

A plasmid's declaration says what the plasmid needs in its author's words: these hosts on these
ports, a workspace at this guest path. Spec 017 grants nothing that vague. It needs complete
recipes: where a proxied host really connects, over which transport, and which host directory
backs a mount, with what access. So the work is split. **The declaration decides what is
reachable**: which hosts and ports, and which workspace path. **The trusted operator decides
where each leads on this host**: the real destination, the host directory and its access. The
operator writes those facts in one file, the **operator declarations file**. The controller
combines the two into the exact capabilities a plasmid holds. A need the declaration does not
name gets nothing, whatever the file says; a need the file does not cover is refused.

Resolution runs once per mutation that attaches plasmids (`plasmid.add`, `plasmid.reload`, and
`cell.new` with a genome), before anything is prepared. Every declared host and port, and every
declared workspace, must be covered by an operator entry for the plasmid's mock mode, or the
request refuses with 103 and nothing is prepared. The output is a list of complete spec 017
capabilities with fresh identities; spec 017 and spec 008 take it from there. Recovery never
reads the operator file: it replays the recipes the journal recorded. Editing the file changes
no capability a cell holds. A change reaches a cell only through an add, or a reload that does
not widen.

The controller and its cells run as one host UID, so a file's owner says nothing about who wrote
it: a workload writes through a read-write Mount as that same UID. The operator file and every
declaration it lists are trusted only when no cell can write them, no other principal can
replace them, and they sit outside every Mount source. This serves intent 004 (a capability is
complete before it is granted), intent 012 (a grant comes from a declared need) and intent 003
(recovery works from the journal alone). It covers the two classes a declaration can express
today, ProxyMap and Mount; Broker, UdsSocket, SessionFile and credentials are owner questions.

**Platform.** macOS first: Darwin arm64 is the only runtime host, and the cell is a libkrun Linux
guest. A Linux host is deferred, not dropped; the resolver is portable logic plus file checks
defined for both. A real ProxyMap needs owner decision O-11, and a real Mount needs O-7.

## Contract

### 1. Where the file comes from

The controller's strict JSON configuration gains an optional key, `operator_declarations`: an
absolute, NUL-free path string. Nothing supplies a default. It is not inferred from
`instance_root`, `registry_root`, the control socket, the caller's HOME, the working directory or
a request field. A relative or non-string value is invalid configuration, refused at startup.

When the key is absent, the controller behaves as if the file existed and listed no plasmid:

- a plasmid selected by `artifact` (spec 020) that declares no network and no workspace attaches;
- any `plasmid.add` without `artifact` is 101, because local selection needs a listing (§5);
- any declared network or workspace need is 103, because nothing covers it.

### 2. When it is read, and what never reads it

The file is read once per resolving request, at the start of resolution. One read serves every
member of that request's closure. Each later request reads it again.

Startup, recovery, `plasmosome.status`, `plasmosome.recovery`, `cell.list`, `cell.status`,
`plasmid.list`, `plasmid.remove`, `cell.kill` and every withdrawal never open it. A missing,
unsafe or corrupt file cannot block startup, recovery or removal.

### 3. Trust checks

The same checks apply to the operator file and to every declaration file it lists. A file that
fails one refuses the request with 108, `path` the file's absolute path, and `detail` naming the
check (`path`, `ancestor`, `parent`, `not_regular`, `owner`, `links`, `mode`, `size`,
`inside_mount_source` or `content`). These are operator faults and carry no `fix`. Nothing is
prepared or journaled.

1. **Path.** The configured path is walked one component at a time from `/`, opening each
   directory without following a symlink. Any symlink component refuses (`path`). On macOS the
   operator writes `/private/var/...`, not `/var/...`.
2. **Ancestors.** Every directory above the file is owned by root or the controller's effective
   UID, is not writable by group or others, and has no ACL entry granting write to another
   principal. This is spec 001 §4.1's ancestor rule with no sticky-bit exception (`ancestor`).
3. **Parent.** The file's own directory is also owned by the controller's effective UID
   (`parent`).
4. **The file.** It is opened with no-follow and non-blocking flags and checked with `fstat` on
   the open descriptor, so a FIFO refuses at once instead of hanging. It must be a regular file
   (`not_regular`), owned by the controller's effective UID (`owner`), with exactly one link
   (`links`), no write for group or others and no ACL entry granting write to another principal
   (`mode`), and at most 1,048,576 bytes (`size`).
5. **Outside every Mount source.** Neither the file nor any directory above it may be, or lie
   inside, a Mount source that a cell of this instance holds or has a pending operation for (from
   the journals), or one the operator file declares (`inside_mount_source`). Sources are compared
   by directory identity (device and inode), found by opening each recorded source path with the
   walk of rule 1. A held source that can no longer be opened refuses only the requests that
   resolve for the cell holding it (`inside_mount_source`); other cells are not blocked.
6. **Content.** The operator file is UTF-8 JSON with no duplicate keys and no unknown fields, and
   `version` is exactly 1 (`content`). A declaration is parsed with spec 011's grammar and keeps
   spec 011's refusals.

Rule 5 has two limits. It opens the source path recorded at attach, not the directory actually
mounted, so an operator who renames a held Mount root, or a directory above it, escapes rule 5
for that holding. And it cannot read the holdings of a quarantined cell, whose journal does not
parse.

### 4. The file's shape

```json
{
  "version": 1,
  "plasmids": {
    "github-pr": {
      "declaration": "/Users/operator/plasmids/github-pr/plasmid.toml",
      "proxy_maps": [
        {"host": "api.github.com", "port": 443, "modes": ["passthrough"], "route": "github-api",
         "transport": "tcp", "destination": "140.82.112.6", "allow_private": false},
        {"host": "api.github.com", "port": 443, "modes": ["simulate"], "route": "github-mock",
         "transport": "tcp", "destination": "127.0.0.1", "allow_private": true}
      ],
      "mounts": []
    },
    "workspace": {
      "declaration": "/Users/operator/plasmids/workspace/plasmid.toml",
      "proxy_maps": [],
      "mounts": [{"target": "/workspace", "source": "/Users/operator/src/project",
                  "access": "read_write"}]
    }
  }
}
```

- `plasmids` maps a plasmid ID to its entry. A key is a nonempty, NUL-free string equal to the
  `id` of the declaration it lists.
- `declaration` is optional: the absolute path of the plasmid's declaration file, for local
  selection (§5).
- `proxy_maps` and `mounts` are required arrays and may be empty.
- A ProxyMap entry has exactly `host`, `port`, `modes`, `route`, `transport`, `destination` and
  `allow_private`. `host` follows spec 001's proxy host rule (a lower-case DNS name). `modes` is a
  nonempty set over `simulate | capture | passthrough` with no repeats. `route` is a nonempty,
  NUL-free label. `transport`, `destination`, `port` and `allow_private` must pass spec 017's
  `ProxyRecipe::validate` as implemented. Among other things, that refuses port 0, an
  IPv4-mapped or IPv4-compatible IPv6 literal, a literal not in its one display spelling, and a
  DNS name whose last label is a number.
- A Mount entry has exactly `target` (an absolute, NUL-free guest path), `source` (an absolute,
  NUL-free host path) and `access` (`read_only | read_write`).
- Within one plasmid's entry, two ProxyMap entries with the same `host`, `port` and `transport`
  must not share a mode, and two Mount entries must not share a `target`. TCP and UDP entries on
  one port are separate.

Any violation refuses the file with 108 (`content`). Version 1 has no entries for Broker,
UdsSocket or SessionFile, no guest IP routes and no credential material; adding any of them is a
new version.

### 5. Local selection

**The named plasmid.** The controller looks up the requested ID in the operator file and opens
the listed `declaration` under §3. The manifest's `id` must equal the requested ID. The journal
records spec 008's `{"kind": "local"}` source.

- An ID with no entry, or an entry with no `declaration`: 101, `target` `plasmid <id>`. Nothing
  else is searched: not the working directory, not `.plasmosome/`, not the registry.
- A declaration that fails §3, or whose `id` differs: 108 with `path` the declaration.

**Its providers.** A local plasmid's `requires` names capability keys, not plasmid IDs. For each
key, in order:

1. If one or more plasmids already attached to the cell provide the key, it is satisfied. None of
   them is read or resolved again, and nothing joins the closure.
2. Otherwise the candidates are the listed plasmids whose declaration's `[provides]` binds the
   key. The search opens every listed declaration under §3. One that fails §3 or does not parse
   refuses the request with 108 naming it: the search fails closed. Exactly one candidate joins
   the closure and is resolved like the named plasmid; spec 001's version selection still applies
   to it. None is 103, `capability` the key, `plasmid` the requirer. More than one is 100 with
   their IDs.

A registry member (spec 020) keeps its imported declaration and its graph's provider bindings.
The operator file supplies only its ProxyMap and Mount entries, keyed by its plasmid ID.

**Genomes.** `cell.new` with a `genome` and no `artifact` has no local grammar to select from:
spec 020 defines genomes only as registry releases, and spec 001 §2's project-local path names no
grammar. Until one exists, it refuses with 101, `target` `genome <name>`, and creates no cell.

### 6. From declaration sections to capabilities

Each closure member is resolved on its own, against its own declaration and its own entry. A
requirer never gains its provider's capabilities, nor a provider its requirer's. Members already
attached are not resolved again.

**Declared values are used exactly.** A value the resolver cannot use exactly is the author's fault:
108 under spec 011's author-refusal rule, with the field, a nonempty `fix` and the plasmid ID. That
covers a port that is not an integer from 1 to 65535, a repeated port, hosts with no ports, a host
that is not a lower-case DNS name, a repeated host, a CIDR that does not parse or has host bits set,
a `[workspace]` with no `mount`, and a `dst` that is relative, has a trailing slash, contains `.`,
`..` or NUL, or is `/`. Port faults follow spec 011 as PR #133 amends it, naming the entry by its
index. A repeated port is field `network.ports[i]`, the repeat, with `fix` exactly `remove this
entry`. An entry that is not an integer from 1 to 65535 is field `network.ports[i]`, with `fix` the
port the entry evidently means when the list does not already declare it, and otherwise `remove this
entry`. Hosts with no ports is field `network.hosts[0]` with `fix` `remove this entry`, because no
port line exists that would not be a guess. So following a `fix` never drops a declared port,
repeats one, or adds one the author did not write. A parser that drops, wraps or truncates an entry
does not meet this.

#### Network becomes ProxyMap

**Which hosts, at which mode.** In `passthrough` every declared host is resolved. In `simulate`
and `capture` every declared host must be one that `[mock].hosts` stands in for, and it is
resolved at that mode. Otherwise the attach refuses with 108, field `mock.hosts`, a `fix` and the
plasmid ID. That includes a plasmid with `[network]` and no `[mock]` that is given or inherits a
mock mode. So attach stays all-or-nothing (spec 011), and a simulated plasmid never reaches a
live service through a host nothing simulates.

**Picking entries.** For each resolved host `h` and port `p` at mode `m`, the resolver picks
every entry with `host` = `h`, `port` = `p` and `m` in `modes`; §4 allows one per transport. Each
becomes one ProxyMap capability: `host` and `route` from the entry, and a ProxyRecipe of its
`transport`, `destination`, `port` and `allow_private`. With no entry, the request refuses with
103, `capability` `entry:network:<h>:<p>`, `plasmid` the member. An entry for another mode is
never used instead.

**Pins.** `pin_cidrs` constrains `passthrough` entries only. In `simulate` and `capture` an entry
leads to the operator's mock endpoint, so pins do not apply; that is what lets a pinned plasmid
be mocked. In `passthrough` with nonempty `pin_cidrs`, an entry is eligible only when its
`destination` is an IP literal inside one of the CIDRs of its own address family. A DNS-name
destination is not eligible, since its addresses are unknown until connection. With no eligible
entry, the result is the 103 above.

**One meaning per host and port in a cell.** The guest's network is per cell, and a new flow
selects among the cell's grants for its host. So the controller compares every new ProxyMap with
the others and with every ProxyMap the cell holds, leaving out the holdings this same reload
replaces. For one host, port and transport, all must come from the same mode and carry the same
recipe:

- different modes refuse with 104: `node` the plasmid with the other mode, `modes` both modes,
  `plasmids` both plasmids, `resolutions` `["remove_plasmid"]`;
- the same mode with different recipes refuses with 108, `path` the operator file, `detail`
  naming both plasmids: the operator has given one host two meanings.

So two attached plasmids that share a host cannot move it to a new destination by reload: each
reload meets the other's held recipe. The operator removes one, reloads the other, and re-adds.

Spec 001 §4.2 selects among a host's grants by host alone, so a host held on two ports, or over
TCP and UDP, always selects one grant and the other's flows refuse. That defect predates this
spec and is tracked in task plasmosome-e6m.

**A mode never changes after attach** (the default pending owner question 6). A mode now selects
which entries realize a plasmid's hosts, so changing it would change a grant. When D2b
propagation would give an attached plasmid a different mode, the attach refuses with 104: `node`
that plasmid, `modes` both, `plasmids` it and the new requirer, `resolutions`
`["remove_plasmid"]`. A reload `mock` that differs from the held mode gets the same 104. A detach
leaves every remaining plasmid at the mode it attached at. To change a mode, detach and attach.

#### Workspace becomes Mount

- `dst` defaults to `/workspace` when `mount` omits it. The resolver picks the one Mount entry
  with `target` = `dst`. It becomes one Mount capability with the entry's `source`, `target` and
  a MountRecipe of its `access`. With no entry: 103, `capability` `entry:workspace:<dst>`.
- `backend` selects no mechanism. Every Mount is realized as spec 017 and spec 001 §4.2 say,
  through the mediated guest filesystem. Spec 001 forbids a virtiofs export, and the grammar's
  default of `"virtiofs"` does not override that.

**The Mount source guard.** A Mount source must not expose the kernel's own state, nor let a cell
rewrite what decides what cells get. The resolver opens the source with the walk of §3 rule 1, so
a symlink anywhere in its path refuses. Every directory above it must meet §3 rule 2, and it must
be a directory. It refuses with 108 (`path` the operator file) when, compared by directory
identity, it is the same directory as, an ancestor of, or a descendant of any of these:

- the instance root, and the registry root if configured;
- the directories of the control socket, of the controller's configuration file, and of the
  running controller executable (its resolved real path);
- the directory of the operator file and of every declaration it lists;
- once spec 023 is accepted, the directory of every `cell_runtime` Artifact.

Not guarded: a mount point inside the source that reaches a guarded directory, and another view
of a guarded directory with a different identity, such as a FUSE or bindfs mount. Another
instance's root on the same host is out of scope: this instance does not know it.

#### Other sections

- `[commands]` with any command: 103, `capability` `unsupported:commands:<id>` for the first.
  Running a command as a subject needs spec 001's host-side attestation, which does not exist.
- `[secrets]` with any reference, or `[model]` (whose `credential` defaults to
  `model-provider/key`): 103, `capability` `unsupported:credential:<ref>`. Credential custody is
  reserved in spec 001, and spec 017 keeps credential material out of recipes. Spec 001's
  credential validation runs first and keeps its 108.
- These are not author errors, so they carry no `fix`. The `entry:` and `unsupported:` strings
  tell a client who can act.
- `[model].endpoint`, `wasm` and `[provides]` add no capability. A plasmid that must reach the
  model endpoint declares its host in `[network]`.

### 7. The output

Each member yields complete capabilities, each with a fresh `GrantId` chosen before prepare and
no reference back to the operator file. A member with no needs yields none and still attaches,
as spec 008's empty attachment. Refusals are checked for the whole closure before prepare; when
several apply, which one is returned is not part of the contract.

### 8. Reload

Nothing watches the file, and an attach never re-resolves an attached plasmid. `plasmid.reload`
re-resolves the plasmid at its held mode against the file as it is then. A local member re-reads
its declaration at the path the file lists now. A registry member keeps its exact imported
declaration (spec 001 §3.12); only its entries are re-read.

**A reload never widens.** It refuses with 109 `widening_forbidden`, `plasmid` the plasmid,
before prepare, when the re-resolved set has any of these against what the plasmid holds:

- a host, port and transport it did not hold;
- `allow_private` true where the held capability had false;
- a Mount target it did not hold, or a different Mount `source` at a held target;
- `read_write` where the held Mount was `read_only`;
- an entry at `passthrough` where the held one was at `simulate` or `capture`, should the owner
  let a reload change mode (question 6).

A changed route or destination, or a narrower set, proceeds with fresh replacement holdings.
Spec 008's reload preflight still applies. To widen, the operator removes the plasmid and adds it
again.

## Changes proposed to accepted specs

These are proposed text, not edits made in this PR. Spec 001 says its text changes in a pull
request with the reasoning written down, so the PR that accepts this spec must carry them, or
this spec stays draft. The mock-mode amendments to 011 and 001 assume the default of owner
question 6.

**Spec 011.**

- `011:165-169`. Replace "It does change one thing about them: a mock mode propagates across the
  dependency closure exactly as spec 001 §3.10 froze it. A mode decides whether a granted call
  reaches a live service or a recording; it is not itself a grant, and that propagation is
  untouched here." with "Nor does it change their mock modes: under spec 022 a mode selects
  which operator entries realize a plasmid's hosts, so an attach whose propagation would change
  an attached plasmid's mode is refused with 104."
- `011:183-185`. Replace "A mode the detached plasmid had declared stops propagating with it;
  what the remaining declarations propagate is governed by spec 001 §3.10, unchanged." with "A
  detach changes no remaining plasmid's mock mode: each keeps the mode it attached at (spec
  022)."
- `011:267-272`. After "attach brings the whole closure with it.", add "Since spec 022, the
  closure bounds which needs a plasmid has; the trusted operator declarations file bounds where
  each one leads on the host, so a reviewer reads both."
- `011:297-298`. Replace "The declaration is the only thing the kernel reads to decide what the
  plasmid may reach." with:

  > The declaration is the only thing the kernel reads to decide what the plasmid may reach:
  > which hosts and ports, which workspace path, which tools. Spec 022's trusted operator
  > declarations decide only where each declared need leads on this host (the real destination,
  > the host directory and its access), and can leave a need unmet, never add one.

- `011:345-347`. Replace "It changes no already-attached plasmid's grants; mock-mode propagation
  across the dependency closure is unchanged from spec 001 §3.10." with "It changes no
  already-attached plasmid's grants or mock mode; propagation that would change an attached
  plasmid's mode refuses with 104 (spec 022)."
- `011:374-376`. Replace "the declarations of a plasmid and its required closure are sufficient
  for a reviewer to bound its reach." with "the declarations of a plasmid and its required
  closure, read with spec 022's operator declarations file, are sufficient for a reviewer to
  bound its reach."
- `011:418-420`. Replace "a mock mode propagating across the closure in the same attach is
  asserted separately and is not counted as a change of grant." with "an attach whose mock-mode
  propagation would change an attached plasmid's mode is refused with 104 and changes nothing
  (spec 022)."
- `011:476-478`. Replace "are frozen in spec 001 §3.10 and untouched. The only thing said about
  them here is that propagation is not a change of grant." with "are in spec 001 §3.10, as spec
  022 amends it: propagation never changes an attached plasmid's mode."

**Spec 001.**

- §1, the 104 row (`001:99`). Replace "per D2b rule 3" with "per D2b rule 3, or per spec 022 when
  a mutation would change an attached plasmid's mode or serve one host, port and transport at two
  modes in a cell; spec 022's cases name one plasmid as `node` and offer only
  `remove_plasmid`".
- §2 (`001:139-141`). After "`.plasmosome/genomes/<name>.toml`; the share/export form is
  `*.genome.toml`.", add "That file is not yet a selection source: until a local genome grammar
  exists, `cell.new` selects a genome only by `artifact` (spec 022)."
- §3.5. Replace the example request and result with this text, which spec 023 proposes too
  (whichever lands second finds it applied):

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
    "plasmids": ["github-pr [mock:simulate]", "workspace [real]"]}}
  ```

  Replace "An absent artifact preserves local selection." with "An absent artifact with a named
  genome refuses with 101 until a local genome grammar exists (spec 022)." Replace "(genome table
  → `plasmid add --mock` → `plasmid reload --mock`)" with "(genome table → `plasmid add --mock`;
  a reload keeps the held mode, spec 022)". After "is the documented alias of `cell.new --genome
  <name>` (D1c).", add "Until a local genome grammar exists, it also needs the genome's
  `artifact` (spec 022)."
- §3.9. In the example, replace `"plasmid": "model-provider"` with `"plasmid": "workspace"` and
  `"label": "model-provider [real]"` with `"label": "workspace [real]"`. Under spec 022 a
  `[model]` plasmid cannot attach until credential custody exists.
- §3.10, propagation. After "Inherited levels yield to the new explicit declaration (D2b rule
  2).", add "They never yield on a plasmid already attached: an attach that would change an
  attached plasmid's mode refuses with 104 naming it, with `resolutions: ["remove_plasmid"]`
  (spec 022)."
- §3.10, credentials (`001:423-428`). After "never a silent downgrade.", add "Until credential
  custody exists, a reference that passes this validation still refuses the attach with 103
  `unsupported:credential:<ref>` (spec 022)."
- §3.10, artifact. Replace "Without artifact, existing local selection remains unchanged." with:

  > Without artifact, the plasmid and its local providers are selected through spec 022's
  > operator declarations file: an unlisted ID is 101, and an unsafe or mismatched declaration
  > is 108. Every closure member's declared network and workspace needs are resolved into
  > complete capabilities through that file before prepare; an uncovered need is 103 naming the
  > capability and plasmid.

- §3.12. Replace "mock mode may be changed in the same swap (D2's third layer)." with "the swap
  keeps the plasmid's mock mode, and a `mock` that differs from it refuses with 104 (spec 022)."
  In the example, remove `, "mock": "simulate"` from the request and change the result's
  `"mock": "simulate"` to `"mock": "capture"`. Replace "Mock overrides and ordinary
  generation-swap checks are unchanged." with:

  > Ordinary generation-swap checks are unchanged. Reload re-resolves the plasmid's capabilities
  > at its held mode against the operator declarations file as it is at reload time; a registry
  > member keeps its exact imported declaration. A reload whose new capabilities would widen
  > what the plasmid holds refuses with 109 before prepare (spec 022).

- §4.1. After the paragraph that introduces `registry_root`, add the paragraph below. Spec 023
  inserts after the same paragraph; whichever lands second goes after the other's.

  > The same strict JSON configuration also admits optional `operator_declarations`, an
  > explicitly supplied absolute path to spec 022's operator declarations file. Nothing supplies
  > a default; a relative or non-string value is invalid configuration. Only the resolving
  > mutations of spec 022 read it. Startup, recovery, status and every withdrawal never open it,
  > and its absence or corruption never blocks them.

**Spec 017** (`017:119-120`). Replace "this is not a new user-editable manifest grammar." with:

> this is not a new manifest grammar for plasmid authors. Spec 022 defines the operator
> declarations file this resolver reads, and the resolution of ProxyMap and Mount; Broker,
> UdsSocket and SessionFile have no declaration source yet. The Mount adapter opens a recorded
> source root itself, walking each component from `/` without following a symlink, so a symlink
> swapped in after resolution refuses at apply.

**Spec 020** (`020:431-432`). Replace "Requests without `artifact` retain existing local
selection semantics." with:

> Requests without `artifact` use spec 022's local selection: a plasmid through the operator
> declarations file, and no genome until a local genome grammar exists.

## Open questions

These need the owner. Where this spec needs an answer to work, it sets a conservative default.

1. **Broker, UdsSocket and SessionFile.** No declaration section selects them. Should a new
   section declare them, or should they stay unreachable through public verbs for now (the
   default)? Task plasmosome-018's closing scenario wants all five classes attached through
   public verbs, so it waits on this.
2. **Credentials in the meantime.** `[secrets]` and `[model]` refuse with 103 until custody
   exists (the default). That makes a model-provider plasmid unattachable. Is that right?
3. **Mock servers.** A mocked host is routed to a mock server the operator runs. Is the operator
   the right owner of that server, or should `[mock].backend` select something the kernel runs?
4. **Local genomes.** Where may a local genome file live, and in what grammar? The project
   directory is writable by the agent, so a genome found there cannot be trusted as declared.
5. **The operator file as a gate.** Every new network or workspace need waits for an operator
   entry, so the file gates reach in practice. It also decides a Mount's access, which a
   declaration cannot express: a plasmid that needs only reads cannot say so. Intent 010 leaves
   the gate's shape to the owner. Is this acceptable as part of the gate, or must the gate and
   the access choice sit elsewhere?
6. **Mode changes after attach.** By default an attached plasmid's mode is fixed: an attach or
   reload that would change it refuses with 104, and a detach leaves it alone (§6). That reverses
   D2b rule 2 for attached plasmids and removes D2's third layer, `plasmid reload --mock`, both
   decided items. The alternative re-resolves the plasmid at the new mode under §8, where a move
   to `passthrough` is a widening. Which does the owner want?
7. **Widening reloads.** A reload that would widen refuses with 109, and the operator removes and
   re-adds the plasmid (the default). Should a widening reload pass through the approval gate
   instead?
8. **Pins and DNS.** Under `pin_cidrs`, a passthrough entry must use an IP literal (the default),
   which gives up DNS failover. Should a DNS destination be allowed, with every resolved address
   checked against the pins at connection? That needs a ProxyRecipe field.

## Acceptance

Each item names the broken implementation it catches. Items 6–9 and 17 need real files on
Darwin; the rest run against the resolver and journal. Item 8's `owner` case needs a second UID
(owner gate O-8), and item 16's first case a registry import (spec 020). No item needs a real
ProxyMap (O-11) or Mount (O-7).

1. Hosts `a.example` and `b.example` on ports 443 and 8443, fully covered, one entry `udp` and one
   with `allow_private: true`: prepare holds four ProxyMaps with their entries' exact fields and
   four distinct fresh IDs. Remove one entry and attach to a fresh cell: 103
   `entry:network:<h>:<p>` naming that pair and the plasmid, and no journal record. Catches:
   dropping a pair, defaulting a field, partial attach, and naming the wrong pair.
2. TCP and UDP entries for one host, port and mode give two capabilities. Entries for one host,
   port and transport with modes `["simulate","capture"]` and `["capture"]`, or a mode repeated in
   one `modes`, refuse the file with 108. Catches: refusing QUIC, and missing a partial overlap.
3. `[mock].hosts` covers the host, with `passthrough` and `simulate` entries of different
   destinations. No mock grants the passthrough one, `--mock simulate` the simulate one, and
   `--mock capture` refuses with 103. A provider that inherits `simulate` gets its simulate entry.
   Catches: picking an entry whatever its mode, and falling back to another mode.
4. Providers. R requires `k` and listed P provides it, each with entries under its own ID: R's
   operations name only R's hosts, P's only P's; with P's entry moved under R's ID, 103 naming P.
   Two listed providers of `k`: 100. None: 103 `k`. An unrelated group-writable listing: 108
   naming it. Two attached providers whose files are then deleted: R attaches and their holdings
   are unchanged. Catches: borrowing another member's entries, an undefined candidate set,
   skipping an unsafe listing, and re-reading an attached provider.
5. Local selection. An unlisted ID is 101 `plasmid <id>`, even with a valid declaration at the
   project root and under `.plasmosome/plasmids/`; so is an entry with no `declaration`. An unsafe
   or mismatched declaration is 108 with `path` the declaration. A listed, matching one attaches
   with source `{"kind": "local"}`. `cell.new` with a genome and no artifact is 101 `genome
   <name>`, with no cell directory, even with `.plasmosome/genomes/<name>.toml` present. Catches:
   searching any default path.
6. Attach, change the entry's `destination`, restart. Recovery holds the original destination.
   Reload: the new generation holds the new one with a fresh ID. With a second plasmid holding
   that host at the old recipe, the reload is 108. Catches: recovery that reads the operator
   file, stale recipes on reload, and a collision check that counts the holdings being replaced.
7. Reload widening: `allow_private` false to true, a Mount `read_only` to `read_write`, a changed
   Mount `source`, and a host added to a local declaration are each 109 with nothing prepared.
   Catches: a reload that silently grants more.
8. Trust checks, each a 108 whose `detail` names the check, with no journal record:
   - a group-writable file, and a 0600 file with an `everyone allow write` ACL (`mode`);
   - a second hard link (`links`);
   - a final-component symlink, and a symlinked ancestor, to valid targets (`path`);
   - a group-writable parent (`parent` or `ancestor`);
   - a FIFO, refused within one second (`not_regular`);
   - over 1 MiB, non-UTF-8, a duplicate key, an unknown field, `version: 2` (`content`);
   - `/private/etc/hosts` (`parent`), and with O-8 a second UID's file in a directory the
     controller's UID owns (`owner`).

   Then delete the file and restart: startup and recovery succeed. Catches: trusting the path
   text, checking only the file's mode, following links, blocking on a FIFO, and reporting every
   fault as a parse error.
9. Cell A holds a `read_write` Mount of S. The operator file inside S, a listed declaration inside
   S, and S declared but not yet held each give 108 `inside_mount_source`; outside S, requests
   succeed. Rename S away: a request for cell A is 108 `inside_mount_source`, and one for cell B
   succeeds. Catches: trusting a file the workload can write, and one stale holding blocking the
   whole instance.
10. Author refusals, each 108 with the field, a nonempty `fix` and the plasmid ID: ports `70000`,
    `-1`, `0`, `65536`, `"443"` and `443.5`; a repeated port or host; hosts with no ports; an
    upper-case or IP-literal host; a malformed CIDR; `192.0.2.1/24`; `[workspace]` with no `mount`;
    `dst = "workspace"`. Each port fault names `network.ports[i]`; `["443"]` gives `fix` `443`, a
    repeat gives `remove this entry`, and hosts with `ports = []` give `network.hosts[0]` and
    `remove this entry`. Following the fixes in turn from `[65536]` with one host ends with no host
    and no port, never a port the author did not write. Catches: a truncating parser, a missing or
    whole-line `fix`, and a `fix` that invents a port.
11. Pins, `pin_cidrs = ["192.0.2.0/25"]`. `192.0.2.10` attaches. `192.0.2.200`,
    `192.0.2.10.example.com` and `2001:db8::1` are 103. `::192.0.2.10`, `::ffff:192.0.2.10` and
    `192.000.002.010` refuse the file with 108, as `ProxyRecipe::validate` does. At `--mock
    simulate` with a `127.0.0.1` entry it attaches. The in-tree `github-pr` fixture attaches in
    both modes against §4's example. Catches: a prefix check, crossing address families, skipping
    `validate`, and pins that make a plasmid unmockable.
12. X and Y both declare `api.github.com:443`. X at `simulate`, then Y at `passthrough`: 104,
    `node` X, `resolutions` `["remove_plasmid"]`. Both at `passthrough` with different
    destinations: 108; with equal recipes, both attach. Catches: letting ID order choose between a
    mock and a live service.
13. P attached at `passthrough`; R added with `--mock capture`, which would propagate to P: 104
    naming P, P unchanged. Reloading P with `mock: "simulate"`: the same 104. R1 at `capture`
    brings P at `capture`; R2 with no mode also uses P; removing R1 leaves P at `capture`, its
    holdings unchanged. Catches: re-resolving an attached provider, and a mode its grants lack.
14. Hosts `a` and `b`, `[mock].hosts = ["a"]`, at `--mock simulate`: 108 `mock.hosts`, nothing
    prepared. With both mocked and covered, it attaches. A `[network]` plasmid with no `[mock]` is
    108 when added at `simulate` and when a provider would inherit `simulate`; at passthrough it
    attaches. Catches: a partial grant reported active, and an unmocked host reaching a live
    service.
15. `[commands]` is 103 `unsupported:commands:<id>`, `[secrets]` `unsupported:credential:<ref>`,
    and `[model]` with no `credential` key `unsupported:credential:model-provider/key`, none with
    a `fix`. Catches: a wrong string, or refusing `[model]` only when `credential` is written.
16. With `operator_declarations` unset and a covering file at the instance root, `~/.plasmosome`,
    the registry root and the working directory: a registry plasmid with no needs attaches, a
    local `plasmid.add` is 101, and a registry plasmid with `[network]` is 103. Catches: a hidden
    default file.
17. Mount sources equal to, inside and above each guarded directory (instance root, registry root,
    control-socket, configuration, controller executable, operator file and declaration
    directories, and once spec 023 is accepted the `membraned` artifact's) each refuse with 108
    before prepare. A sibling `<instance-root-name>-x` attaches. On a case-insensitive volume, a
    source spelled in a different case from the instance root refuses, and so does a source with a
    symlink component. Catches: a string-prefix check, a case-sensitive comparison, and a
    following walk.

## Out of scope

- How ProxyMap and Mount are enforced. That is spec 017 and spec 001 §4.2.
- Selecting a proxy grant by port and transport, which is task plasmosome-e6m.
- The approval gate for a declaration. That is open in spec 011 and intent 010.
- Credential custody, command subjects and attestation.
- A local genome grammar.
- Watching the operator file, or pushing its changes into running cells.
- Version selection between providers, which spec 001 already does.
