---
id: 022
title: Trusted operator declarations, and how a manifest becomes exact grants
status: draft
intents: [004, 012, 003]
---

## Behavior

A plasmid manifest says what a plasmid needs in its author's words: these hosts on these ports,
a workspace at this guest path. Spec 017 grants nothing that vague. It needs complete recipes:
where a proxied host really connects, over which transport, and which host directory backs a
mount. Those are facts about this host. The author cannot know them and the agent must not choose
them. This spec adds one file the operator writes, the **operator declarations file**, and says
how the controller combines it with a manifest to produce the exact capabilities a plasmid holds.
The manifest still decides what a plasmid may reach. The operator file only decides how each
declared need is met on this host. A need the manifest does not declare gets no capability,
whatever the file says.

Resolution runs once per mutation that attaches plasmids (`plasmid.add`, `plasmid.reload`, and
`cell.new` with a genome), before anything is prepared. Every declared host and port, and every
declared workspace, must be covered by the operator file for the plasmid's resolved mock mode.
If one is not, the request refuses with code 103 naming the missing capability and the plasmid,
and nothing is prepared. The output is a fixed list of complete spec 017 capabilities, each with
a fresh identity. Spec 017 and spec 008 take it from there. Recovery never reads the operator
file: it replays the complete recipes the journal recorded. So editing the file never changes a
capability a cell already holds. A change reaches a cell only through an explicit add or reload.

This exists for three intents. Intent 004 needs every capability complete before it is granted,
so that removing it removes exactly that one. Intent 012 needs a grant to come from a declared
need, not from whatever the host happens to allow. Intent 003 needs recovery to work from the
journal alone, without a lookup table that may have changed since (spec 017's "Recovery never
consults a mutable name-to-recipe table"). This spec covers the two classes whose need a manifest
can express today, ProxyMap and Mount. Broker, UdsSocket and SessionFile have no manifest source
yet, and credentials stay refused; both are owner questions below.

**Platform.** The product is macOS first: Darwin arm64 is the only runtime host, and the cell is
a Linux guest run by libkrun. Linux/KVM host support is deferred, not dropped. The resolver is
portable logic plus file checks, and its file checks are defined for both platforms. Evidence for
each acceptance item says which level it reached: the model (fake and composite backends), the
Darwin OS mechanism, the Darwin pinned runtime, or Linux, which is deferred.

## Contract

### 1. Where the file comes from

The controller's strict JSON configuration gains an optional key, `operator_declarations`: an
absolute, NUL-free path string. Nothing supplies a default. It is not inferred from
`instance_root`, `registry_root`, the control socket, the caller's HOME or a request field. A
relative or non-string value is invalid configuration, refused at startup like any other.

When the key is absent, the controller behaves as if the file existed and listed no plasmid. Every
resolution that needs an entry then refuses with the code it would get for a missing entry
(section 5 and section 6). A plasmid that declares neither network nor workspace still attaches.

### 2. When it is read, and what never reads it

The file is read once per resolving request, at the start of resolution. One read serves every
member of that request's closure, so all of them are resolved against the same contents. Each
later request reads it again.

Startup, recovery, `plasmosome.status`, `plasmosome.recovery`, `cell.list`, `cell.status`,
`plasmid.list`, `plasmid.remove`, `cell.kill` and every withdrawal never open the file. A missing,
unsafe or corrupt file cannot block startup, recovery or removal. Recovery uses the recorded
recipes, as spec 008 already requires.

### 3. Trust checks on the file

The controller opens the file without following a symlink and checks the opened file, not the
path text:

- it is a regular file owned by the controller's effective UID;
- its mode grants no write to group or others, and no ACL entry grants write to another
  principal;
- no ancestor directory can be replaced by another principal, under the same rule spec 001 §4.1
  applies to the private socket directory;
- it is at most 1,048,576 bytes;
- it is UTF-8 JSON with no duplicate keys and no unknown fields, and `version` is exactly 1.

Any failed check refuses the request with code 108. `path` is the file's absolute path and
`detail` names the failed check and, for a content fault, the JSON location. The refusal carries
no `fix` field: spec 011's `fix` is the line a plasmid author would write, and an author cannot
repair the operator's file. Nothing is prepared.

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
         "destination": "api.github.com", "allow_private": false},
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

