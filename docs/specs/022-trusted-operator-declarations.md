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
backs a mount, with what access. The work is split in two. **The declaration decides what is
reachable**: which hosts and ports, and which workspace path. **The trusted operator decides
where each of those leads on this host**: the real destination, the host directory, and the
access. The operator writes those facts in one file, the **operator declarations file**. The
controller combines the two into the exact capabilities a plasmid holds. A need the declaration
does not name gets nothing, whatever the file says; a need the file does not cover is refused.

Resolution runs once per mutation that attaches plasmids (`plasmid.add`, `plasmid.reload`, and
`cell.new` with a genome), before anything is prepared. Every declared host and port, and every
declared workspace, must be covered by an operator entry for the plasmid's resolved mock mode.
If one is not, the request refuses with code 103 and nothing is prepared. The output is a fixed
list of complete spec 017 capabilities, each with a fresh identity; spec 017 and spec 008 take it
from there. Recovery never reads the operator file. It replays the recipes the journal recorded,
so editing the file never changes a capability a cell already holds. A change reaches a cell only
through an explicit add, or a reload that does not widen what the plasmid holds.

The controller and its cells run as one host UID, so a file's owner says nothing about who wrote
it: a workload writes through a read-write Mount as that same UID. The operator file and every
declaration it lists are therefore trusted only when no cell can write them, no other principal
can replace them, and they sit outside every Mount source. This exists for three intents. Intent
004 needs every capability complete before it is granted, so that removing it removes exactly
that one. Intent 012 needs a grant to come from a declared need, not from whatever the host
allows. Intent 003 needs recovery to work from the journal alone, with no lookup table that may
have changed since. This spec covers the two classes a declaration can express today, ProxyMap
and Mount. Broker, UdsSocket and SessionFile have no declaration source yet, and credentials stay
refused; both are owner questions below.

**Platform.** The product is macOS first: Darwin arm64 is the only runtime host, and the cell is
a Linux guest run by libkrun. Linux/KVM host support is deferred, not dropped. The resolver is
portable logic plus file checks, defined for both host platforms. Evidence for each acceptance
item says which level it reached: the model (fake and composite backends), the Darwin OS
mechanism, the Darwin pinned runtime, or a Linux host, which is deferred. A real ProxyMap also
needs owner decision O-11, and a real Mount needs O-7.

## Contract

### 1. Where the file comes from

The controller's strict JSON configuration gains an optional key, `operator_declarations`: an
absolute, NUL-free path string. Nothing supplies a default. It is not inferred from
`instance_root`, `registry_root`, the control socket, the caller's HOME, the working directory or
a request field. A relative or non-string value is invalid configuration, refused at startup.

When the key is absent, the controller behaves as if the file existed and listed no plasmid:

- a plasmid selected by `artifact` (spec 020) that declares no network and no workspace attaches;
- any `plasmid.add` without `artifact` is 101, because local selection needs a listing (section 5);
- any declared network or workspace need is 103, because nothing covers it.

### 2. When it is read, and what never reads it

The file is read once per resolving request, at the start of resolution. One read serves every
member of that request's closure, so all of them see the same contents. Each later request reads
it again.

Startup, recovery, `plasmosome.status`, `plasmosome.recovery`, `cell.list`, `cell.status`,
`plasmid.list`, `plasmid.remove`, `cell.kill` and every withdrawal never open it. A missing,
unsafe or corrupt file cannot block startup, recovery or removal.

### 3. Trust checks

The same checks apply to the operator file and to every declaration file it lists. A file that
fails any of them refuses the request with code 108, `path` the file's absolute path, and
`detail` naming the failed check (one of `path`, `ancestor`, `parent`, `not_regular`, `owner`,
`links`, `mode`, `size`, `inside_mount_source`, `content`). These are operator faults, so the
refusal carries no `fix`. Nothing is prepared and nothing is journaled.

1. **Path.** The configured path is walked one component at a time from `/`, opening each
   directory without following a symlink. Any symlink component refuses (`path`). The operator
   writes the path without symlinks; on macOS that means `/private/var/...`, not `/var/...`.
2. **Ancestors.** Every directory above the file is owned by root or the controller's effective
   UID, is not writable by group or others, and has no ACL entry granting write to another
   principal. This is spec 001 §4.1's ancestor rule, with no sticky-bit exception (`ancestor`).
