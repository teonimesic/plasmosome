---
id: 032
title: Credential custody, where a credential is kept and how a cell uses it without holding it
status: draft
intents: [012, 011, 006]
---

## Behavior

A cell uses a credential without ever holding it. The real value stays on the host, in a file the
operator names. Only the cell's `membraned` reads it to use it, when it applies the grant the
credential serves; the controller only checks it, and drops what it read. The cell's egress relay
then attaches it to requests the relay itself sends to the granted destination, and to no others.
A component names the credential by its reference and never holds more than that name. A workload
holds at most a **surrogate**: a random stand-in that works only in its own cell's relay, for its
own attachment. When the attachment ends, the binding ends with it, and nothing the guest holds
works any more.

This is the gap spec 026 names: "A harness that makes its own TLS connections needs the real
credential in the guest, or an intercepting proxy on the host. Neither is designed."
(`026:92-93`). Until now spec 022 refused every reference: "`[secrets]` with any reference, or
`[model]` (whose `credential` defaults to `model-provider/key`): 103, `capability`
`unsupported:credential:<ref>`." (`022:348-349`). And spec 028 left one rule waiting: "a minted
credential must not outlive the attachment carrying it" (`028:635-636`). Three intents want this.
Intent 012 says "a capability is revoked as soon as it stops being needed", and "The agent inside
is the last thing that should be relied on to give a capability up." A value read in the guest
cannot be taken back, so revocation is real only if the value never enters. Intent 011 says "If
the isolation only holds when the workload cooperates, it is not isolation I can rely on." So a
harness presents its key the ordinary way, and custody never relies on it to keep the key safe.
Intent 006 asks for "high security standards in both its isolation strategy and its dependencies".

What changes, and what does not. Spec 022's 103 becomes a binding when an operator entry covers
the reference and its delivery mode is one this spec realizes: `handle` for a component, and
`inject` for an HTTP or process consumer with a path scope. `helper`, `mint` and `[model]` stay
refused, each with its reason. The **harm guards** in §9 are fixed now, and each has an acceptance
item. Shapes that only a consumer can teach wait in §10, each with a default, as spec 031 does with
its W items. This spec edits no other spec; it proposes changes below. Its status is `draft`, and
its acceptance is the owner's: question Q1 carries spec 022's question 2, and question Q3 decides
whether the relay may intercept a harness's TLS.

**Two paths.** The **component path** is spec 031's `handle` for a `wasm` consumer, and its first
consumer is `pr.comment`. It needs no new TLS work, because spec 031's relay already makes every
TLS connection a component's request needs. The **workload path** is `inject` for an `http` or
`process` consumer, and its first consumer is a harness's model key. There the relay terminates
TLS for the flows of an attachment that binds a credential at their target, and presents a
certificate from an **instance authority** that the guest image trusts. A client that does not
trust it fails its handshake and never gets the value. The component path rests on spec 031,
which is a draft, so its rules take effect only once spec 031 is accepted.

**Levels.** Each acceptance item names the lowest level that proves it: the **model**
(controller, journal, fake backend, injected clock), the **membrane** (`membraned`'s relay on
Darwin, with a fake guest bridge, a fake component host, fixture TLS servers, test roots and a
test authority), or the **pinned runtime** (a booted repo-built cell). As in spec 028, "That waits
on owner decisions O-1" (`028:34`), O-6 and O-7, and here also O-11, for a real ProxyMap.
Pinned-runtime items are reported not run until then, never passed.

**Platform.** macOS first: Darwin arm64 is the only runtime host. The file checks are Darwin's,
as spec 022 §3 states them. A Linux host is deferred, not dropped.

## Threat model

Trust runs as spec 026 says: "Trust runs from the owner, and the operator files the owner writes,
to the host kernel and the binaries built from this tree." (`026:62-63`). The operator file, every
secret file it names and the instance authority's key are the owner's.

Defended:

- **A hostile workload or a poisoned agent in the guest** that wants the value, wants to use it at
  another host or path, or wants to use it after detach. The guest never holds the value. Its
  surrogate works only through the relay, at the bound target, while the attachment stands.
- **A component that escaped the WebAssembly sandbox.** Spec 031 says a component can never
  reach "a credential's value; the workload's processes and memory" (`031:169`). This spec keeps
  that true: the component host holds no value, no surrogate and no key.
- **A plasmid author** who names references and hosts. The declaration bounds what may be bound.
  Only the operator places a credential, and only at a target the same plasmid holds.
- **A bound host that echoes the value exactly**, in a response head or an unencoded body. On a
  request that carries a value, the relay asks for an unencoded response, refuses one it cannot
  scrub, and replaces each exact copy (§5).

Not defended:

- **A same-UID process on the host.** Spec 026 says "a same-UID process defeats every host-side
  check (spec 001)" (`026:102`). Such a process can read the secret file and `membraned`'s memory.
- **Misuse within scope at the bound host.** Custody bounds where and while a credential is used,
  not what an admitted request does there.
- **A copy the bound host transforms.** JSON that writes `/` as `\/`, percent-encoding, base64, or
  an echo split or reversed is not an exact copy, so the scrubber cannot find it. `path_scope` is
  the operator's bound on which paths can echo (Q7).
- **A client that inspects the certificate chain**, or checks certificate transparency. It can
  tell that the workload path's TLS is intercepted.
- **A value the operator places in a cell some other way** (§7).

## Contract

### 1. Where a credential is kept

A credential's value is kept at rest in one file per credential, which an entry in version 2 of
spec 022's operator declarations file names. It is never written inline in that file, never taken
from the controller's environment, and never produced by running a command. The macOS Keychain is
not a source yet (Q2).

**Version 2.** A version 2 file is version 1 plus an optional `credentials` array in each plasmid
entry. A version 1 file stays valid and binds no credential. `credentials` in a version 1 file is
an unknown field (`content`). Spec 022 reserved this: "adding any of them is a new version."
(`022:166-167`).

```json
{
  "version": 2,
  "plasmids": {
    "github-pr": {
      "declaration": "/opt/operator/plasmids/github-pr/plasmid.toml",
      "proxy_maps": [
        {"host": "api.github.com", "port": 443, "modes": ["passthrough"], "route": "github-api",
         "transport": "tcp", "destination": "140.82.112.6", "allow_private": false}
      ],
      "mounts": [],
      "credentials": [
        {"ref": "github:token:api", "host": "api.github.com", "port": 443,
         "header": "authorization", "scheme": "Bearer",
         "source": {"file": "/opt/operator/secrets/github-pr-api"},
         "surrogate_prefix": "ghp_"}
      ]
    }
  }
}
```

A credential entry has these fields. The names are provisional. A field outside these rules
refuses the file with 108 (`content`).

- `ref`: the ID of one reference in the plasmid's `[secrets]`.
- `host`, `port`, and an optional `transport` (default `tcp`): the target of one of the same
  plasmid's ProxyMap holdings (§3). `udp` is refused, since the relay intercepts TCP flows only.
- `header`: a lower-case HTTP field name, such as `authorization` or `x-api-key`. `host`,
  `content-length`, `transfer-encoding`, `connection`, `te`, `upgrade` and `trailer` are refused:
  the relay replaces every field of the name it binds, and these decide routing or framing.
- `scheme`: optional, a token such as `Bearer`. The field's value is then the scheme, one space
  and the credential's value; without a scheme it is the value alone.
- `source`: exactly `{"file": PATH}`, where PATH is canonical as spec 022 §4 defines it. No other
  kind of source exists.
- `surrogate_prefix`: optional, 1 to 16 letters, digits, `_` and `-` (Q8).
- `publishers`: optional, a nonempty list of `{"registry_id": UUID, "publisher": NAME}`, each a
  spec 020 registry and a publisher in it (§3).

One reference may have entries at several targets. Two entries with the same `ref`, `host`, `port`
and `transport` in one plasmid's entry refuse the file with 108 (`content`).

**The file checks.** A secret file is checked as spec 022 §3 checks the operator file, with two
rules made stricter. Each failure refuses the request with 108, `path` the secret file's absolute
path, and `detail` naming the check. The refusal never carries the file's bytes. These are
operator faults, so they carry no `fix`.

1. **Spec 022's rules 1 to 5.** The no-follow walk (`path`), the ancestors (`ancestor`), the
   parent (`parent`), the file itself, opened without following a link or blocking, and outside
   every Mount source (`inside_mount_source`). Spec 022 says of the file: "It must be a regular
   file (`not_regular`), owned by the controller's effective UID (`owner`), with exactly one link
   (`links`)" (`022:83-85`). Rule 4's mode and size are replaced by the two rules below.
2. **Mode.** No read or write permission for group or others, and no ACL entry granting read or
   write to another principal (`mode`). Spec 022 refuses only write on the operator file; a secret
   file must not be readable either.