- `plasmids` maps a plasmid ID to its entry. A key that is not a valid plasmid ID refuses the file.
- `declaration` is optional: the absolute path of the plasmid's declaration file, used only for
  local selection (section 5).
- `proxy_maps` and `mounts` are required arrays and may be empty.
- A ProxyMap entry has exactly `host`, `port`, `modes`, `route`, `transport`, `destination` and
  `allow_private`. `host` follows spec 001's proxy host rule (a lower-case DNS name, never a
  numeric address). `port` is 1 to 65535. `modes` is a nonempty set over `simulate | capture |
  passthrough` with no repeats. `route`, `transport`, `destination` and `allow_private` follow
  spec 017's ProxyRecipe rules, and `route` is a nonempty, NUL-free label.
- A Mount entry has exactly `target` (an absolute, NUL-free guest path), `source` (an absolute,
  NUL-free host path) and `access` (`read_only | read_write`).
- Within one plasmid's entry, two ProxyMap entries with the same `host` and `port` must not share
  a mode, and two Mount entries must not share a `target`. Either refuses the file. So one declared
  need, in one mode, is met by at most one entry, and the operator never leaves the choice to
  chance.

Version 1 has no entries for Broker, UdsSocket or SessionFile, no guest IP routes, and no
credential material. Adding any of them is a new version.

### 5. Local selection of a plasmid

Spec 001 §3.10 says a `plasmid.add` without `artifact` "preserves local selection" but never says
what that selects. This spec defines it. The controller looks up the requested ID in the operator
file and opens the `declaration` path listed there, under the same checks as section 3. It parses
the file as a manifest and requires the manifest's `id` to equal the requested ID. The journal
records spec 008's `{"kind": "local"}` source, which asserts no content pin.

- An ID with no entry, or an entry with no `declaration`: code 101, `target` `plasmid <id>`.
- An unsafe or unreadable declaration file: code 108, `path` the declaration path.
- A manifest whose `id` differs from the requested ID: code 108, `path` the declaration path.
- A manifest that fails spec 011's grammar: its existing 108 refusal, unchanged.

The same rule selects each local member of a closure. A registry member (spec 020) keeps its
imported declaration; the operator file then supplies only its ProxyMap and Mount entries, keyed
by the member's plasmid ID.

Listing a plasmid in this file is not spec 011's approval gate. It records where the operator
keeps a declaration and how its needs are met on this host. Where the gate sits is still open in
spec 011, and this spec does not answer it.

`cell.new` with a `genome` name and no `artifact` has no local grammar to select from: spec 020
defines genomes only as registry releases. Until a local genome grammar exists, it refuses with
code 101, `target` `genome <name>`. This is an open question below.

### 6. From manifest sections to capabilities

Each member of the closure is resolved on its own, against its own manifest and its own entry in
the operator file, at its resolved D2b mode. A requirer never gains its provider's capabilities,
and a provider never gains its requirer's.

**`[network]` becomes ProxyMap.** For each host `h` in `hosts` and each port `p` in `ports`, the
resolver picks the one entry with `host` = `h`, `port` = `p` and the member's mode in `modes`.
That entry becomes one ProxyMap capability: `host` and `route` from the entry, and a ProxyRecipe
of the entry's `transport`, `destination`, `port` and `allow_private`.

- No such entry: code 103, `capability` `network:<h>:<p>`, `plasmid` the member's ID. An entry
  for another mode is never used instead.
- `hosts` nonempty and `ports` empty: code 108 under spec 011's author-refusal rule, field
  `network.ports`, `fix` `ports = [443]`, and the plasmid ID.