3. **Parent.** The file's own directory is owned by the controller's effective UID, besides
   meeting rule 2 (`parent`).
4. **The file.** It is opened with no-follow and non-blocking flags, then checked with `fstat` on
   the open descriptor, so a FIFO is refused at once instead of hanging the request. It must be
   a regular file (`not_regular`), owned by the controller's effective UID (`owner`), with exactly
   one link (`links`), a mode that grants no write to group or others and no ACL entry granting
   write to another principal (`mode`), and at most 1,048,576 bytes (`size`).
5. **Outside every Mount source.** Neither the file nor any directory above it may be the same
   directory as, or lie inside, any Mount source that a cell of this instance holds or has a
   pending operation for (from the journals), or any Mount source the operator file declares
   for any plasmid (`inside_mount_source`). Sources are compared by directory identity (device
   and inode), found by opening each recorded source path with the walk of rule 1. A recorded
   source that can no longer be opened also refuses, because the controller can no longer prove
   the file is outside it; the operator removes that holding first.
6. **Content.** The operator file is UTF-8 JSON with no duplicate keys and no unknown fields,
   and `version` is exactly 1 (`content`). A declaration is parsed with spec 011's grammar, and its
   grammar faults keep spec 011's own refusals.

### 4. The file's shape

```json
{
  "version": 1,
  "plasmids": {
    "github-pr": {
      "declaration": "/Users/operator/plasmids/github-pr/plasmid.toml",
      "proxy_maps": [
        {"host": "api.github.com", "port": 443, "modes": ["passthrough"],
         "route": "github-api", "transport": "tcp",
         "destination": "140.82.112.6", "allow_private": false},
        {"host": "api.github.com", "port": 443, "modes": ["simulate"],
         "route": "github-mock", "transport": "tcp",
         "destination": "127.0.0.1", "allow_private": true}
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

- `plasmids` maps a plasmid ID to its entry. A key is a nonempty, NUL-free string, and it must
  equal the `id` of the declaration it lists.
- `declaration` is optional: the absolute path of the plasmid's declaration file, used for local
  selection (section 5).
- `proxy_maps` and `mounts` are required arrays and may be empty.
- A ProxyMap entry has exactly `host`, `port`, `modes`, `route`, `transport`, `destination` and
  `allow_private`. `host` follows spec 001's proxy host rule (a lower-case DNS name, never a
  numeric address). `port` is 1 to 65535. `modes` is a nonempty set over `simulate | capture |
  passthrough` with no repeats. `route`, `transport`, `destination` and `allow_private` follow
  spec 017's ProxyRecipe rules; `route` is a nonempty, NUL-free label. A numeric `destination`
  must be written in canonical form (the form the standard library prints for that address).
- A Mount entry has exactly `target` (an absolute, NUL-free guest path), `source` (an absolute,
  NUL-free host path) and `access` (`read_only | read_write`).
- Within one plasmid's entry, two ProxyMap entries with the same `host`, `port` and `transport`
  must not share any mode, and two Mount entries must not share a `target`. TCP and UDP entries
  on one port are separate and may both exist.

Any violation refuses the file with 108 (`content`). Version 1 has no entries for Broker,
UdsSocket or SessionFile, no guest IP routes, and no credential material. Adding any of them is a
new version.

### 5. Local selection, and how a local closure is found

Spec 001 §3.10 says a `plasmid.add` without `artifact` "preserves local selection" but never says
what that selects. This spec defines it.

**The named plasmid.** The controller looks up the requested ID in the operator file and opens
the `declaration` listed there, under section 3. It parses the manifest and requires its `id` to
equal the requested ID. The journal records spec 008's `{"kind": "local"}` source.

- An ID with no entry, or an entry with no `declaration`: 101, `target` `plasmid <id>`. No other
  place is searched: not the working directory, not `.plasmosome/`, not the registry.
- A declaration that fails section 3: 108 with `path` the declaration.
- A manifest whose `id` differs from the requested ID: 108 with `path` the declaration.

**Its providers.** A local plasmid's `requires` names capability keys, not plasmid IDs. For each
key, in order:

1. A plasmid already attached to the cell that provides the key satisfies it. That provider is
   not read again and not resolved again: it keeps its recorded capabilities.
2. Otherwise the candidates are the plasmids the operator file lists with a `declaration` whose
   `[provides]` binds that key. Exactly one candidate joins the closure and is selected and
   resolved like the named plasmid. No candidate is 103 with `capability` the key and `plasmid`
   the requirer. More than one is 100 with `candidates` their IDs.

Spec 001's existing version selection still applies to the one candidate. The search opens every
listed declaration it needs, under section 3. A registry member (spec 020) keeps its imported
declaration and its graph's provider bindings; the operator file then supplies only its ProxyMap
and Mount entries, keyed by the member's plasmid ID.

**Genomes.** `cell.new` with a `genome` name and no `artifact` has no local grammar to select
from: spec 020 defines genomes only as registry releases, and spec 001 §2's project-local path
names no grammar. Until a local genome grammar exists, it refuses with 101, `target`
`genome <name>`, and creates no cell.

Listing a plasmid in this file is not spec 011's approval gate. But every new network or
workspace need now requires an operator entry before it can attach, so in practice the file also
gates reach. Where the approval gate sits is still the owner's question.

### 6. From declaration sections to capabilities

Each member of the closure is resolved on its own, against its own declaration and its own entry
in the operator file. A requirer never gains its provider's capabilities, and a provider never
gains its requirer's. Members already attached to the cell are not resolved again.

#### Network becomes ProxyMap

First, the declared values must be valid. Each fault below is the author's, so it is 108 under
spec 011's author-refusal rule: the field, a nonempty `fix`, and the plasmid ID.

| Fault | Field | `fix` |
| --- | --- | --- |
| `hosts` nonempty and `ports` empty | `network.ports` | `ports = [443]` |
| a port that is not an integer from 1 to 65535 (for example `-1`, `0`, `65536`, `"443"`, `443.5`) | `network.ports` | `ports = [443]` |
| a repeated port | `network.ports` | the line with the repeat removed |
| a host that is not a lower-case DNS name | `network.hosts` | the lower-case form when that is valid, else `hosts = ["api.example.com"]` |
| a repeated host | `network.hosts` | the line with the repeat removed |
| a CIDR that does not parse | `network.pin_cidrs` | `pin_cidrs = ["192.0.2.0/24"]` |
| a CIDR with host bits set | `network.pin_cidrs` | the CIDR with its host bits cleared |

The resolver uses the declared values exactly. A parser that drops or truncates entries does not
meet this; today's `manifest.rs` turns 70000 into 4464, 65536 into 0 and -1 into 65535.

**Which hosts, at which mode.** In `passthrough`, every declared host is resolved. In `simulate`
and `capture`, only the hosts in `[mock].hosts` are resolved, at that mode; a declared host that
the mock does not stand in for gets no capability in those modes. It is unreachable, and reaching
for it is reported as a denial under spec 011. This keeps a simulated plasmid from touching a live
service through a host nothing simulates.

**Picking entries.** For each resolved host `h` and each port `p`, at mode `m`, the resolver
picks every entry with `host` = `h`, `port` = `p` and `m` in `modes`. Section 4 allows at most one
per transport. Each picked entry becomes one ProxyMap capability: `host` and `route` from the
entry, and a ProxyRecipe of the entry's `transport`, `destination`, `port` and `allow_private`.
With no entry, the request refuses with 103, `capability` `entry:network:<h>:<p>`, `plasmid` the
member's ID. An entry for another mode is never used instead.

**Pins.** `pin_cidrs` constrains `passthrough` entries only. In `simulate` and `capture` the entry
leads to the operator's mock endpoint, not to the provider, so pins do not apply; this is what
lets a pinned plasmid be mocked. In `passthrough`, when `pin_cidrs` is nonempty, an entry is
eligible only when its `destination` is a numeric address inside one of the CIDRs. The check uses
the parsed address in its own family: an IPv6 address, including an IPv4-mapped (`::ffff:a.b.c.d`)
or IPv4-compatible (`::a.b.c.d`) form, is compared only with IPv6 CIDRs and is never converted to
IPv4. A DNS-name destination is not eligible, because its addresses are not known until
connection. If no entry is eligible, the result is the 103 above.

**One cell, one meaning per host and port.** The guest's network is per cell, and a new flow
selects among the cell's grants for its host. So after resolving the closure, the controller
compares every new ProxyMap with every ProxyMap the cell already holds and with each other. For
one host, port and transport, all holdings must come from the same mode and carry the same
recipe:

- different modes refuse with 104, `node` `network:<h>:<p>/<transport>`, `modes` both modes,
  `plasmids` both plasmids, and `resolutions` `["remove_plasmid"]`, plus `force_simulate` or
  `force_passthrough` when that names the mode the cell already holds;
- the same mode but different recipes refuse with 108, `path` the operator file, `detail`
  naming both plasmids. The operator has given one host two meanings.

**A mode never changes after attach.** When the new closure's D2b propagation would give an
attached provider a mode different from the one it attached at, the attach refuses with 104,
`node` that provider, `modes` both modes, `plasmids` the provider and the new requirer, and
`resolutions` `["remove_plasmid"]`. The provider is not resolved again. Changing an attached
plasmid's mode needs a detach and a new attach, so attach still changes no attached plasmid's
grants (spec 011).

#### Workspace becomes Mount

- A `[workspace]` table with no `mount` is 108, field `workspace.mount`, `fix`
  `mount = { dst = "/workspace" }`. Today's parser silently drops such a table.
- `dst` defaults to `/workspace` when `mount` omits it. A `dst` that is relative, has a trailing
  slash, contains `.` or `..` components or NUL, or is `/`, is 108, field `workspace.mount.dst`,
  `fix` `mount = { dst = "/workspace" }`.
- The resolver picks the one Mount entry with `target` = `dst`. It becomes one Mount capability
  with the entry's `source`, `target` and a MountRecipe of its `access`. With no entry: 103,
  `capability` `entry:workspace:<dst>`.
- `backend` selects no mechanism. Every Mount is realized as spec 017 and spec 001 §4.2 say,
  through the mediated guest filesystem. Spec 001 forbids a virtiofs export, and the grammar's
  default of `"virtiofs"` does not override that.

**The Mount source guard.** A Mount source must not expose the kernel's own state, and must not
let a cell rewrite the files that decide what cells get. The resolver opens the source with the
walk of section 3 rule 1, so a symlink anywhere in its path refuses. Every directory above the
source must meet section 3 rule 2. The source must be a directory. It is then compared by
directory identity with each of these, and refuses with 108 (`path` the operator file) when it is
the same directory, an ancestor, or a descendant of any of them:

- the instance root and the registry root, if configured;
- the directory of the control socket;
- the directory of the controller's configuration file;
- the directory of the operator file, and the directory of every declaration it lists;
- the directory of every `cell_runtime` Artifact (spec 023): `membraned`, the helper, its
  libraries, the kernel, the initramfs, the root image, and the host and guest policies.

Spec 017's Mount adapter opens the source again at apply with the same no-follow walk, so a
symlink swapped in after resolution is refused there. Not detected: a mount point inside the
source that reaches a guarded directory, and another view of a guarded directory with a
different identity, such as a FUSE or bindfs mount. Linux bind mounts keep the same identity
and are caught.

#### Other sections

- `[commands]` with any command: 103, `capability` `unsupported:commands:<id>` for the first
  command. Running a command as a subject needs spec 001's host-side attestation, which does not
  exist yet (code 110 on `cell.exec`).
- `[secrets]` with any reference, or `[model]` (whose `credential` always has a value, defaulting
  to `model-provider/key`): 103, `capability` `unsupported:credential:<ref>`. Credential custody is
  reserved in spec 001, and spec 017 keeps credential material out of recipes. Spec 001's existing
  credential validation still runs first and keeps its 108.
- These three are not author errors: the declaration is valid, and the kernel cannot yet grant
  it. They carry no `fix`.
- `[model].endpoint` adds no capability of its own. A plasmid that must reach it declares its host
  in `[network]`.
- `wasm` and `[provides]` add no capability under this spec. The plasmid interface is reserved by
  spec 001 §5.

The prefixes tell a client who can act on a 103: `entry:` means the operator must add an entry,
`unsupported:` means the kernel cannot grant that section yet, and a bare key means a provider is
missing, as before. A `requires` key or `[provides]` binding that begins with `entry:` or
`unsupported:` is 108 under spec 011's rule, with a `fix` naming the key without the prefix.

### 7. The output

Resolution of one member yields an ordered list of complete capabilities:

1. ProxyMaps, in `hosts` order, then `ports` order, then `tcp` before `udp`;
2. then the Mount, if any.

Each capability gets a fresh `GrantId` from spec 017's issuance rule, chosen before prepare. The
list carries no reference back to the operator file: the recipe is complete, and recovery needs
nothing else. A member with no network and no workspace yields an empty list and still attaches,
as spec 008's empty attachment.

Refusals are checked for the whole closure before anything is prepared. When several apply, one
is returned; which one is not part of the contract.

### 8. After attach, and reload

Changing or deleting the operator file changes no capability a cell holds. Nothing watches the
file, and an attach never re-resolves a plasmid that is already attached.

`plasmid.reload` re-resolves the plasmid against the file as it is at reload time. A local member
re-reads its declaration at the path the file lists now, as `{"kind": "local"}` allows. A registry
member keeps its exact imported declaration, as spec 001 §3.12 requires; only its entries are
re-read.

**A reload never widens.** The controller compares the re-resolved capabilities with the ones the
plasmid holds. The reload refuses with 109 `widening_forbidden`, `plasmid` the plasmid, before
prepare, when the new set has any of these:

- a host, port and transport the plasmid did not hold;
- `allow_private` true where the held capability for that host, port and transport had false;
- a Mount target the plasmid did not hold;
- `read_write` where the held Mount at that target was `read_only`.

A changed route or destination, or a narrower set, is not a widening and proceeds with fresh
replacement holdings. Spec 008's reload preflight is unchanged and still applies. To widen, the
operator removes the plasmid and adds it again.

### 9. Refusals, in one place

| Case | Code | Fields |
| --- | --- | --- |
| Operator file or declaration fails a trust check | 108 | `path` = file, `detail` = check |
| Operator file content invalid or ambiguous | 108 | `path` = file, `detail` = `content` |
| Two plasmids give one host, port and transport different recipes in one mode | 108 | `path` = operator file, `detail` |
| Mount source exposes kernel state or is not a directory | 108 | `path` = operator file, `detail` |
| Local ID not listed, or listed with no declaration | 101 | `target` = `plasmid <id>` |
| Local genome without `artifact` | 101 | `target` = `genome <name>` |
| Declaration `id` differs from the requested ID | 108 | `path` = declaration, `detail` |
| Author fault in `[network]`, `[workspace]` or a key prefix | 108 | field, `fix`, plasmid |
| Need not covered in this mode | 103 | `capability` = `entry:...`, `plasmid` |
| No provider for a `requires` key | 103 | `capability` = the key, `plasmid` |
| More than one listed provider for a key | 100 | `candidates` |
| Commands, secrets or model declared | 103 | `capability` = `unsupported:...`, `plasmid` |
| Two modes for one host and port in a cell, or a mode change on an attached provider | 104 | `node`, `modes`, `plasmids`, `resolutions` |
| Reload would widen | 109 | `plasmid` |

No new code is added. Every refusal happens before prepare and leaves no journal record.

## Changes proposed to accepted specs

These are proposed text, not edits made in this PR. `docs/specs/README.md` does not say that the
accepting PR applies them, and spec 001 says its text changes in a pull request with the
reasoning written down. So the PR that accepts this spec must carry these edits, or this spec
stays draft.

**Spec 011, Contract (`011:297-298`).** Replace "The declaration is the only thing the kernel
reads to decide what the plasmid may reach." with:

> The declaration is the only thing the kernel reads to decide what the plasmid may reach: which
> hosts and ports, which workspace path, which tools. Spec 022's trusted operator declarations
> decide only where each declared need leads on this host (the real destination, the host
> directory and its access), and can leave a need unmet, never add one.

**Spec 011, "Attach widens the cell and does nothing else" (`011:165-169`).** Replace "A mode
decides whether a granted call reaches a live service or a recording; it is not itself a grant,
and that propagation is untouched here." with:

> A mode decides whether a granted call reaches a live service or a recording. Under spec 022 it
> selects which operator entry realizes each host, so an attached plasmid's mode never changes:
> an attach whose propagation would change it is refused with 104.

**Spec 001 §2 (`001:139-141`).** After "`.plasmosome/genomes/<name>.toml`; the share/export form
is `*.genome.toml`.", add:

> That project-local file is not yet a selection source: until a local genome grammar exists,
> `cell.new` selects a genome only by `artifact` (spec 022).

**Spec 001 §3.5.** Replace "An absent artifact preserves local selection." with:

> An absent artifact with a named genome refuses with 101 until a local genome grammar exists
> (spec 022).

**Spec 001 §3.10, propagation.** After "Inherited levels yield to the new explicit declaration
(D2b rule 2).", add:

> Propagation never changes the mode of a plasmid that is already attached: when the new closure
> would give an attached provider a different mode, the attach refuses with 104 naming that
> provider (spec 022).

**Spec 001 §3.10, credentials (`001:423-428`).** After "never a silent downgrade.", add:

> Until credential custody exists, a reference that passes this validation still refuses the
> attach with 103 `unsupported:credential:<ref>` (spec 022).

**Spec 001 §3.10, artifact.** Replace "Without artifact, existing local selection remains
unchanged." with:

> Without artifact, the plasmid and its local providers are selected through spec 022's operator
> declarations file: an unlisted ID is 101, and an unsafe or mismatched declaration is 108. Every
> closure member's declared network and workspace needs are resolved into complete capabilities
> through that file before prepare; an uncovered need is 103 naming the capability and plasmid.

**Spec 001 §3.12.** After "No artifact parameter is accepted here.", add:

> Reload re-resolves the plasmid's capabilities against the operator declarations file as it is at
> reload time; a registry member keeps its exact imported declaration. A reload whose new
> capabilities would widen what the plasmid holds refuses with 109 before prepare (spec 022).

**Spec 001 §4.1.** After the paragraph that introduces `registry_root`, add:

> The same strict JSON configuration also admits optional `operator_declarations`, an explicitly
> supplied absolute path to spec 022's operator declarations file. Nothing supplies a default; a
> relative or non-string value is invalid configuration. Only the resolving mutations of spec 022
> read it. Startup, recovery, status and every withdrawal never open it, and its absence or
> corruption never blocks them.

**Spec 001 §4.2, proxy selection (`001:1070-1072`).** Replace "At a new TCP connection/UDP flow,
the shim obtains that host's smallest currently eligible GrantId through4091grant.select before
checking packet transport and destination port against the selected recipe." with:

> At a new TCP connection/UDP flow, the shim obtains, through 4091 grant.select, the smallest
> currently eligible GrantId at that host among grants whose recipe port and transport match the
> flow. A host may hold grants for several ports; spec 022 keeps every grant at one host, port and
> transport equal in recipe within a cell.

and in the SelectionTarget paragraph, after "key is respectively the recorded guest_path,
guest_path, host, name or target.", add:

> For a new proxy flow, the request also carries the flow's `port` and `transport`, and selection
> considers only grants whose recipe matches them.

Without this change a host declared with two ports gets two grants, and every new flow selects
the smaller GrantId whatever its port, so one of the two ports always refuses.

**Spec 017, "Resolved recipes and their trusted input" (`017:119-120`).** Replace "this is not a
new user-editable manifest grammar." with:

> this is not a new manifest grammar for plasmid authors. Spec 022 defines the operator
> declarations file this resolver reads, and the resolution of ProxyMap and Mount; Broker,
> UdsSocket and SessionFile have no declaration source yet.

**Spec 020 (`020:431-432`).** Replace "Requests without `artifact` retain existing local
selection semantics." with:

> Requests without `artifact` use spec 022's local selection: a plasmid through the operator
> declarations file, and no genome until a local genome grammar exists.

## Open questions

These need the owner. Where this spec needs an answer to work, it sets a conservative default.

1. **Broker, UdsSocket and SessionFile.** No declaration section selects them. Should a new section
   declare them, or should they stay unreachable through public verbs for now (the default)? Spec
   018's closing scenario wants all five classes attached through public verbs, so it waits on
   this.
2. **Credentials in the meantime.** `[secrets]` and `[model]` refuse with 103 until custody exists
   (the default). That makes a model-provider plasmid unattachable. Is that right?
3. **Mock servers.** A mocked host is routed to a mock server the operator runs. Is the operator
   the right owner of that server, or should `[mock].backend` select something the kernel runs?
4. **Local genomes.** Where may a local genome file live, and in what grammar? The project
   directory is writable by the agent, so a genome found there cannot be trusted as declared.
5. **The operator file as a gate.** Every new network or workspace need waits for an operator
   entry, so the file gates reach in practice. Intent 010 leaves the gate's shape to the owner. Is
   this acceptable as part of the gate, or must the gate sit elsewhere?
6. **Widening reloads.** A reload that would widen refuses with 109, and the operator removes and
   re-adds the plasmid (the default). Should a widening reload instead pass through the approval
   gate?
7. **Pins and DNS.** Under `pin_cidrs`, a passthrough entry must use a numeric destination (the
   default), which gives up DNS failover. Should a DNS destination be allowed, with every resolved
   address checked against the pins at connection, which needs a ProxyRecipe field?

Not for the owner, but open: spec 011's grammar still defaults `workspace.mount.backend` to
`"virtiofs"`. Removing that field is a spec 011 change this spec does not make.

## Acceptance

Each item names the broken implementation it catches. Items 1–5, 10–18 and 20 run at the model
level against the resolver and journal. Items 6–9 and 19 need real files and directories, the
Darwin OS mechanism level. Nothing here needs an owner gate except a real Mount (O-7) or ProxyMap
(O-11), which no item requires. Item 17's first case needs a registry import (spec 020).

1. A plasmid declaring hosts `a.example` and `b.example` and ports 443 and 8443, with full
   coverage, where one entry uses `udp` and one sets `allow_private: true`. The journal's prepare
   holds four ProxyMaps in hosts-then-ports order, each with its entry's exact route, transport,
   destination, port and allow_private, and four distinct fresh IDs. Catches: dropping a pair,
   reordering, or filling a recipe field with a default.
2. Remove one of those four entries and attach the same plasmid to a fresh cell. It refuses with
   103 `entry:network:<h>:<p>` naming that exact pair and the plasmid, and the cell's journal has
   no record of it. Catches: partial attach, and naming the wrong pair.
3. One plasmid with `[mock].hosts` covering its host and two entries, one `passthrough` and one
   `simulate`, with different destinations. With no mock the passthrough destination is granted;
   with `--mock simulate` the simulate destination; with `--mock capture` it refuses with 103. A
   provider that inherits `simulate` through D2b gets its simulate entry. Catches: taking the
   first entry for a host and port whatever its mode, and falling back to another mode.
4. Requirer R requires key `k`, and listed plasmid P provides it. Entries exist for each under its
   own ID only. Both attach; R's operations name only R's hosts and P's only P's. Move P's entry
   under R's ID: the attach refuses with 103 naming P. Catches: resolving a member against
   another member's entries.
5. Provider discovery. Two listed plasmids both provide `k`: 100 with both IDs. None does: 103
   with `capability` `k`. A provider of `k` is already attached and its declaration file is then
   deleted: R still attaches, and the provider's holdings are unchanged. Catches: an undefined
   candidate set, and re-reading an attached provider.
6. Attach, change the entry's `destination`, restart the controller. Recovery succeeds and holds
   the original destination. Reload: the new generation holds the new destination with a fresh
   ID. Catches: recovery that re-reads the operator file, and a reload that reuses stale recipes.
7. Reload widening. Flip `allow_private` from false to true: reload is 109 and nothing is
   prepared. Change a Mount from `read_only` to `read_write`: 109. Add a host to a local
   declaration: 109. Catches: a reload that silently grants more.
8. Trust checks on the operator file, each refusing with 108 whose `detail` names the check, and
   none leaving a journal record:
   - a group-writable file (`mode`), and a 0600 file with an `everyone allow write` ACL (`mode`);
   - a second hard link to the file (`links`);
   - a final-component symlink to an otherwise valid trusted file (`path`);
   - a symlinked ancestor directory pointing at a valid trusted directory (`path`);
   - a group-writable parent directory (`parent` or `ancestor`);
   - a FIFO at the path, refused within one second rather than hanging (`not_regular`);
   - a file over 1 MiB (`size`), non-UTF-8 bytes, a duplicate key, an unknown field and
     `version: 2` (`content`);
   - the key pointed at root-owned `/private/etc/hosts` (`owner`, not a JSON fault).

   Then delete the file and restart: startup and recovery succeed. Catches: a reader that trusts
   the path text, checks only the file's own mode, follows links, blocks on a FIFO, or reports
   every fault as a parse error.
9. Writes through a Mount. Cell A holds a `read_write` Mount of directory S. Put the operator
   file inside S: every resolving request refuses with 108 `inside_mount_source`. Do the same
   with a listed declaration inside S, and with S only declared in the operator file and not yet
   held. Move the file outside S: requests succeed. Catches: trusting a file the workload can
   write because it has the controller's UID.
10. Local selection. An unlisted ID refuses with 101 `plasmid <id>`, even with a valid
    declaration for that ID at the project root and under `.plasmosome/plasmids/`. An entry with
    no `declaration` is 101. An unsafe declaration is 108 with `path` the declaration. A listed
    declaration whose `id` differs is 108. A listed, matching declaration attaches with source
    `{"kind": "local"}`. Catches: searching any default path.
11. `cell.new` with a genome name and no artifact refuses with 101 `genome <name>` and creates no
    cell directory, even with a valid genome at `.plasmosome/genomes/<name>.toml`. Catches:
    reading spec 001 §2's path as a selection source.
12. Author refusals. Each of these refuses with 108 carrying the field, a nonempty `fix` and the
    plasmid ID, and attaches nothing: ports `70000`, `-1`, `0`, `65536`, `"443"` and `443.5`; a
    repeated port; a repeated host; hosts with no ports; an upper-case host; an IP-literal host;
    a malformed CIDR; `192.0.2.1/24`; `[workspace]` with no `mount`; `dst = "workspace"`; a
    `requires` key beginning `entry:`. Catches: today's truncating parser, a refusal with no
    `fix`, and a silently dropped section.
13. Pins, with `pin_cidrs = ["192.0.2.0/25"]`. Passthrough destination `192.0.2.10` attaches;
    `192.0.2.200`, `192.0.2.10.example.com`, `::192.0.2.10` and `::ffff:192.0.2.10` each refuse
    with 103; `192.000.002.010` refuses the operator file with 108. The same plasmid with
    `--mock simulate` and a `127.0.0.1` simulate entry attaches. The in-tree `github-pr` fixture
    attaches in passthrough and in simulate against section 4's example file. Catches: a string
    prefix check, converting IPv6 forms to IPv4, and pins that make a plasmid unmockable.
14. One cell, two plasmids X and Y, both declaring `api.github.com:443`. X attached at
    `simulate`, then Y at `passthrough`: Y refuses with 104 naming both. Both at `passthrough`
    with entries whose destinations differ: 108. Both at `passthrough` with equal recipes: both
    attach. Catches: letting UUID order choose between a mock and a live service.
15. Provider P is attached at `passthrough`. Requirer R is added with `--mock capture`, which
    would propagate to P: it refuses with 104 naming P, and P's recorded holdings are unchanged.
    Catches: re-resolving an attached provider, and reporting a mode its grants do not have.
16. Hosts `a` and `b`, `[mock].hosts = ["a"]`. At `--mock simulate`, `a` gets its simulate entry,
    `b` gets no capability, and the attach succeeds. At passthrough both are granted. Catches:
    routing a host the mock does not stand in for to the live service.
17. Unsupported sections. `[commands]` refuses with 103 `unsupported:commands:<id>`, `[secrets]`
    with `unsupported:credential:<ref>`, and a `[model]` with no `credential` key with
    `unsupported:credential:model-provider/key`, each with no `fix`. Catches: a wrong capability
    string, or refusing `[model]` only when `credential` is written out.
18. With `operator_declarations` unset, and a covering file placed at the instance root,
    `~/.plasmosome`, the registry root and the working directory: a registry-imported plasmid with
    no needs attaches; a local `plasmid.add` is 101; a registry plasmid with `[network]` is 103.
    Catches: a hidden default file.
19. Mount sources equal to, inside, and above each guarded directory: the instance root, the
    registry root, the control-socket directory, the configuration file's directory, the operator
    file's directory, a declaration's directory and the `membraned` artifact's directory. Each
    refuses with 108 before prepare. A sibling named `<instance-root-name>-x` attaches. On a
    case-insensitive volume, a source spelled with different letter case from the instance root
    refuses. A source with a symlink component refuses. Catches: a string-prefix check, a
    case-sensitive path comparison, and a following walk.
20. TCP and UDP entries for one host, port and mode give two capabilities. Entries for one host,
    port and transport with modes `["simulate","capture"]` and `["capture"]` refuse the file with
    108, and so does a mode repeated inside one `modes` array. Catches: refusing QUIC, and
    missing a partial overlap.

## Out of scope

- How ProxyMap and Mount are enforced. That is spec 017 and spec 001 §4.2.
- The approval gate for a declaration. That is open in spec 011 and intent 010.
- Credential custody, command subjects and attestation.
- A local genome grammar.
- Watching the operator file, or pushing its changes into running cells.
- Version selection between providers, which spec 001 already does.