3. **Size.** At most 65,536 bytes, read through a limit of 65,537, as spec 026 asks: "The parser
   checks the bound while reading, for example by reading through a limit of the bound plus one
   byte." (`026:408-409`). A larger file is `size`.

The controller runs all of them at resolution, rule 5 included. `membraned` runs all but rule 5
again at apply (§4). The instance authority's key file takes the same checks from the same two
readers, with C4's content rule in place of the one below.

**The content rule.** A secret file holds one value. After one trailing LF is removed, it is a
nonempty run of visible ASCII, 0x21 to 0x7E. A CR, any other LF, a NUL, a space or any other byte
is `content`. This stops header injection through CR or LF, and keeps a stray newline out of a
header.

**A later Mount.** Rule 5 sees the Mount sources that exist when a credential is resolved. A
Mount resolved later meets spec 022's Mount source guard instead, which this spec extends (the
proposed spec 022 change): no Mount source may be, lie inside, or be an ancestor of the directory
of a secret file or of the instance authority's key. So neither order of attach puts a secret in
a guest.

**When the file is read.** Twice for each use, and never otherwise:

- by the controller, at resolution, to check it (§3, §4);
- by the cell's `membraned`, at the apply of the holding the binding serves, to use it (§4).

Startup, recovery, status and every withdrawal never open it, as spec 022 says of the operator
file: "`plasmid.list`, `plasmid.remove`, `cell.kill` and every withdrawal never open it"
(`022:63`). A missing or unsafe secret file never blocks them.

**Why a file.** It reuses spec 022's walk and checks, and tests can drive it with plain files. A
Keychain read needs the user's session unlocked and Security.framework calls. A value inline in
the operator file would sit in a file read at every resolving request, parsed by code that reports
content faults. The controller's environment is inherited by its children and readable by same-UID
tools; the stub in `plasmosome-core`'s `gatekeeper.rs` reads `AK_SECRET_GITHUB` and
`AK_SECRET_MODEL` from it today. A command would run a program, which a name never permits under
spec 017: "not implicit permission to run a program, follow a redirect or use ambient DNS"
(`017:128`).

### 2. What a binding is

A **credential binding** joins one declared reference to one ProxyMap holding of the same
attachment. It is a record of the attachment, never of a recipe: spec 008's replacement gains
`credentials`, a list of bindings (the proposed spec 008 change). A binding holds:

- `ref`, the reference ID;
- `holding`, the `GrantId` of the ProxyMap holding it serves, with that holding's `host`, `port`
  and `transport`;
- `mode`, `handle` or `inject` (§3);
- `header` and `scheme`, from the operator's entry;
- `path_scope`, copied from the reference's scope, and empty when it has none;
- `source`, the secret file's path;
- `surrogate`, for `inject` only.

It never holds a value or a digest of one. Only its surrogate's length comes from the value (Q8).

**Why the attachment.** Spec 028 makes the attachment the unit of lifetime: "A lifetime belongs to
an **attachment**: one plasmid in one cell, one change in spec 008's journal." (`028:41`). Recipes
stay free of credentials: "Credential material must not be placed in contents, arguments or
recipes." (`017:130-131`). A sixth capability class would link two classes, which spec 017 forbids:
"No implicit ownership link to another class's upstream exists" (`017:417`). A recipe field would
break spec 022's rule that the grants at one host, port and transport in a cell "carry the same
recipe" (`022:297`) as soon as two plasmids bind there, and would leave no place for a surrogate
that differs per attachment.

**Apply, withdrawal and observation.** `membraned` gets each binding with the apply of the
holding it serves: spec 001 §4.1's `membrane.effect.apply` params gain `credentials`, the bindings
that holding serves, with their sources and surrogates. The controller sends paths, never a value
or a key. `membraned` drops each binding with that holding's withdrawal. `membrane.cell.observe`
lists each binding `membraned` holds, by reference, `GrantId`, mode and surrogate, never a value or
a source path (the proposed spec 001 change). A withdrawal finishes only once the observation lists
none of its holding's bindings, as spec 008 says: "Append and sync finish only after all required
cleanup is independently observed complete." (`008:310`). So recovery's resumed cleanup withdraws
a binding whose holding is gone. A binding listed for a holding the journal does not record is
stray, in exact drift (the proposed spec 008 change).

**The surrogate.** An attachment gets one surrogate for each `inject` reference it binds. It is
drawn at prepare from the operating system's random source, in the shape of Q8, with a random part
of at least 128 bits. The controller never issues a surrogate that a journal of the instance
holds: a draw that matches one is drawn again. It is recorded in the journal beside the binding.
It is not a secret: it works only in its own cell's relay, and only for its own attachment. Status
shows it (below). A reload that keeps the binding keeps the surrogate. It is never reused after its
attachment ends.

The kernel writes the surrogate nowhere in the guest. The operator passes it to the harness as on
any machine, for example with `cell.exec` and argv `["env", "API_KEY=<surrogate>", "harness"]`,
since "There is no environment, directory or user parameter." (`024:52`), and PID1 "calls `execve`
with the environment exactly" (`024:201`) three fixed variables. The harness then presents it the
ordinary way, as intent 011 asks. Spec 027 counts "a name or value the kernel wrote: in environment
variables" (`027:159`) as a hint, and the kernel writes none. One surrogate per attachment makes
revocation exact: removing one plasmid kills only its surrogate, and a reload keeps a running
harness's surrogate working.

**Status.** Each plasmid object in `cell.status` and `plasmid.list` gains `credentials`, always
present: a list of `{ref, host, port, transport, mode, surrogate}`, with `surrogate` only for
`inject`, and `[]` when the plasmid binds none. It never shows a value or a source path. The
operator reads the surrogate there.

### 3. From a reference to a binding

Resolution happens in spec 022's single read of the operator file, for each closure member, after
its ProxyMap holdings are resolved. Spec 001's credential validation runs first and keeps its 108.
Then each `[secrets]` reference goes through these steps, in declaration order.

1. **The mode.** The resolver takes the first mode in the reference's `delivery` that this spec
   realizes, written or derived, and records it. This spec realizes `handle`, for consumer `wasm`,
   and `inject`, for consumers `http` and `process` with a nonempty `path_scope`, when the
   controller's configuration names `credential_authority` (C4). Spec 011 derives one mode when
   `delivery` is absent: "`wasm` derives `handle`; `git` derives `helper`; `http` and `process`
   derive `inject` when the ref carries a non-empty `path_scope` and `mint` when it does not."
   (`011:368-370`). With no realized mode, the reference is 103, `capability`
   `unsupported:credential:<ref>`. So `git` with `["helper"]` or `["helper", "mint"]`, and an
   unscoped `http` or `process` reference, which derives `mint`, stay refused. `["mint", "inject"]`
   with a path scope binds as `inject`. Taking the first listed realized mode keeps the author's
   order, and grants no mode the author did not list.
2. **The entry.** The member's operator entry must hold at least one credential entry whose `ref`
   is the reference. Otherwise the reference is 103, `capability` `entry:credential:<ref>`,
   `plasmid` the member. This mirrors spec 022's network refusal: "the request refuses with 103,
   `capability` `entry:network:<h>:<p>`, `plasmid` the member" (`022:282-283`).
3. **The publisher.** A local plasmid's entry serves it. A registry member (spec 020) is served
   only by an entry whose `publishers` lists both the registry it was imported from and its
   publisher, the `publisher` of its `ReleaseRef`. A publisher's name is unique only within one
   registry: "A release address is `(registry_id, kind, population, publisher, name, version)`"
   (`020:59`). An entry with no `publishers`, or one that does not list that pair, counts as no
   entry: 103 `entry:credential:<ref>`. Spec 022 keys its other entries "by plasmid ID alone (the
   default), so an imported registry plasmid named `workspace`, from any publisher, gets the
   operator's `workspace` entries" (`022:561-562`). For a credential, that would let any
   publisher's plasmid of the same name act with the owner's token (Q5).
4. **The targets.** Each entry for the reference names a host, port and transport at which the
   member holds a ProxyMap in this request. Each becomes one binding at that holding. A binding
   serves the holding the plasmid has at its mode. At `simulate` or `capture` that holding leads
   to the operator's mock endpoint, and the relay attaches nothing there (§5): a mock gets the
   surrogate, or no credential, and never the value.
5. **The file.** The controller opens the entry's source under §1, reads it through its bound,
   checks its content and drops the bytes (§4). For an `inject` binding it also checks the
   instance authority's key file under §1's file checks, rule 5 included.

**What the operator file cannot add.** A credential entry whose `ref` names no reference in the
member's declaration, or whose target the member holds no ProxyMap at, refuses the request with
108, `path` the operator file, and `detail` naming the reference and the target. The declaration
bounds what may be bound; the operator only places it.

**`ttl`.** It is a minted value's expiry at its issuer, and only `mint` creates one. For `handle`
and `inject` nothing is created, so `ttl` changes nothing, and the attachment's lifetime bounds the
binding. An author who wants a shorter life writes `[lifecycle].lease_ms` (spec 028).