- A port outside 1 to 65535, or a ports entry that is not an integer: code 108, field
  `network.ports`. The resolver uses the declared values exactly. A parser that drops or
  truncates entries does not meet this.
- A host that does not meet spec 001's proxy host rule: code 108, field `network.hosts`.
- When `pin_cidrs` is nonempty, only an entry whose `destination` is a numeric address inside one
  of those CIDRs is eligible. A DNS-name destination is not eligible, because its addresses are
  not known until connection. If the only entry is ineligible, the result is the 103 above. An
  invalid CIDR is code 108, field `network.pin_cidrs`.

**`[workspace]` becomes Mount.** The target is `workspace.mount.dst`. The resolver picks the one
Mount entry with that `target`, and it becomes one Mount capability with the entry's `source`,
`target` and a MountRecipe of its `access`. With no such entry: code 103, `capability`
`workspace:<dst>`. The manifest's `backend` field selects no mechanism: every Mount is realized
as spec 017 and spec 001 §4.2 say, through the mediated guest filesystem. Spec 001 forbids a
virtiofs export, and the grammar's default of `"virtiofs"` does not override that.

A Mount source must not expose the kernel's own state. The resolver opens the source as a
directory and compares directory identities (device and inode), not path text. It refuses with
code 108, `path` the operator file, when the source is the same directory as, an ancestor of, or
a descendant of any of these: the instance root, the registry root if one is configured, the
directory of the control socket, the directory holding the operator file, or the directory of any
declaration the file lists. A source that does not exist or is not a directory refuses the same
way. This guards a trusted operator's mistakes; it does not detect a mount point inside the
source that reaches one of those directories.

**Other sections.**

- `[commands]` with any command: code 103, `capability` `commands:<id>` for the first command.
  Running a command as a subject needs spec 001's host-side attestation, which does not exist yet
  (code 110 on `cell.exec`). Attach refuses rather than grant a command nothing can run.
- `[secrets]` with any reference, or `[model]` (whose `credential` always has a value): code 103,
  `capability` `credential:<ref>`. Credential custody is reserved in spec 001, and spec 017 keeps
  credential material out of recipes. Whether to refuse these in the meantime is an owner
  question.
- `[model].endpoint` adds no capability of its own. A plasmid that must reach it declares its host
  in `[network]`.
- `[mock]` adds no capability of its own. It changes which entry a host uses, through the mode.
- `wasm` and `provides` add no capability under this spec. The plasmid interface is still
  reserved by spec 001 §5.

### 7. The output

Resolution of one member yields an ordered list of complete capabilities:

1. ProxyMaps, in `hosts` order, and for each host in `ports` order;
2. then the Mount, if any.

Each capability gets a fresh `GrantId` from spec 017's issuance rule, chosen before prepare. One
declared need yields exactly one capability. The list carries no reference back to the operator
file: the recipe is complete, and recovery needs nothing else. A member with no network and no
workspace yields an empty list and still attaches, as spec 008's empty attachment.

Refusals are checked for the whole closure before anything is prepared. When several apply,
one is returned; which one is not part of the contract.

### 8. After attach

Changing or deleting the operator file changes no capability a cell holds. Nothing watches the
file.

`plasmid.reload` re-resolves the plasmid against the file as it is at reload time, and every
capability it produces is a fresh replacement holding with a fresh ID. A local member re-reads
its declaration at the path the file lists now, as `{"kind": "local"}` allows. A registry member
keeps its exact imported declaration, as spec 001 §3.12 requires; only its ProxyMap and Mount
entries are re-read. Spec 008's reload preflight is unchanged and applies to the re-resolved set:
a reload that cannot stage its replacements without withdrawing an old holding or widening
access still refuses before prepare.

### 9. Refusals, in one place

| Case | Code | Fields |
| --- | --- | --- |
| Operator file unsafe, oversize, malformed, or ambiguous | 108 | `path` = file, `detail` |
| Mount source exposes kernel state, or is not a directory | 108 | `path` = file, `detail` |
| Local ID not listed, or listed with no declaration | 101 | `target` = `plasmid <id>` |
| Local genome without `artifact` | 101 | `target` = `genome <name>` |
| Declaration unsafe or its `id` differs | 108 | `path` = declaration, `detail` |
| Hosts with no ports; invalid port, host or CIDR | 108 | spec 011 field, `fix` where 011 requires it, plasmid |
| Need not covered in this mode | 103 | `capability`, `plasmid` |
| Commands, secrets or model credential declared | 103 | `capability`, `plasmid` |

No new code is added. Every refusal happens before prepare and leaves no journal record.

## Changes proposed to accepted specs

These are proposed text, not edits made in this PR.

**Spec 001 §4.1**, after the paragraph that introduces `registry_root`, add:

> The same strict JSON configuration also admits optional `operator_declarations`, an explicitly
> supplied absolute path to spec022's operator declarations file. Nothing supplies a default; a
> relative or non-string value is invalid configuration. Only the resolving mutations of spec022
> read it. Startup, recovery, status and every withdrawal never open it, and its absence or
> corruption never blocks them.