**`[model]`.** It stays 103, `capability` `unsupported:credential:<ref>`, with the reference
`[model].credential`, `model-provider/key` by default. A model key is declared instead as a
`[secrets]` reference with consumer `process` and a `path_scope`, which derives `inject` and
attaches. The reason: `ModelSpec` carries only an endpoint and a bare credential, with no consumer
or scope, and spec 022 already says "A plasmid that must reach the model endpoint declares its
host in `[network]`." (`022:354-355`). Question Q1 carries spec 022's question 2.

**The refusals.** Spec 022's 103 becomes this:

| Case | Reply |
| --- | --- |
| Covered by an entry, with a realized mode | a binding |
| No covering entry, or none for a member's registry and publisher | 103 `entry:credential:<ref>` |
| Its `delivery` names no realized mode | 103 `unsupported:credential:<ref>` |
| `[model]` | 103 `unsupported:credential:<ref>` (Q1) |
| An entry for an undeclared reference, or at a target not held | 108, `path` the operator file |
| A secret file or key file that fails a check | 108, `path` that file |

None carries a `fix`, as spec 022 says: "These are not author errors, so they carry no `fix`. The
`entry:` and `unsupported:` strings tell a client who can act." (`022:352-353`). Nothing is
prepared or journaled. When several apply, which one is returned is not part of the contract, as
in spec 022 §7.

**Reload.** "A reload never widens." (`022:372`). Before prepare, it compares the re-resolved
bindings with what the plasmid holds. It refuses with 109 `widening_forbidden` on any of these:

- a binding for a reference it did not bind;
- a held reference at a host, port or transport it did not serve, or at a mode it did not have;
- a `path_scope` entry the held binding did not have.

A changed source, header or scheme proceeds, and the replacement reads the value again at apply.
A binding the reload keeps, the same reference at the same target, keeps its surrogate. A dropped
binding narrows, and proceeds. So a rotated value reaches a running cell through a reload, and the
running harness keeps its surrogate.

**Recovery.** Recovery replays the journal and never opens the operator file or a secret file, as
spec 022 says: "Recovery never reads the operator file: it replays the recipes the journal
recorded." (`022:24-25`). The bindings and their surrogates come from the journal, and the
observation shows what each `membraned` holds (§2). A `membraned` that survived a controller
restart keeps the values it read at apply. A cell whose `membraned` is gone is unaccounted (spec
023) and serves nothing.

### 4. Who holds the value

Only the cell's `membraned` reads a value to use it. It reads it at the apply of the holding the
binding serves, from the source the journal records, and holds it in memory for that holding only.
The controller opens and checks the file at resolution, reads it through its bound to check its
content, and drops the bytes there.

- No control socket, guest lane, journal record, session log line, `membraned.log` line, reply or
  status carries a value. Only the relay's own TLS connection to the bound destination does.
- The controller never sends a value or a key to `membraned`. The apply params, the launch record,
  and the argv, environment and files of a launch carry paths only.
- Each `membraned` reads only its own cell's credentials.
- The controller stays out of the data path. Spec 028 keeps it out of the fence, "so the fence
  needs no timer and no controller" (`028:125-126`). A relay that asked the controller on each
  request would put it back in.
- No verb, new or old, has a value as its payload.
- `membraned` sets its core file size limit to zero before it reads a value or the key, so a
  crash writes neither to disk.

At apply, `membraned` repeats every check of §1 except spec 022's rule 5. That rule needs the
instance's journals, which only the controller reads, so it is the controller's, at resolution.
`membraned` opens the journaled path with spec 022's no-follow walk, never with an open that
follows a link. So a file swapped after resolution, for a link, a FIFO, or a file whose owner or
mode is wrong, fails the apply. A failed apply aborts the transaction as any failed apply does
(spec 001 §3.10, spec 008). The reply is 108, with `path` the secret file and `detail` the check.

**The type.** The value lives in one type of `plasmosome-core`'s gatekeeper module. It implements
no Debug, Display, Clone or Serialize, and has no conversion to a string or to bytes. It clears its
memory when dropped. It has three uses, each a method of the module: writing itself into an
outgoing header, finding its exact copies for the scrubber, and giving its length to Q8. So the
crate's rule still holds: "**Secrets never leave the gatekeeper.** Plasmids receive handles."
`Gatekeeper::get`, which returns a value as a `String`, and the `AK_SECRET_*` stub that fills
values from the environment, are removed. Nothing outside the module uses them today.

### 5. How a use is enforced

**The common rules.** The relay is the cell's host egress relay in `membraned`: spec 031's for a
component's requests, and spec 017's ProxyMap relay for the workload's flows. On both paths, it
attaches a binding's value to a request only when all of these hold.

1. The gate of the holding the binding serves admits the request first. From that gate's fence
   on, nothing is attached (§6). An attached request is an admission on that holding, so it counts
   as use for the holding's idle limit.
2. The request reaches the relay through that holding: a component's request at the holding's
   target, or a workload flow whose selected grant is that holding. Spec 001 §4.2 serves each flow
   from the "smallest currently eligible GrantId among the holdings at that target" (`001:1426`).
   A flow through another attachment's grant gets no value.
3. The holding is at `passthrough`. At `simulate` or `capture` nothing is attached.
4. The relay itself sends the request to the holding's recorded destination, resolved as spec 017
   says: "A DNS name is resolved only for this authorized destination." (`017:124`).
5. It sends it over its own TLS connection, after verifying the upstream's certificate for the
   binding's host against the relay's pinned roots and the path's extra roots: C4's `roots` on the
   workload path, and spec 031's `component_runtime.roots` on the component path. It never
   attaches a value over
   plain text: at a target whose upstream does not complete that handshake, nothing is attached.
6. It sends a request it wrote itself. It parses the request, checks it under the request rules
   below, and writes what it sends from what it checked: the method, the canonical path, the
   query, exactly one `Host`, the fields it kept, and framing it chose. It never forwards the
   guest's raw bytes, nor a component's.
7. An upstream connection that carried an attached request is never shared with another
   attachment's flow, with an unbound flow, or with a component request that names no reference.
   An attached request goes only on a connection that has carried nothing but requests the relay
   wrote under the request rules for the same attachment.
8. The relay never follows a redirect. It returns a redirect response as it came, and a request
   to the new location is a new request, checked like any other.

**The request rules.** These apply to every request on an intercepted flow, and to every
component request that names a reference. The relay refuses a request that breaks one before it
sends any byte. On the workload path it then closes the guest's connection.

1. **Target.** The request target is origin-form: a path and an optional query. An absolute-form
   target is refused unless its authority is the binding's host and port, and is then sent in
   origin-form.
2. **Host.** The request has exactly one `Host` field, equal to the binding's host, ignoring ASCII
   case, with no port or with the binding's port. On the component path the relay writes `Host`
   from the request's URL, so a `Host` the component sends is a second one, and is refused.