**Spec 001 §3.10**, replace the last sentence of the `artifact` bullet ("Without artifact,
existing local selection remains unchanged.") with:

> Without artifact, the plasmid is selected locally through spec022's operator declarations file:
> an unlisted ID is101, and an unsafe or mismatched declaration is108. Every closure member's
> declared network and workspace needs are resolved into complete capabilities through that file
> before prepare; an uncovered need is103 naming the capability and plasmid.

**Spec 001 §3.5**, replace "An absent artifact preserves local selection." with:

> An absent artifact with a named genome refuses with101 until a local genome grammar exists
> (spec022).

**Spec 001 §3.12**, after "No artifact parameter is accepted here.", add:

> Reload re-resolves the plasmid's capabilities against the operator declarations file as it is
> at reload time (spec022); a registry member keeps its exact imported declaration.

**Spec 017**, "Resolved recipes and their trusted input", after "The caller resolves trusted
operator declarations into them before choosing/preparing the operation.", add:

> Spec022 defines those declarations and the resolution of ProxyMap and Mount; Broker, UdsSocket
> and SessionFile have no manifest source yet.

## Open questions

These need the owner. This spec does not decide them.

1. **Broker, UdsSocket and SessionFile.** No manifest section selects them. Should a new manifest
   section declare them, should the operator file attach them to a plasmid by itself, or should
   they stay unreachable through public verbs for now? Spec 018's closing scenario wants all five
   classes attached through public verbs, so it is blocked until this is answered.
2. **Credentials in the meantime.** This spec refuses `[secrets]` and `[model]` with 103 until a
   custody contract exists. That makes a model-provider plasmid unattachable. Is that the right
   interim, or should those sections attach with no capability and a recorded denial?
3. **Mocks through mode-keyed entries.** A mocked host is routed by an operator entry for the
   `simulate` or `capture` mode, pointing at a mock server the operator runs. Is the operator the
   right owner of that server, or should `[mock].backend` select something the kernel runs?
4. **Local genomes.** Where may a local genome file live, and in what grammar? The project
   directory is writable by the agent, so a genome found there cannot be trusted as declared.
5. **One file per instance or per cell.** Version 1 is one file per instance. Should a cell be
   able to carry its own narrower file?
6. **ProxyMap realization depends on O-11.** This spec produces complete ProxyMap capabilities,
   but guest TCP behind the TUN needs either a userspace TCP/IP stack or an nftables redirect.
   Until that is decided, ProxyMap reaches the model level only.

Not for the owner, but open: spec 011's grammar still defaults `workspace.mount.backend` to
`"virtiofs"`. Removing that field is a spec 011 change this spec does not make.

## Acceptance

Each item names the broken implementation it catches.

1. Attach a plasmid declaring two hosts and two ports with full operator coverage. The journal's
   prepare holds four ProxyMap operations in hosts-then-ports order, each with the entry's route,
   transport, destination, port and allow_private, and four distinct fresh IDs. Catches: a
   resolver that drops a host-port pair, reorders, or fills a recipe field with a default.
2. Remove one of those four operator entries and attach the same plasmid to a fresh cell. The
   request refuses with 103 `network:<h>:<p>` naming that exact pair and the plasmid, and the
   cell's journal has no record of it. Catches: partial attach of the covered pairs, and a
   refusal that names the wrong pair.
3. Differential on mode: one plasmid, one entry for `passthrough` only. Attach with no mock
   succeeds; attach with `--mock simulate` refuses with 103. Catches: falling back to an entry for
   another mode.
4. A closure where requirer R needs `api.a.com` and provider P needs `api.b.com`, with entries for
   each under its own ID only. Both attach, R's operations name only `api.a.com` and P's only
   `api.b.com`. Then move P's entry under R's ID: the attach refuses with 103 naming P. Catches:
   resolving a member against its requirer's or provider's entries.
5. Attach, then change the entry's `destination` and restart the controller. Recovery succeeds,
   and the recovered desired state holds the original destination. Then reload: the new
   generation holds the new destination with a fresh ID. Catches: recovery that re-reads the
   operator file, and a reload that reuses stale recipes.
6. Make the operator file group-writable, then a symlink, then owned by another UID (where the
   test can arrange it), then add a duplicate key, then an unknown field, then `version: 2`. Each
   refuses the attach with 108 carrying the file path and no `fix`. Then delete the file and
   restart: startup and recovery succeed. Catches: a reader that trusts the path text, a lenient
   parser, and a startup that depends on the file.
7. Two ProxyMap entries with the same host and port and overlapping modes, and separately two
   Mount entries with one target. Each refuses with 108. Catches: a resolver that picks one entry
   silently.
8. Mount sources equal to, inside, and above the instance root, and a source reached through a
   symlink to the registry root. Each refuses with 108 before prepare. A sibling directory of the
   instance root attaches. Catches: a path-prefix comparison, and a guard that follows symlinks.
9. Local selection: an unlisted ID refuses with 101 `plasmid <id>`; a listed declaration whose
   `id` differs refuses with 108; a listed, matching declaration attaches with source
   `{"kind": "local"}`. Catches: searching the working directory or any default path.
10. `cell.new` with a genome name and no artifact refuses with 101 `genome <name>` and creates no
    cell directory. Catches: inventing a local genome lookup.
11. A manifest with `hosts` and no `ports` refuses with 108, field `network.ports`, a nonempty
    `fix` and the plasmid ID. A manifest with `ports = [70000]` refuses with 108; it does not
    attach a grant on port 4464. Catches: today's parser, which truncates ports with `as u16`.
12. `pin_cidrs = ["192.0.2.0/24"]`: an entry with destination `192.0.2.10` attaches; one with
    `198.51.100.1` or a DNS name refuses with 103. Catches: ignoring pins, or checking them only at
    connect time with no recipe field to check against.
13. Manifests with `[commands]`, `[secrets]` and `[model]` each refuse with 103 before prepare.
    Catches: attaching a plasmid whose declared needs nothing can grant.
14. With `operator_declarations` unset: a plasmid with no network and no workspace attaches as an
    empty attachment, and one with `[network]` refuses with 103. Catches: a hidden default file.

## Out of scope

- How ProxyMap and Mount are enforced. That is spec 017 and spec 001 §4.2.
- The approval gate for a plasmid's declaration. That is open in spec 011 and intent 010.
- Credential custody, command subjects and attestation.
- A local genome grammar.
- Watching the operator file, or pushing its changes into running cells.