3. **Path.** The relay decodes percent-escapes once. It refuses a path whose decoded form holds a
   `/` inside a segment, a backslash, a `;`, a `%`, a NUL or other control byte, a `.` or `..`
   segment, or bytes that are not valid UTF-8. So `..;`, a raw `\`, `%2F`, `%2e`, `%25` and
   `%c0%ae` are each refused. Scope is checked on the decoded path, and the relay sends the path it
   checked, re-encoded in one form: each byte RFC 3986 allows in a segment as itself, and every
   other byte as an upper-case escape.
4. **Framing.** It refuses a request with both `Content-Length` and `Transfer-Encoding`, with two
   `Content-Length` values, with a `Transfer-Encoding` other than `chunked`, with an obs-fold, a
   space before a colon or a bare LF, with a header line or a head over its bound (C3), or with the
   binding's header more than once. A component's own `Content-Length`, `Transfer-Encoding` or
   `Connection` is refused, since the relay writes framing itself. It drops the client's
   `Connection`, every field that `Connection` names, `Keep-Alive` and `Upgrade`, so a request it
   attaches a value to never switches protocols.

Each of these rules closes one way to send the value somewhere the binding does not name: a path
trick, another site at one address, or a second request smuggled past the rules.

**Scope.** A path is inside a binding's `path_scope` when its decoded form, without the query,
matches an entry. An entry that ends in `/` matches itself and every path under it. A `*` segment
matches one nonempty segment, as in the fixture's `/repos/*/pulls/`. Any other entry matches only
itself.

**The component path.** A component's requests already leave through spec 031's relay, which
"makes the TLS connection, verifies the certificate against a pinned root set and
`component_runtime.roots`, and attaches any credential" (`031:201-202`). So this path needs no new
TLS work. Spec 031's draft says "The component host forwards each request's method, URL, headers
and body" (`031:532-533`), so the request rules apply to what it forwards. A component names a
reference on each request (C1). For a request that names one, the relay:

- refuses, before it sends any byte, a request naming a reference the plasmid's declaration does
  not have, or one with no binding at the request's host, port and transport (§8);
- refuses, before it sends any byte, a request whose path is outside the reference's `path_scope`,
  when it has one (§8); such a request is never sent, with the value or without it;
- removes every field of the binding's header name that the component sent, and adds one field:
  the scheme, one space and the value, or the value alone.

A request that names no reference goes as spec 031's relay sends it, with no value. The component
host never holds the value, a surrogate or the instance authority's key.

**The workload path.** A harness makes its own TLS connections, so a stream relay cannot see its
requests. The relay therefore terminates TLS only for the flows of an attachment that binds an
`inject` credential at the flow's target.

- A flow's target is its host, port and transport, as spec 001 §4.2 selects it. The relay
  intercepts a TCP flow only when its selected grant is the holding of an `inject` binding whose
  gate admits. A flow of an attachment that holds no binding there stays a relayed stream that the
  relay never reads. Spec 001 §4.2 picks each flow's grant, so when two attachments hold one
  target, a flow served from the one with no binding is not intercepted, and its surrogate reaches
  the host unchanged. That fails closed; an operator who wants the harness's flows bound gives the
  target to one attachment.
- On an intercepted flow, the relay presents a leaf certificate for the flow's host, the host its
  synthetic address is bound to, whatever name the client sent. The instance authority signs it
  (C4, G14). The operator adds the authority's certificate to the root image's system trust store.
- It offers only HTTP/1.1 to the client (C3).
- It reads each request with its bounded parser, applies the request rules and the substitution
  rules, and sends the request it wrote under the common rules.

**The substitution rules.** The relay puts a binding's value in place of its surrogate only when
all of these hold. Otherwise the request goes without the value, the surrogate as sent, and the
bound host refuses it as it refuses any wrong key.

1. The gate of the binding's holding admits the request, and the flow's selected grant is that
   holding.
2. The path is inside the binding's `path_scope` (**Scope**).
3. The binding's header holds, after the scheme and one space when the binding has a scheme,
   exactly the surrogate.
4. The surrogate is this binding's: the same cell, the same attachment, and a binding at this
   target.

The relay never substitutes in the URL, in any other field, or in the body. A request without the
binding's header, or whose header holds anything but this binding's surrogate, gets no value.

**A client with its own roots.** A client that trusts its own root bundle instead of the system
store does not trust the instance authority. The operator points it at the system store on its
command line, as on any machine behind an intercepting proxy (Q3). Otherwise it fails with its
ordinary unknown-authority error, and never gets the value.

**Reflection.** On a request it attaches a value to, the relay sets `Accept-Encoding: identity`
and removes `TE`, `Range` and `If-Range`. It refuses that request's response when it has a content
coding other than `identity`, a transfer coding other than `chunked`, or status 206: the guest's
connection is closed, or the component's request fails, and no byte of the body passes. On an
intercepted flow, and on a component's request at a bound target, it drops response trailers.
Then it scrubs. It replaces each exact copy of a value bound at the target, in the whole head of
each interim and final response, the status line and field names included, and in the body,
across chunk boundaries, under G12's hold-back. On the workload path it writes
that binding's surrogate when the surrogate has the value's length, and otherwise the placeholder.
On the component path it writes the placeholder: a run of `x` as long as the value. Each
replacement keeps the length, so the response's framing is unchanged; the price is that the run of
`x` tells a component the value's length. A copy the host transforms is not found (Q7).

**The parsers.** The relay's guest-facing TLS server, its request parser, its upstream response
parser and its scrubber read bytes that a hostile workload or a bound host writes. Spec 026's rule
applies: "A parser whose input a less-trusted party can write states a bound." (`026:405`). Each
also has a fuzz target, since "Hostile writers are a cell's workload, a plasmid author, a faulty
broker, and a control-socket client." (`026:412-413`). Their rows land with their tasks: "A new
parser lands in the same PR as its row below and its bound." (`026:414`).

### 6. How a binding ends

Spec 028 asks that "a minted credential must not outlive the attachment carrying it"
(`028:635-636`). This spec realizes no mint, and holds the same rule for every binding. A binding
ends with the holding it serves, for every cause spec 028 names (`lease`, `idle`, `exec` and
`provider`), and at `plasmid.remove`, at a reload's replacement and at `cell.kill`.

- **At B.** "From B on, the host gate of every grant in the attachment admits no request, on any
  reference, opened before B or after." (`028:123-124`). So the relay attaches nothing from B on,
  on any flow, old or new.
- **The leaf.** A leaf's notAfter is the `not_after` reading of the earliest bound among the
  admitting `inject` bindings at its target, as known at issue, and at most 24 hours after issue.
  "A lease bound is two readings." (`028:101`), and a certificate holds one UTC time, so the leaf
  takes the wall-clock one. It is an upper bound only. B can move earlier at a time unknown at
  issue: "An idle limit or exec scope that ends the lifetime first moves B earlier." (`028:58-59`).
  So the gate ends use at the current B, by either reading, whatever the certificate says. When no
  binding at the target admits, `membraned` destroys the leaf's key at once, and new flows there
  are not intercepted; a drain that never finishes keeps nothing. It issues a new leaf at the next
  handshake whenever the earliest admitting bound changes: when that binding fences or is
  withdrawn, or when a bound moves later. "Only `plasmid.renew`, a recorded operator verb, adds
  time" (`028:15`).
- **At withdrawal.** `membraned` drops the binding and clears the value's memory. A flow it
  intercepted before stays intercepted until it closes, and carries no value.
- **The surrogate.** It never works again after its attachment ends. A later attachment of the
  same plasmid draws a new one.

Why this is enough: "A revoke removes reachability, not objects already held" (`028:190`). The
guest never held the value, so nothing the guest holds outlives B. A request admitted before B may
finish within D with its credential attached, as spec 028 allows any admitted work: "IO admitted
before B may finish within D" (`028:196`).

### 7. When a value enters a cell

Never, under this spec. No mode delivers a value into a cell. A consumer that cannot work without
one is refused: `helper` and `mint` are 103 (§3), and a client that refuses the instance authority
fails its handshake (§5). Nothing falls back to delivering the value.

When `mint` lands, a minted value may enter, scoped by the reference's `scope`. It expires at its
issuer no later than the attachment's bound B. An attachment with `until_revoked` cannot carry a
minted value unless its issuer revokes it at withdrawal.

A value the operator places in a cell another way, such as a file in a Mount or an argument, is
outside custody. Spec 017 already says: "There is no promise to revoke a secret or data already
delivered to the cell." (`017:451`). Question Q4 asks whether an operator opt-in should exist.

### 8. What a denial looks like

**To the workload.** A refused request is an ordinary error from spec 024 §9's table, such as "An
established TCP flow is reset, so its next call fails with `ECONNRESET`." (`024:384`). A request
the relay does not substitute reaches the host with the surrogate, and the host refuses it as any
wrong key. The relay never answers with an HTTP status or page of its own: spec 027 counts "an
HTTP status or page from an intermediary" (`027:158`) as observing the membrane.

**To the author**, for task plasmosome-z8n3. Spec 011's rule decides what is reported: "A refusal
is reported only when the line that would have allowed it can be named." (`011:433`). On the
component path, three refusals are reported. Each is refused before any byte is sent, and the
report is on the plasmid whose component sent the request, which the relay knows.

- A reference the declaration does not have: `denied` the reference, `field` `secrets.refs`, and
  `fix` the entry `{ id = "<ref>", consumer = "wasm" }`. A bare reference would not parse: for a
  `refs` entry with no consumer, `plasmosome-core`'s manifest parser says "consumer must be wasm,
  git, http or process".
- A path outside the reference's `path_scope`: `denied` the path, `field` that reference's
  `scope.path_scope`, and `fix` the path.
- A host no grant of the cell holds, whatever reference the request names. Spec 011's network
  rule reports "A DNS lookup of, or a new flow to, a host that no grant of the cell holds"
  (`011:434-435`), with `field` `network.hosts`.

A declared reference with no binding at a held host is refused and not reported: only the
operator's entry would change it, as for "a Mount's access, which the operator decides"
(`011:442`). The request rules' refusals are not reported either, since no declaration line
changes them. On the workload path nothing is reported for want of a credential, because the relay
refuses no request for that; it only declines to substitute.

**Never a value.** No denial, refusal, log line or record names a value. No denial, log line or
status names a source path; only a 108 for a file that failed its check names that file, in
`path`.

### 9. The harm guards this spec fixes now

These hold whatever the consumers teach. Each is a rule with an acceptance item, and each says
what holds outside the guest.

- **G1. No value in the cell.** The kernel writes no value into the guest: no file, mount,
  environment variable, argument, DNS answer or log the workload can read. No Mount source is,
  lies inside, or is an ancestor of the directory of a secret file or of the instance authority's
  key. On a request that carries a value, the relay asks for an unencoded response, refuses one it
  cannot scrub, drops trailers, and scrubs each exact copy before the response reaches the guest.
  A copy the host transforms is not defended. The workload holds at most a surrogate.
- **G2. No value in any record or reply.** No value or key, and no digest or encoding of one, is
  in the journal, the session log, `membraned.log`, any message from the controller to
  `membraned`, the operator file, a recipe, status, a refusal, an error's text, a denial or the
  component host.
- **G3. One reader, one holder.** Only the cell's `membraned` reads a value to use it, at apply,
  from the journaled source, through the no-follow walk, and holds it in memory for that holding
  only. The controller reads it only to check it, drops it at once, and never sends a value to
  `membraned`.
- **G4. Attached only where granted.** A value is attached only to a request the relay wrote
  itself from its own parse, on a flow of the binding's own holding, sent over TLS to the recorded
  destination after verifying the certificate for the binding's host, and only under §5's request
  and substitution rules. This holds in every mode: at `simulate` and `capture` nothing is
  attached. Never on a redirect, which the relay never follows, and never on an upstream
  connection that another attachment's flow, an unbound flow or a component request naming no
  reference shares.
- **G5. Only while admitted.** The binding's gate admits the request before anything is attached.
  From the fence or the withdrawal on, nothing is attached on any flow, old or new.
- **G6. Explicit use.** A value is never added to a request that did not name its reference or
  present its surrogate.
- **G7. No way to ask.** No guest port, lane or verb carries a value or a request for one, and
  port 4041 stays unused. No control-socket verb returns a value.
- **G8. Refuse rather than expose.** An unrealized mode is 103. A client that refuses the instance
  authority fails its handshake, and a response the relay cannot scrub is refused. Nothing falls
  back to delivering the value.
- **G9. Revocation is the attachment's.** Every cause of spec 028, removal, a reload's replacement
  and `cell.kill` end the binding. A surrogate never works again after its attachment ends, and
  the controller never issues one that a journal holds.
- **G10. The declaration bounds, the operator places.** A binding exists only for a reference the
  declaration names, at a target the same plasmid holds. An entry naming anything else is 108.
- **G11. Invisible to the workload.** The kernel writes no name or value in the guest. The
  surrogate is shaped like a key (Q8), and every failure is an ordinary error. Limit: an
  intercepted client sees the instance authority's chain and an HTTP/1.1-only offer, which a
  client that inspects either can tell apart.
- **G12. Bounded parsers, and a hold-back that tells nothing.** The secret file is read through
  its bound. The relay's guest-facing TLS server, request parser, upstream response parser and
  scrubber state bounds and have fuzz targets, and their spec 026 rows land with their tasks. The
  scrubber holds back the last (value length − 1) bytes of each chunk until the next chunk arrives
  or the body ends. What it holds back, and when it releases it, never depends on the bytes or on
  the value, so the timing cannot tell the guest whether an echo matches the start of the value.
- **G13. Not keyed by ID alone.** An entry serves a registry member only when it names the
  member's registry and publisher.
- **G14. The instance authority.** Its private key never enters a cell. It is never in the
  journal, a session log, status, a refusal, a reply or the component host. Only `membraned` reads
  it, under §1's file checks, which the controller also runs, rule 5 included, at each resolution
  that binds an `inject` credential; the controller reads it only to check it, and drops the
  bytes. The authority's certificate file holds no key. A leaf certificate is issued only for a
  target with an `inject` binding that admits, and expires no later than that attachment's bound
  B as known at
  issue; with several bindings at the target, the earliest. That notAfter is an upper bound only:
  the gate ends use at the current B, and `membraned` destroys the leaf's key when no binding at
  the target admits. A leaf's own key stays in `membraned`'s memory.

### 10. What waits for a consumer

Each has a default. The first consumer's PR settles it, by editing this spec.

- **C1. How a component names a reference.** This is spec 031's W7: "Credential handles: task
  plasmosome-3g2c's spec, with `pr.comment` as the consumer." (`031:549`). Default: by reference
  ID, in a field of each request on the egress channel, outside the HTTP request itself. The first
  consumer is `pr.comment`.
- **C2. Header forms.** Default: one header, with an optional scheme. No Basic authentication
  until a `git` consumer needs it.
- **C3. HTTP towards the guest client, and the request parser's bounds.** Default: ALPN
  `http/1.1` only; a header line of at most 8,192 bytes and a request head of at most 65,536 bytes;
  a body passed through as it streams, never held whole. None is measured.
- **C4. The instance authority.** Default: the controller's configuration gains
  `credential_authority: {certificate: Artifact, key: PATH, roots: Artifact | null}` (the proposed
  spec 001 change). `certificate` is the authority's certificate, a file of exactly one PEM
  `CERTIFICATE` block and nothing else: one that holds any other block, a private key included,
  refuses startup, so the file the operator adds to the image carries no key. `key` is the
  canonical path of its private key, under §1's file checks, holding one PEM private key instead
  of §1's content rule. `roots` are extra roots the relay trusts upstream: for tests, as spec 031's
  `component_runtime.roots`, and in production for an upstream behind a private CA, such as an
  `allow_private` destination. The operator adds `certificate` to the root image with
  `tools/root-image`; its subject and its file name in the image name nothing about Plasmosome, as
  spec 027 asks of every name in the guest. The controller passes the paths, never the key, to
  each cell's `membraned` in its launch record (the proposed spec 023 change). A cell's
  `membraned` reads the key at the apply of its first `inject` binding, and drops it with its
  last. Without `credential_authority`, `inject` is not realized.
- **C5. The surrogate's shape.** Question Q8.

## Changes proposed to accepted specs

These are proposed text. The PR that adds this spec edits no other document. Each change is
applied by a later reviewed change to the document it amends, before any task that relies on it,
and none starts while this spec is a draft. Each item quotes the words it replaces or follows,
with their lines at main `54f24f8bcd4090b90f7a4097eb98fb48a4061c64`, and each spec's block names
that commit.

**Spec 022** (words read at main `54f24f8b`).

- Behavior: replace "Broker, UdsSocket, SessionFile and credentials are owner questions."
  (`022:35`) with "Broker, UdsSocket and SessionFile are owner questions. Credentials are spec
  032's: a declaration's `[secrets]` references resolve through version 2 of this file." Spec 031,
  a draft, proposes another text for the same sentence, beginning "Spec 031 gives Broker a source"
  (`031:749`). Whichever lands second keeps both: "Spec 031 gives Broker a source; UdsSocket and
  SessionFile are owner questions, and credentials are spec 032's."
- §3 rule 6: replace "and `version` is exactly 1 (`content`)" (`022:96-97`) with "and `version` is
  exactly 1 or 2 (`content`)".
- §4: replace "Version 1 has no entries for Broker, UdsSocket or SessionFile, no guest IP routes
  and no credential material; adding any of them is a new version." (`022:165-167`) with "Version
  1 has no entries for Broker, UdsSocket or SessionFile, no guest IP routes and no credentials.
  Version 2 (spec 032) is version 1 plus an optional `credentials` array in each plasmid entry; an
  entry names where a value is kept, never the value. Adding any other kind is a new version."
- §5, registry members: replace "The operator file supplies only its ProxyMap and Mount entries,
  keyed by its plasmid ID alone." (`022:239`) with "The operator file supplies its ProxyMap and
  Mount entries, keyed by its plasmid ID alone, and spec 032's credential entries, which serve it
  only when their `publishers` name its registry and publisher."
- §6, other sections: replace "`[secrets]` with any reference, or `[model]` (whose `credential`
  defaults to `model-provider/key`): 103, `capability` `unsupported:credential:<ref>`. Credential
  custody is reserved in spec 001, and spec 017 keeps credential material out of recipes. Spec
  001's credential validation runs first and keeps its 108." (`022:348-351`) with "A `[secrets]`
  reference resolves under spec 032 §3: a covered reference becomes a credential binding of the
  attachment; one no entry covers is 103, `capability` `entry:credential:<ref>`; one whose
  `delivery` names no mode spec 032 realizes is 103, `capability`
  `unsupported:credential:<ref>`. `[model]` (whose `credential` defaults to `model-provider/key`)
  is 103 `unsupported:credential:<ref>` (spec 032, question Q1). Spec 001's credential validation
  runs first and keeps its 108."
- §6, the Mount source guard: after "once spec 023 is accepted, the directory of every
  `cell_runtime` Artifact." (`022:338`), add "- the directory of every secret file the operator
  file names or a held credential binding records, and of the instance authority's key (spec
  032)." So a Mount whose source is, lies inside, or is an ancestor of one is 108.
- §7: after "no reference back to the operator file." (`022:360`), add "A member's credential
  bindings (spec 032) are recorded on its attachment, beside its capabilities, and also carry no
  value."
- §8, the widening list: after "`read_write` where the held Mount was `read_only`." (`022:378`),
  add "- a credential binding for a reference it did not bind, or a held reference bound at a
  host, port or transport it did not serve, or at a mode it did not have;" and "- a `path_scope`
  entry the held binding did not have." After "A changed route or destination, or a narrower set,
  proceeds with fresh replacement holdings." (`022:380`), add "So does a changed credential
  source: the replacement reads the value again at apply, and each kept binding keeps its
  surrogate (spec 032)."
- Question 2: replace "**Credentials in the meantime.** `[secrets]` and `[model]` refuse with 103
  until custody exists (the default). That makes a model-provider plasmid unattachable. Is that
  right?" (`022:552-553`) with "**Credentials.** Spec 032 answers most of this:
  a `[secrets]` reference attaches when an entry covers it and its mode is realized. `[model]`
  still refuses with 103, and spec 032's question Q1 carries the rest."
- Question 5: after "including a `read_write` Mount (§5)." (`022:562-563`), add "Spec 032 does not
  key credentials this way: a credential entry serves a registry member only when it names the
  member's registry and publisher."
- Acceptance item 15: in "`[commands]` is 103 `unsupported:commands:<id>`, `[secrets]`
  `unsupported:credential:<ref>`," (`022:711`), replace "`[secrets]`
  `unsupported:credential:<ref>`," with "an uncovered `[secrets]` reference
  `entry:credential:<ref>`, one whose `delivery` names only `helper` or `mint`
  `unsupported:credential:<ref>`,".
- Out of scope: replace "Credential custody, command subjects and attestation." (`022:739`) with
  "Command subjects and attestation. Credential custody is spec 032."

**Spec 026** (words read at main `54f24f8b`).

- Not covered yet: replace "**Credential custody for a foreign harness.** A harness that makes
  its own TLS connections needs the real credential in the guest, or an intercepting proxy on the
  host. Neither is designed. A credential delivered into a cell is assumed readable, and detach
  does not revoke what was read." (`026:92-94`) with "**Credentials outside custody.** Spec 032
  keeps every credential an operator entry names out of the cell: the cell's egress relay attaches
  it on the host, and only to the granted destination. A harness that makes its own TLS
  connections reaches one only through the relay's interception; a client that refuses the
  instance authority fails, and is never given the value. A credential placed in a cell any other
  way, such as a file in a Mount or an argument, is outside custody: it is assumed readable, and
  detach does not revoke what was read."
- §5, no row now. The relay's request parser and response scrubber are new parsers with hostile
  writers, and their rows land with their tasks, as "A new parser lands in the same PR as its row
  below and its bound." (`026:414`) requires.

**Spec 028** (words read at main `54f24f8b`).

- §5: after "These references are not residue, hold nothing unfinished, and leave readiness
  alone." (`028:207-208`), add "**Credential bindings** (spec 032) hold no value in the cell, so
  nothing a reference holds outlives B. From B the gate admits no request, so the relay attaches
  nothing; withdrawal drops the value from `membraned`; and the attachment's surrogate is never
  accepted again. A request admitted before B may finish within D with its credential attached,
  like any admitted IO."
- Out of scope: replace "How long a cell lives (spec 023), and revoking bytes or secrets already
  delivered into a cell." (`028:634`) with "How long a cell lives (spec 023), and revoking bytes
  already delivered into a cell. Spec 032 delivers no credential value into one."
- Out of scope: replace "A credential's own `ttl`: when custody lands (spec 022), a minted
  credential must not outlive the attachment carrying it." (`028:635-636`) with "A minted
  credential's own expiry. Spec 032 realizes no mint; when one lands, its value expires at its
  issuer no later than the attachment's bound B (spec 032 §7)."

**Spec 001** (words read at main `54f24f8b`).

- §3.6: after "Each object in `plasmids` carries `lifetime`, always present, as spec 028 section 9
  defines it." (`001:494`), add "- Each object in `plasmids` also carries credentials, always
  present, as spec 032 §2 defines them: each binding's reference, target, mode and, for `inject`,
  its surrogate; never a value or a source path."
- §3.9: replace "Each object carries `lifetime`, as in §3.6." (`001:583-584`) with "Each object
  carries `lifetime` and `credentials`, as in §3.6."
- §3.10: replace "Until credential custody exists, a reference that passes this validation still
  refuses the attach with 103 `unsupported:credential:<ref>` (spec 022)." (`001:627-628`) with "A
  reference that passes this validation resolves under spec 032 §3: it becomes a credential
  binding of the attachment when an operator entry covers it and its mode is realized, and
  otherwise refuses the attach with 103 `entry:credential:<ref>` or
  `unsupported:credential:<ref>`."
- §4: replace "The credential vsock proxy remains RESERVED: port4041 terminates at the membrane
  and proxies to the controller; custody state stays kernel-core." (`001:834-835`) with "No guest
  port carries a credential or a request for one (spec 032), and port 4041 stays unused. Custody
  state stays kernel-core: the journal records each credential binding, and only the cell's
  `membraned` reads a value, at apply."
- §4.1, the configuration: after "in which `stop_deadline_ms` is optional. Recovery never reads
  it." (`001:868-869`), add "The same configuration admits optional `credential_authority:
  {certificate: Artifact, key, roots}`, spec 032's instance authority, in which `key` is a
  canonical absolute path and `roots` an Artifact or null. Recovery never reads the key."
- §4.1, `membrane.cell.observe`: in "Its assembled logical result is" (`001:918`), add
  `credentials` after `gates`. After "its idle deadline and whether its gate has fenced (spec
  028)." (`001:923`), add "`credentials` lists each spec 032 binding the supervisor holds, by
  reference, `GrantId`, mode and surrogate, never a value or a source path. Like `gates`, it is
  read fresh at each call."
- §4.1: in "`membrane.effect.apply` params are" (`001:953`), add `credentials` after `lifetime`
  in the params object. After "work at the bound plus D (spec 028 section 4)." (`001:957`), add
  "`credentials` lists the spec 032 bindings the operation's holding serves, each with its
  reference, mode, header, scheme, path scope, source and surrogate, and never a value. It is `[]`
  for a holding that serves none."
- §4.2: after "Workload edits of resolver configuration grant no bypass." (`001:1428-1429`), add
  "On a flow whose selected grant is the holding of a spec 032 `inject` binding, the host relay
  terminates the flow's TLS and attaches the value as spec 032 §5 says; every other flow stays a
  relayed stream."

**Spec 008** (words read at main `54f24f8b`).

- Journal records: in "`"provides": [STRING, ...], "effects": [RecordedEffect, ...] }`"
  (`008:103`), add `"credentials": [RecordedCredential, ...]` after `"provides": [STRING, ...]`.
  After "Null removes the attachment." (`008:105-106`), add "`RecordedCredential` is spec 032
  §2's credential binding: its reference, the `GrantId` of the ProxyMap holding it serves and that
  holding's target, its mode, header, scheme, path scope and source path, and its surrogate for
  `inject`; never a value. `credentials` is required, is
  `[]` for an attachment that binds none, and is sorted by reference, then by `GrantId`. A
  reload's replacement carries each kept binding's surrogate. Like `provides`, it is part of the
  per-cell format1 before its first writer ships, so no journal written without it exists."
- Desired state: in "`provides: Vec<String>, generation: u64, effects: Vec<RecordedEffect> }`."
  (`008:442`), add `credentials: Vec<RecordedCredential>` after `provides: Vec<String>`. After
  "requirements with their providers, provided keys," (`008:455`), add "credential bindings with
  their surrogates,".
- Recovery: after "Stray verified holdings still appear in exact drift;" (`008:497`), add "an
  observed spec 032 credential binding matches the journaled binding of a standing holding by
  reference, `GrantId` and surrogate, and one listed for a holding the journal does not record is
  stray drift. A holding's withdrawal is complete only once the observation lists none of its
  bindings."

**Spec 017** (words read at main `54f24f8b`).

- Keep "Credential material must not be placed in contents, arguments or recipes."
  (`017:130-131`), and replace "Existing credential references and core custody remain
  separate." (`017:131`) with "Credential bindings belong to the attachment (spec 032, spec 008),
  never to a recipe."
- ProxyMap adapter: after "Inspect effective relay rules, actual flow sockets and guest route/TUN
  state." (`017:403`), add "On a flow whose selected grant is the holding of a spec 032 `inject`
  binding, the relay terminates the flow's TLS and attaches the value only to requests it writes
  and sends to the recorded destination; every other flow stays a stream it never reads."

**Spec 023** (words read at main `54f24f8b`).

- §4, the config record: replace the whole sentence "It has exactly these keys: `cell`,
  `instance_root`, `runtime_recipe` (the template plus the three per-cell paths) and
  `boot_deadline_ms` (the remaining start budget at spawn)." (`023:182-184`) with "It has exactly
  these keys: `cell`, `instance_root`, `runtime_recipe` (the template plus the three per-cell
  paths), `boot_deadline_ms` (the remaining start budget at spawn) and `credential_authority`
  (spec 032's instance authority, as the paths of its certificate, key and roots, never the key
  itself; null when the controller's configuration has none)."

## Open questions

Each has the default this spec uses until the owner answers.

1. **Q1. `[model]`, carrying spec 022's question 2.** Spec 022 asked whether a model-provider
   plasmid should stay unattachable until custody exists. Default: `[model]` stays 103
   `unsupported:credential:<ref>`, and a model key is a `[secrets]` reference with consumer
   `process` and a `path_scope`, so a model-provider plasmid written that way attaches. The
   alternative gives `[model]` a consumer and a scope, which is a spec 011 grammar change.
2. **Q2. Where a value is kept.** Default: one file per credential, under §1's checks. The macOS
   Keychain is not a source yet: it needs the user's session unlocked, and a framework call in the
   controller and in each `membraned`. The alternative adds a Keychain source to §1's `source`, so
   both readers make that call, and a locked session refuses the read.
3. **Q3. Intercepting a workload's TLS.** Default: yes, only for the flows of an attachment that
   binds an `inject` credential at their target, with an instance authority the operator adds to
   the root image. Many runtimes ignore the system trust store. Node uses its bundled roots unless
   `NODE_EXTRA_CA_CERTS` is set, Python's `requests` uses `certifi` unless `REQUESTS_CA_BUNDLE` is
   set, and a Rust binary built with `webpki-roots` has no switch at all. Such a client fails its
   handshake until the operator points it at the store, and some cannot be pointed. The
   HTTP/1.1-only offer is visible to a client too. The alternative is no workload path at all, so
   a harness's own key stays outside custody.
4. **Q4. A value in the guest.** Default: never; a consumer that needs one is refused. The
   alternative is an operator opt-in per entry, shown in status, readable in the guest, and not
   revoked by detach.
5. **Q5. Registry members.** Default: an entry serves a registry member only when its
   `publishers` names the member's registry and publisher, since a publisher's name is unique only
   within one registry (spec 020). Is a bare publisher name ever enough, for example when the
   controller knows one registry only? A yes changes harm guard G13 and item 5, and has to say
   what such an entry serves once a second registry is configured.
6. **Q6. `helper` and `mint`.** Default: 103 `unsupported:credential:<ref>` until a consumer needs
   one. A minted value then expires at its issuer by B (§7). The alternative realizes one now,
   and either one puts a value in the cell (§7, Q4).
7. **Q7. Reflection.** Default: on a request that carries a value, ask for `identity`, refuse a
   response that is still encoded or is a 206, drop trailers, and scrub exact copies across chunks.
   A copy the host transforms is not defended. The alternative decodes compressed bodies to scrub
   them, which puts a decompressor on hostile bytes inside the relay.
8. **Q8. The surrogate's shape.** Default: the entry's `surrogate_prefix`, then random characters
   from `A-Z`, `a-z` and `0-9`: at least 22 of them, which carry at least 128 bits, and more up to
   the value's length at resolution. Its length tells the guest the value's length, unless the
   minimum makes it longer. The alternative is one fixed length, which hides the value's length
   from status and the guest; scrubbing then writes the run of `x`, which still shows it.

## Acceptance

Each item names its level, the guards it proves, and the broken custody it catches. The
**marker** is a distinctive value written into a secret file, such as `mk-` followed by 32 random
hex digits. A search for a marker looks for its plain text and for its base64, base64url,
percent-encoded and hex forms, and its SHA-256 in hex, so a copy in another encoding is found too.

1. **A covered reference becomes a binding (model; G2, G10).** A `wasm` reference, with an entry
   at the plasmid's declared `api.github.com:443`: the attachment's journal record holds the
   binding, with its reference, target, mode `handle`, header and source, and the ProxyMap recipe
   is unchanged. The marker is never found in the journal, the session log, any reply, status or
   refusal text. Catches: a value copied into a record; a binding in a recipe; a binding at a
   target the plasmid does not hold.
2. **What spec 022's 103 becomes (model; G8).** An uncovered reference is 103
   `entry:credential:<ref>` naming the plasmid. `git` with `["helper"]` and with
   `["helper", "mint"]`, and an unscoped `process` reference, are 103
   `unsupported:credential:<ref>`. `["mint", "inject"]` with a path scope binds with mode
   `inject`. `[model]` with no `credential` key is `unsupported:credential:model-provider/key`.
   Without `credential_authority`, an `inject` reference is `unsupported:credential:<ref>`. None
   has a `fix`, and none leaves a journal record. Catches: a mode the author did not list; an
   unrealized mode bound; a wrong string; a partial attach.
3. **The operator file cannot add reach (model; G10).** An entry for a reference the declaration
   does not name, and an entry at a host the plasmid does not declare, are each 108 with `path`
   the operator file and a `detail` naming the reference and target. Each of these is 108
   `content`: a `credentials` key in a version 1 file; two entries with the same reference and
   target; `transport: udp`; a `header` of `host`, `content-length` or `transfer-encoding`; and a
   17-character `surrogate_prefix`. Catches: an operator file that widens what a declaration asks
   for; an entry that binds a framing field, or a binding that can never apply.
4. **The secret file's checks (model, real files on Darwin; G1, G2, G12).** Each is 108 with
   `path` the secret file and `detail` the check: mode 0640, and 0600 with an
   `everyone allow read` ACL (`mode`); a symlink (`path`); a group-writable ancestor
   (`ancestor`); a second link (`links`); a FIFO, refused within 1 s (`not_regular`); 65,537 bytes
   (`size`); a CR, an inner LF, a NUL, a byte above 0x7E and an empty file (`content`); a file
   inside a held Mount source (`inside_mount_source`). Where the test host can make them, a parent
   directory and a file that another user (uid N) owns are `parent` and `owner`; otherwise those
   rows are reported not run. With a binding standing, a later plasmid attached to another cell
   is 108 with `path` the operator file, and nothing is prepared, when its Mount source is the
   secrets directory, lies inside it (a subdirectory), or is an ancestor of it: one row each. The
   three rows run twice: once while the operator file names the secret file, and once after the
   operator file stops naming it, with the binding still standing. The marker never appears in
   any refusal. A value with one trailing LF is sent without it. Catches: a readable secret
   trusted; header injection through CR or LF; secret bytes echoed in a refusal; a trailing
   newline sent; a later Mount that puts a bound secret file in a guest; a Mount guard that reads
   only the operator file.
5. **Not keyed by ID alone (model; G13).** A registry member with the local entry's ID is 103
   `entry:credential:<ref>` when the entry has no `publishers`. It binds when the entry names its
   registry and publisher. It is 103 for another publisher, and for the same publisher name in
   another registry. Catches: a credential keyed by plasmid ID, or by a bare publisher name.
6. **Reload (model; G9).** A reload that keeps the binding keeps its surrogate. A new reference, a
   held reference at a new target, a held reference at a new mode, and a new `path_scope` entry
   are each 109 with nothing prepared. A changed source proceeds, and the replacement's apply reads
   the new file. Catches: a reload that adds a credential; one that changes a running harness's
   surrogate; refusing a rotation.
7. **Recovery (model; G3).** Attach, delete the operator file and the secret file, and restart the
   controller: recovery holds the binding and the surrogate from the journal, and opens neither
   file. Catches: recovery that reads the operator file; a surrogate drawn again at restart.
8. **No interface returns a value (model; G2, G7).** `compile_fail` doctests show the custody type
   has no Debug, Display, Clone or Serialize, and no conversion to a string or to bytes. No
   control verb or guest lane method in the method tables returns one. `Gatekeeper::get` and the
   `AK_SECRET_*` stub are gone, and a `source` naming anything but a file is refused. Catches: a
   `get` that returns the value; a value printed by `{:?}`; values from the inherited environment.
9. **The component path (membrane; G4, G6).** With a fake component host, and a fixture TLS server
   for H trusted through test roots: a request naming R to H arrives with the value in the
   binding's header, and a request naming nothing arrives with no value, on another upstream
   connection than any request naming R, whichever is sent first. These are each refused
   before any byte reaches a fixture: a request naming R to a declared but unbound H2; one naming
   an undeclared R2; and one naming R that adds its own `Host`, `Content-Length`,
   `Transfer-Encoding` or `Connection`. Only R2 yields §8's denial, and its `fix`, written into
   the declaration, parses. A request to a host no grant holds yields the `network.hosts` denial.
   Catches: attaching unasked; attaching at an unbound host; a component's `Host` sent to a shared
   front; an attached request behind an unchecked one on one connection; a denial for a line the
   author cannot write; a `fix` that does not parse.
10. **The workload path's substitution rules (membrane; G4).** With a test client that trusts a
    test authority, the surrogate in the bound header, `Host` H and a path inside the
    `path_scope`, the fixture receives the value. The fixture receives the surrogate, and no value,
    for a path outside the scope; for the surrogate in the URL or in the body; for another
    attachment's surrogate; and on a flow whose selected grant is another attachment's holding at
    the same target. Catches: substitution outside the scope; substitution in a body; use across
    attachments; a value on another attachment's flow.
11. **The upstream (membrane; G4).** A fixture whose certificate no trusted root signed: the TLS
    connection fails, and the fixture receives no request byte. A fixture that answers 302 to
    another host: the client receives the 302, and the relay sends nothing else. Catches: an
    unverified upstream; a followed redirect carrying the value.
12. **Reflection (membrane; G1, G8, G12).** A fixture echoes the request's headers and body, and
    puts the value in its reason phrase, in a field name and in a 103 response's fields. The
    client, and on the component path the fake component host, receive the surrogate or the
    placeholder where the value was, in each of those places. The value is 40 characters, split
    across two transfer-coding chunks, with a chunk-size line between the halves, at every offset.
    The fixture receives `Accept-Encoding: identity` and no `TE`, `Range` or `If-Range`, whatever
    the client sent. A fixture that answers with gzip anyway, one that answers with
    `Transfer-Encoding: gzip, chunked`, and one that answers 206 each get the client's connection
    closed, and the component's request fails, with no byte of the body passed on. The fixture's
    trailers never reach the client. After each chunk, the relay releases the same number of bytes
    whether the chunk's tail matches a prefix of the value or not, so a scrubber that holds back
    only on a prefix match fails this item. Catches: a reflection into the guest; a compressed or
    ranged echo; a scrubber that misses a split copy; a hold-back whose timing tells the value.
13. **Refuse rather than expose, and read nothing else (membrane; G8, G11).** A client that does
    not trust the test authority fails its handshake with the ordinary unknown-authority error,
    and the fixture receives nothing. A flow to a target with no binding, and a flow of an
    attachment that binds nothing at a bound target, are relayed as streams: each sees the
    fixture's own chain, and bytes that are not HTTP pass unchanged. Catches: a fallback to plain
    text or to delivery; intercepting a flow, or a peer's flow, that carries no credential.
14. **The fence and withdrawal (membrane and model; G5, G9).** At B, a request with the surrogate
    is refused as spec 024 §9 says, and the fixture receives nothing. After withdrawal,
    `membrane.cell.observe` lists no binding for the attachment, and the withdrawal does not finish
    while it still lists one. In the model, recovery reports a binding listed for a holding the
    journal does not record as stray drift. A new attachment of the same plasmid gets a different
    surrogate, and the old one is never substituted again. Catches: attaching after the fence; a
    value kept after withdrawal; a binding that outlives its holding unseen; a surrogate reused.
15. **Status (model; G2, G7).** `plasmid.list` and `cell.status` show `credentials` for each
    plasmid, with reference, target, mode, and the surrogate for `inject`. The marker and the
    source path appear nowhere. Catches: a value or source in status; a surrogate the operator
    cannot find.
16. **Hostile requests (membrane; G4, G12).** Each of these is refused before a byte reaches the
    fixture, and the connection closed: both `Content-Length` and `Transfer-Encoding`; two
    `Content-Length` values; `Transfer-Encoding: gzip, chunked`; an obs-fold; a space before a
    colon; a bare LF; a header line, and a head, over the bound; two `Host` fields; a `Host` with
    another port; an absolute-form target with another authority; the bound header twice; and the
    paths `..;`, a raw `\`, `%2F`, `%2e`, `%25` and `%c0%ae`. An absolute-form target with the
    bound authority arrives in origin-form. The fixture receives each admitted request in the
    relay's own form, not the client's bytes, and two attachments' flows to one destination arrive
    on different upstream connections. Fuzz targets for the TLS server, the request parser, the
    response parser and the scrubber exist, and their corpus replays under `cargo test`. Catches:
    a second request smuggled past the rules; a second `Host` routed to another site; a path
    trick; raw bytes forwarded; a shared upstream connection; an unbounded parser.
17. **No value in a real cell (pinned runtime; G1, G11).** In a booted cell,
    `env KEY=<surrogate> curl https://H/<scoped path>` reaches a fixture of H that receives the
    value. A scan of the guest's files, every process's environment and arguments, and
    `mountinfo` finds no copy of the marker. Reported not run until O-1, O-6, O-7 and O-11.
    Catches: a value staged in the guest by any path.
18. **The instance authority (membrane and model; G1, G14).** The marker here is the test
    authority's private key as DER bytes, searched raw and in each form the preamble names, so its
    base64 form is the PEM body without line breaks. It is never found in the journal, the session
    log, any
    reply, status or refusal text, nor in any byte the fake component host or the fake guest
    bridge receives. A client that connects to a declared host with no `inject` binding gets no
    certificate from the instance authority: it sees the fixture's own chain. A leaf the relay
    presents for a bound target expires no later than the attachment's B. A key file that fails a
    §1 file check, rule 5 included, is 108 with `path` the key file. A `certificate` file that
    also holds a PEM private key refuses startup. A later plasmid whose Mount
    source is the key's directory, lies inside it (a subdirectory), or is an ancestor of it,
    attached to another cell, is 108 with `path` the operator file, and nothing is prepared: one
    row each. Catches: an authority key in a record or the guest; interception of a flow with no
    binding; a leaf that outlives its attachment; an unchecked key file; a Mount that exposes the
    key; a certificate file that carries the key into the image.
19. **What the controller sends `membraned` (model; G2, G3, G14).** The fake backend records every
    message the controller sends a `membraned`: each `membrane.effect.apply`'s params, the launch
    record, and the argv, environment and files of each launch. Neither marker, the value's or the
    key's, is in any of them, and the apply params name the secret file only by its path.
    Catches: a controller that reads the value and sends it in the apply params; a key sent at
    launch.
20. **What `membraned` writes (membrane; G2, G14).** Both markers are set: the value's in the
    secret file, and the key's as item 18 defines it. After a substitution on each path, after a
    leaf is issued with that key, and after each refusal of §5, `membraned.log`, every log
    `membraned` writes, every reply on `membrane.uds`, and every byte the fake component host and
    the fake guest bridge receive hold neither marker. Catches: a relay that logs the request it
    sent, or a parse error that quotes it; a relay that logs the key when it signs a leaf; an error
    to a component that quotes the request the relay sent.
21. **No surrogate, no value (membrane; G6).** On an intercepted flow, a request without the
    binding's header, one whose header holds anything but this binding's surrogate, and one with
    the surrogate only in another field each reach the fixture with no value. Catches: a relay
    that adds the value to a request that did not present the surrogate.
22. **A shared leaf (membrane; G14).** Two attachments bind `inject` credentials at one target,
    with leases ending at different times. The leaf's notAfter is the earlier `not_after`. After
    the earlier binding fences, the next handshake gets a leaf whose notAfter is the later bound.
    After `plasmid.renew` moves a bound later, the next handshake gets a later notAfter. When an
    idle limit ends an attachment before its leaf's notAfter, nothing is attached from then. When
    no binding at the target admits, a drain that never finishes holds no leaf key, and new flows
    there are not intercepted. Catches: a leaf with the latest bound; no new leaf after a renewal;
    a leaf trusted to end use; a leaf key kept through a stuck drain.
23. **Out of scope on the component path (membrane; G4).** A request naming R to H, on a path
    outside R's `path_scope`, is refused before any byte reaches the fixture. It is reported with
    `field` R's `scope.path_scope` and `fix` the path. Catches: an out-of-scope request sent with
    the value, or sent without it.
24. **The checks at apply (membrane; G3).** Between resolution and apply, the secret file is
    replaced by a symlink, a FIFO and a 0644 file, and, where the test host can make one, a file
    another user (uid N) owns. Each apply is 108 with `path` the secret file and `detail` the
    check, and no request carries a value. Catches: a `membraned` that follows a link at apply, or
    accepts a file whose owner or mode is wrong.
25. **Mock modes (membrane; G4).** With the holding at `simulate`, the operator's mock fixture
    receives the surrogate, or for a component no credential, and never the value. At `capture`,
    the capture file holds no marker. Catches: the owner's live value sent to a mock or written to
    a capture file.
26. **The surrogate (model; G9).** A fake random source first returns a surrogate that a journal
    of the instance holds: the controller draws again, and the attachment's surrogate differs from
    every journaled one. With a 16-character prefix and a 20-character value, the random part
    still has at least 22 characters, and scrubbing writes the run of `x`. Catches: a reused
    surrogate; a surrogate with almost no random part.

## Out of scope

- `helper` and `mint`, and any credential issuer (Q6).
- The macOS Keychain as a source (Q2).
- Basic authentication, and HTTP/2 towards the guest client (C2, C3).
- An audit of each credential use. Intent 012 asks to see what a cell was given, not each use.
- What an admitted request may do at the bound host, such as its method or its body.
- A Linux host.
- How spec 024, or a later spec, might let `cell.exec` set environment variables.
