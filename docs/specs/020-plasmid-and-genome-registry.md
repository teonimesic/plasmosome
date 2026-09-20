---
id: 020
title: A registry for curated and user plasmids and genomes
status: accepted
intents: [014]
---

## Behavior

A registry lets someone find a plasmid or genome, inspect its exact contents, bring it into a
project, and register one for other people to use. Both artifact kinds have curated and user
entries. Neither population is a different package format or a different execution path.
A genome preserves a working selection of plasmids, including their pinned providers and mock
choices; it is not a search query that assembles different parts next week.

Registration publishes immutable content under a publisher's name. The registry authenticates
that publisher, checks the package and its complete dependency graph, then makes the release
visible in one transaction. Curated registration additionally requires the registry operator's
curator authority. A user cannot acquire that authority by writing `curated` in a file, and
curation never grants a capability to a cell.

To use an entry, a host-side client resolves an explicit version, verifies every digest and
materializes the complete package graph. The ordinary controller then consumes that exact graph
through its existing attach or cell-creation path. Downloading successfully is reported as
`fetched`, not `attached` or `ready`. A registry does not replace the cell's admission checks,
credential custody, transactional attach, or operating-system enforcement.

### Scope and choice

The design is one authenticated HTTP service with a transactional catalog and content-addressed
files, plus a host-side client. There is no required hosting provider or production deployment.
The service runs on macOS and Linux; a test owns its listen address, database, object directory
and credentials. HTTPS protects off-host traffic. A configured loopback-only HTTP service is
available for local operation and deterministic tests, never as an automatic fallback.

A registry is not `plasmosome-core::ToolRegistry`. That existing structure maps attached tool
names to an owner and description; it neither distributes implementation files nor registers a
genome. Spec011 supplies the plasmid declaration, and explicitly leaves this registry to
intent014. This spec defines distribution identities, the genome distribution grammar, and the
exact handoff to the controller. It does not freeze the component WIT world or settle spec011's
owner-controlled approval gate.

## Contract

### 1. Identities and references

A service has a persistent `registry_id`, a non-nil UUID in canonical lower-case form. It is
created with its database, retained across backup/restore, and returned by every response.
A client profile binds one local alias to that ID and one explicit origin URL. Connecting to
another ID refuses, even if the origin URL or a local alias was reused. Registry ID is a namespace,
not authentication; the profile's transport and credentials authenticate the service.

A release address is `(registry_id, kind, population, publisher, name, version)`:

- `kind` is exactly `plasmid | genome`; `population` is exactly `curated | user`.
- `publisher` and `name` are ASCII lower-case identifiers matching
  `[a-z0-9]+(?:-[a-z0-9]+)*`, each at most 64 bytes.
- `version` is an exact, case-sensitive string matching `[A-Za-z0-9][A-Za-z0-9._-]*`, at most
  128 bytes. It is not a range, channel, or ordering instruction. The registry does not interpret
  `latest`, choose a newest release, or silently substitute another version.
- `digest` is `sha256:` followed by 64 lower-case hexadecimal digits. It hashes the exact
  descriptor bytes, not a reserialized equivalent. The version address becomes permanently bound
  to this digest on first successful registration.

A `ReleaseRef` carries `kind`, `population`, `publisher`, `name`, `version`, and `digest`.
The registry ID is carried by the enclosing package or operation. All dependencies belong to
that same registry. Cross-registry dependency resolution is refused; no package supplies a URL
that a server or client follows. Importing a graph into another registry is an explicit new
registration of its nodes, not a redirect or an automatic mirror.

The manifest's `id` and `version` must equal the release's `name` and `version`. These registry
restrictions do not change what an unregistered local declaration can parse. A registry address
is not a `PluginId`, `CellOwner`, `GrantId`, cell name, or credential reference. Runtime ownership
remains cell-qualified under decision003 and specs008/017. Two cells may use the same release;
its content digest is never substituted for either cell's ownership or fresh grant identities.

### 2. Packages and the genome file

A package is a UTF-8 `package.json` descriptor and the files it names. All JSON records below
reject duplicate keys, unknown fields, missing required fields, wrong types and invalid UTF-8.
There is no archive extraction, install hook, build script, executable post-processing step,
redirect, or remote URL inside a descriptor.

The descriptor has required fields:

| Field | Meaning |
| --- | --- |
| `schema` | Integer `1` |
| `registry_id` | The destination registry ID |
| `release` | `{kind, population, publisher, name, version}` without a digest |
| `declaration` | Relative path to the one plasmid or genome TOML file |
| `files` | Nonempty array of `{path, digest, size}`; SHA-256 hashes exact file bytes |
| `providers` | Array of `{capability, release: ReleaseRef}` for a plasmid; empty for a genome |

A file path is a nonempty relative slash-separated UTF-8 path. Each component is nonempty and
neither `.` nor `..`; backslash, NUL and ASCII control characters are forbidden. Leading slash,
drive-prefix syntax, symlinks, hard links, devices, sockets and directories as file entries are
refused. Paths are unique, including under ASCII case folding, and no path is another file's
ancestor. Files are ordinary non-executable bytes; their source permission bits are not shipped.
The client and server enforce these rules before reading or placing files. Reads and writes are
relative to owned directory handles without following symlinks, including intermediate entries;
checking a path and later reopening it by name is not sufficient.

There is exactly one `files` entry for `declaration`. Every package file is listed. A plasmid
uses the spec011 grammar, including purpose and tool descriptions. If `[impl].wasm` is present,
it names a listed package-relative file. The same rule applies to a local recorded-mock source:
all its ordinary files are listed beneath the declared relative directory. Absolute implementation
paths, out-of-package references and a missing component or recording refuse registration.
Credential references and declared hosts are declarations, not files to read or credentials to
resolve during registration. A declaration that legitimately needs no component remains valid.
No source compilation occurs in the registry.

For a plasmid, `providers` binds every distinct capability string in `[requires].capabilities`
exactly once to a pinned plasmid release. Extra, missing or repeated bindings refuse. The named
provider must declare that exact capability key in `[provides]`; a tool name is not a capability
key. This adds distribution bindings outside the manifest, not a new capability-range grammar.
Spec011's deferred range-selection question remains deferred: no version range is guessed.

A genome's declaration is a strict TOML document with required `id`, `version`, `description`
(nonblank string), and nonempty `[plasmids]`. Each key under `[plasmids]` is the referenced
plasmid's manifest ID; each value is a table with required `release: ReleaseRef` and optional
`mock`, exactly `simulate | capture | passthrough`. TOML expresses `release` as an inline table
or nested table with those six fields. It must name a plasmid, never another genome. No other
fields are accepted. For example, with a real digest in place of the explanatory token:

```toml
id = "researcher"
version = "1.0.0"
description = "Work on pull requests with recorded GitHub responses."

[plasmids.github-pr]
release = { kind = "plasmid", population = "curated", publisher = "plasmosome", name = "github-pr", version = "1.2.0", digest = "sha256:<64 lowercase hex digits>" }
mock = "simulate"
```

The example's explanatory digest is not a valid fixture. Published genomes contain real digests.
The runtime meaning of `[plasmids.X].mock` stays spec001's genome-default layer; omission means
no explicit declaration, not an explicit passthrough that defeats inheritance. A genome packages
no running cell, workspace contents, credential values, operator assertions or ledger history.
`cell.save` and `cell.load` remain outside this spec.

The full graph is all genome roots or the requested plasmid, then every transitive provider.
It is acyclic. Repeated identical references share a node. Two nodes with the same manifest ID
but different release references refuse the whole graph, even if their capabilities look alike;
the runtime's cell-local plasmid identity must not be made ambiguous. A curated release may
reference only curated releases. A user release may reference either population, with every
origin still visible. A user root never acquires a curated badge through a curated dependency.

Maximum accepted sizes are: descriptor and each declaration 1 MiB; each file 64 MiB; 128 files
and 256 MiB of file bytes per package; 256 distinct releases, depth 64 and 1 GiB of distinct file
bytes per complete graph. Limits are checked with overflow-safe arithmetic before allocation or
publication, and while streaming bytes. Limit refusal is not a truncated success. These are
format/service limits, not permission to allocate the entire graph in memory.

### 3. Authority and curation

The operator provisions principals outside the public API. A principal is a stable name with a
set of writable publisher namespaces and a boolean `curator`. Authentication uses random bearer
tokens of at least 256 bits, generated outside packages; the server stores their cryptographic
hashes and constant-time compares authenticated requests. Tokens have an explicit expiry and
revoked state. A token grants only the namespaces and curator flag in the current server record.
A client cannot submit its own principal, role or curation outcome in a request.

The strict JSON auth file is `{schema:1,tokens:[...]}`. Each token row has required `sha256`
(64 lower-case hex digits hashing the token's ASCII bytes), `principal` (a publisher-format
identifier), `publishers` (a nonempty unique array of publisher identifiers), `curator` (boolean),
`expires_at` (UTC RFC3339), and `revoked` (boolean). Tokens are 64 lower-case hex characters
representing 32 random bytes; the client sends those ASCII characters after `Bearer `. Duplicate
hashes refuse configuration. The operator atomically replaces the owner-only auth file to grant
or revoke authority. Every mutation authenticates against a fresh complete file before commit;
missing, unreadable or malformed auth state fails closed without publishing. Revocation committed
to that file before a release transaction's final authorization check takes effect on that write.

Read endpoints are public. User registration requires a current token authorized for the exact
publisher. Curated registration requires both that namespace permission and `curator=true`.
An ordinary publisher can register user releases without asking a curator. An authorized curator
can register a newly authored curated plasmid or genome directly, or deliberately copy an existing
user graph into curated releases. Copying uses new descriptors/digests with explicit references;
it never changes an existing entry's population in place. Curation is the operator's endorsement
of the selected graph, not a claim of code auditing, signature provenance, or safe cell authority.
The API creates no self-service curator role and no submission queue or implied owner approval.

The registration receipt records the authenticated principal, registration UTC time, exact
release address/digest and catalog generation. Its population is determined by authorized server
state, not an unverified manifest label. The same checks apply to either artifact kind. Tokens,
HTTP authorization headers and credential file contents never enter descriptors, catalog rows,
receipts, logs, errors, locks or cells. A publisher may put secrets into arbitrary file bytes;
the service does not claim that parsing proves their absence. Publication uses the explicit
inspect/expected-digest sequence in §5, without an interactive prompt or an implicit upload.

### 4. Registry service and HTTP API

The service is a separate host program, `plasmosome-registry serve --config PATH`, not a control
method that starts a daemon or widens a cell. Its strict TOML config requires `registry_id`,
absolute `database`, `objects` and `auth_file` paths, and `listen` as a numeric IP:port socket
address. Port zero is permitted for a private test; the actual bound origin is reported at startup.
Exactly one transport choice is required: absolute `tls_cert` and `tls_key` paths, or
`allow_loopback_http = true`. The latter accepts only an explicit loopback listen address;
no wildcard listener. The certificate is PEM and key is PKCS#8 PEM. Authentication records
live in the owner-only regular `auth_file`, not
in package storage. Off-host serving requires TLS; clients use normal certificate/hostname
validation and reject HTTP redirects rather than forwarding credentials to another origin.
There is no silent TLS downgrade. Startup validates private directories/config ownership and
refuses another writer using the same catalog. The service never reads the Git Beads store.
The server creates a missing private database/object directory from this config and binds its
registry ID on initialization; an existing database with another ID refuses. All storage/config
paths must be owned by the server account and not group/world-writable; the private key, auth
file and database are inaccessible to other accounts. A production reverse proxy may terminate
TLS only over a private loopback HTTP listener; clients still use the configured HTTPS origin.

Persistent catalog transactions use SQLite with foreign keys, full synchronous durability and
one writer. Object files use digest-derived names in the service-owned object directory; no
client path selects a storage pathname. Uploads go to unique private temporary files, are hashed
and synced, then installed without replacing a differing object. A release becomes visible only
after its descriptor, all files, and complete registered dependency graph are validated and
objects are durable. Its release row, references, receipt and incremented catalog generation
commit together. Readers see the old complete state or the new complete state, never half an entry.
Orphan temporary/unreferenced blobs are not releases; restart can remove them after excluding
active uploads. Committed content is retained, including yanked releases.

The API prefix is `/v1`. Objects use `application/octet-stream`; other request and response
bodies use `application/json`. Request bodies have explicit content lengths and the limits
above; a body that ends early, exceeds its length or exceeds its limit refuses. A service-wide
request deadline of 60 seconds bounds a request, including slow input; it is not reset by a byte.
Clients use the same deadline and do not turn a timeout into success. Large transfers are streamed.

| Request | Successful result |
| --- | --- |
| `GET /v1` | `{registry_id, schema:1}` |
| `PUT /v1/blobs/<digest>` | Authenticated upload; `201 {registry_id,digest,size}` or `200` for identical existing bytes |
| `POST /v1/releases` | Authenticated exact descriptor bytes; `201` with receipt, or `200` with original receipt for an identical retry |
| `GET /v1/releases/<kind>/<population>/<publisher>/<name>/<version>` | Release descriptor bytes, digest, receipt and current `active | yanked` state |
| `GET /v1/blobs/<digest>` | Exact bytes of a blob referenced by a committed release, with length and digest headers |
| `GET /v1/catalog?kind=...&population=...&query=...&limit=...` | Catalog page described below |
| `POST /v1/releases/<kind>/<population>/<publisher>/<name>/<version>/yank` | Authorized `{digest,reason}`; receipt of terminal yanked state |

The JSON release response encodes descriptor bytes as base64 in `descriptor`, so hashing does
not depend on a response serializer. A registration receipt is
`{registry_id,release:ReleaseRef,principal,registered_at,generation}`. Dates are UTC RFC3339.
Every JSON success includes `registry_id`; every blob response includes `Registry-Id` and
`Digest` headers, which the client verifies against the requested profile/digest. Arbitrary
uncommitted uploads are not readable through the public blob endpoint. Upload requires a valid
publishing principal, but upload alone neither owns a release name nor publishes content.

Registration checks namespace authority even for a retry. The same address and exact descriptor
returns its original receipt without a new generation; different descriptor bytes at that
address return `version_conflict`, regardless of author. Concurrent first registrations have one
winner. A response lost after commit is resolved by reading the exact address or repeating the
same authenticated POST; the client never invents another version. No force-overwrite operation
exists. Missing or yanked dependencies refuse registration. Those conditions and the graph check
are rechecked inside the publication transaction, not only during upload.

Yank requires the original population's namespace/curator authority. It retains content, creator,
receipt and digest, records the authenticated yanker and nonblank public reason, and increments
generation once. A matching repeated yank returns the original yank receipt; a different reason
is a conflict. There is no unyank, deletion or version reuse. Yanking neither detaches running
cells nor deletes already imported content. New online resolution refuses a yanked root or
provider. An already materialized graph remains an exact historical artifact, not proof that
its current catalog endorsement is unchanged. Publish a new version to distribute a correction.

Catalog search includes both kinds and populations unless exact filters are supplied. Query is
a case-sensitive substring of `name` or description; it does not execute expressions or model
instructions. Results sort by `(kind,population,publisher,name,version)` as UTF-8 bytes, with no
semantic version ranking. Each row contains the complete release reference, registry ID,
purpose, receipt and yank state/reason. Tool sentences and dependency refs are obtained by
`show`, not guessed from search text. Each page has `generation`, `items`, and optional
`next_cursor`; `limit` is 1–100, default 50. The first request captures a complete immutable
catalog snapshot; opaque cursors bind that snapshot and the original filters. It expires after
five minutes or server restart, and at most 32 snapshots are retained; exhaustion refuses a new
snapshot rather than evicting an active one. Expired/unknown cursors return `cursor_expired`.
Continuation uses `GET /v1/catalog?cursor=OPAQUE` with no other query fields. A cursor contains
no credential and grants no publishing authority; a client may retry the same cursor for the
same page while it remains live. The terminal page omits `next_cursor`.
A client follows all pages before reporting a complete search; failure is not an empty registry.

Errors are `{registry_id,error:{code,message}}`, with optional `field` or `release` inside error
only when that context exists. Missing fields are omitted, not null. The closed string codes are
`invalid_request`, `package_invalid`, `unauthenticated`, `forbidden`, `not_found`,
`version_conflict`, `digest_mismatch`, `dependency_unavailable`, `dependency_conflict`,
`cycle`, `limit_exceeded`, `yanked`, `cursor_expired`, `busy`, `io_failure` and `internal`.
They map respectively to HTTP400 for malformed/package input;401/403 for auth;404 for absence;
409 for conflicts, unavailable dependencies, cycles, yanks and cursor expiry;413 for limits;
503 for busy; and500 for storage/internal failure. `digest_mismatch` is400. Clients branch on
code, not message. Diagnostics identify the rejected reference/field without tokens or hidden
filesystem paths. A registry error is not a new code in spec001's controller error enum.

### 5. Host-side client, fetch and register

`plasmosome registry` is the common CLI for both kinds, outside a cell. It has these commands:

`ALIAS` uses the publisher/name grammar in §1, including its 64-byte limit. It is nonempty,
lower-case ASCII and compared exactly for uniqueness. Reject invalid aliases before profile IO;
`/`, `@`, `#`, whitespace and control characters cannot be profile delimiters or aliases.

- `source add ALIAS --url URL --registry-id UUID [--token-file PATH] [--allow-loopback-http]`
  stores an explicit profile; `source list` reports profiles with credentials omitted. Reusing an
  alias refuses; changing a source requires `source remove ALIAS` and re-addition. Removal drops
  only the profile, not imports or running cells. No artifact can write source configuration.
- `search --source ALIAS [--kind plasmid|genome] [--population curated|user] [--query TEXT]`
  obtains the complete catalog result. `show REF` obtains the descriptor, purpose, tools,
  complete exact graph, populations and yanks without executing anything.
- `inspect DIRECTORY --source ALIAS` checks the descriptor and every listed local file without
  upload, publication or network access. It displays the exact destination, descriptor digest
  and file list. Remote dependency availability and publishing authority remain unchecked.
- `register DIRECTORY --source ALIAS --expect-digest DIGEST` revalidates those bytes and refuses
  a changed descriptor or file before upload. It stages all verified bytes in private immutable
  snapshots before its first upload and uploads only those snapshots, not reread mutable inputs.
  The required digest is the one returned by inspect;
  there is no implicit confirmation, interactive prompt or noninteractive bypass flag. It then
  uploads missing content and registers the descriptor. The directory is required: no implicit
  current-directory upload. Only listed files are sent; the server checks authority and the
  complete dependency graph regardless of local inspection.
- `fetch REF --output DIRECTORY [--offline]` verifies and materializes the full graph. It never
  starts a controller, creates a cell, grants credentials, or runs package code.
- `yank REF --reason TEXT` uses the exact version and expected digest; no wildcard yanks.

`REF` is `ALIAS/KIND/POPULATION/PUBLISHER/NAME@VERSION`, optionally followed by `#sha256:HEX`.
Every artifact result includes its full resolved reference and registry ID as specified below.
Omitting a digest makes
one exact-version lookup, after which the returned digest pins the whole operation. Supplying one
requires an exact match. No command accepts an unqualified package name or searches a different
source after failure. These registry commands do not change the `plasmid new` contract.

All commands support `--json`, emit one complete success object on stdout, and exit0. In JSON
mode that strict envelope is `{schema:1,command:COMMAND,result:RESULT}`. `COMMAND` and `RESULT`
are fixed by this table; JSON object ordering is not significant:

| COMMAND | Required RESULT fields |
| --- | --- |
| `source.add` | `profile: Profile` |
| `source.list` | `profiles: [Profile,...]`, sorted by alias |
| `source.remove` | `alias` |
| `search` | `registry_id`, `generation`, `items: [CatalogEntry,...]` after all pages |
| `show` | `registry_id`, `root: ReleaseRef`, `packages: [PackageView,...]` |
| `inspect` | `destination: Profile`, `registry_id`, `release: ReleaseRef`, `files: [{path,digest,size},...]`, `dependency_state: "not_checked"` |
| `register` | `receipt: RegistrationReceipt` |
| `fetch` | `state: "fetched"`, `registry_id`, `root: ReleaseRef`, `releases: [ReleaseRef,...]`, `output`, `online_checked` |
| `import` | `state: "imported"`, `registry_id`, `root: ReleaseRef`, `releases: [ReleaseRef,...]`, `output` |
| `yank` | `registry_id`, `release: ReleaseRef`, `yank: YankInfo` |

`Profile` is `{alias,url,registry_id,has_token,allow_loopback_http}`; token paths/bytes are
absent. `RegistrationReceipt` is §4's receipt. `YankInfo` is
`{principal,reason,yanked_at,generation}`. `CatalogEntry` is
`{registry_id,release:ReleaseRef,description,receipt:RegistrationReceipt,state}` plus `yank`
exactly when state is yanked. `PackageView` is `{release:ReleaseRef,descriptor,description,
tools:[{name,description},...],receipt:RegistrationReceipt,state}` plus the same conditional yank;
`descriptor` is base64 exact descriptor bytes, and a genome's tools array is empty. The show
array contains the full graph sorted by digest, not just its root. These definitions also fix
the corresponding HTTP catalog row and release-response contents; the latter adds registry_id
to PackageView. Show may inspect a yanked graph, but never reports it usable. An unavailable
dependency still refuses a complete show. Inspect derives the full reference from its descriptor
and computed digest. File lists sort by path; graph lists sort by digest; outputs are absolute
local directory paths. Empty collections are arrays, never omitted or null.

Failure emits no stdout success or partial result. JSON stderr is one strict envelope
`{schema:1,command:COMMAND,error:{domain,code,message}}`, with optional `registry_id`, `release`,
`field` and `path` inside error only when known. Invalid command syntax uses COMMAND `invalid`.
Domain `registry` preserves §4's closed error code and supplied context. Domain `client` has
closed codes `invalid_argument`, `invalid_path`, `profile_exists`, `profile_missing`,
`profile_invalid`, `transport`, `tls`, `deadline`, `local_io`, `integrity_mismatch`, `cache_miss`,
`destination_exists`, `registry_identity_mismatch` and `publication_changed`. Client transport,
tls, deadline and local_io exit1; every other client code and every registry refusal exits2.
Paths appear only for caller-supplied inputs or owned residual staging, never credentials.
Human mode has the same completion/refusal semantics and escapes remote control characters.
Inspect is the complete preview; register requires its expected descriptor digest and never
prints a success before the server's receipt. No failure prints a success prefix.

Private client state defaults to `$HOME/.plasmosome/registry`, overridable with
`--registry-root PATH` for every command. Profiles and token files are owner-only regular files;
a token file is read only for its configured origin and never copied into the artifact cache.
HTTPS is required except an explicit loopback HTTP profile. Source URLs may not contain embedded
credentials or fragments.

A fetch's output directory must not exist. The client first validates the entire online graph's
release records, hashes and bounds, then stages all files in a private sibling directory. It
rechecks bytes as installed and publishes the directory with an atomic no-replace operation.
Cancellation, corruption, refusal or concurrent destination creation leaves that destination
unchanged; the client owns and removes only its own staging directory. No partial output is
reported as fetched. A failed cleanup reports its exact owned residual staging path as local IO.
The output contains `registry-lock.json` and `packages/HEX/descriptor.json` plus
`packages/HEX/files/PATH` for each listed file, where HEX is the descriptor digest without its
`sha256:` prefix. Metadata paths and payload paths cannot collide. Different packages cannot
overwrite each other. Declaration-relative implementation lookup in a registered package uses
that package's `files` root, regardless of where its declaration file is located beneath it.

The strict lock has `{schema:1,registry_id,root:ReleaseRef,releases:[ReleaseRef,...]}`; releases
are the full graph sorted by digest with no duplicate nodes. Each digest directory verifies
against its own descriptor and the root. The lock is an integrity/selection record, not an
approval token. All release records and file bytes are also stored in the private content cache
by digest after verification. An offline fetch requires a digest-qualified REF and a completely
cached verified graph; it rehashes bytes and reports `online_checked:false`. It never claims a
fresh yank check. Ordinary fetch never silently becomes offline. Missing/corrupt cache is an
error, not permission to download a different version. Online fetch reports the catalog state it
actually checked; a subsequent yank is not retrospectively invisible and is not live revocation.

### 6. Using an imported graph in a cell

The host imports a fetched directory through `plasmosome registry import DIRECTORY`. The
controller's trusted local `registry_root` is configured explicitly; the host import validates
the lock/descriptors/files again and installs the complete immutable graph under that root by
digest with atomic no-replace publication. This is a second verification boundary, not trust in
an arbitrary client-supplied path. The directory must be owned by the host operator and not be
writable by the workload. Imported files are read-only and non-executable to the workload; package
bytes are not an instruction to run a host executable. Import reports `imported` and the root
reference; it is not a successful cell operation. Existing equal imports are idempotent.
Import uses the explicitly selected client `--registry-root`; the intended controller must be
configured with that same trusted root. Its output lives at `imports/REGISTRY_UUID/ROOT_HEX`
and has the fetched-directory layout above. This whole root graph, not a sequence of independently
visible package inserts, is the atomically published unit. No server origin or workload-supplied
location overrides the controller config. Existing imports are reverified before reporting an
idempotent success; a conflicting/corrupt destination is an error, never overwritten.

This spec adds one optional `artifact` object to the requests of spec001 §3.10 `plasmid.add`
and §3.5 `cell.new`, and to their host CLI forms:

```text
plasmid add NAME --artifact REF --kernel KERNEL --cell CELL
plasmosome cell new --genome NAME --artifact REF --kernel KERNEL
```

On the wire, `artifact` is `{registry_id,release:ReleaseRef}` with a required digest. For
`plasmid.add` it must be a plasmid whose declared ID equals `plasmid`; for `cell.new` it must
be a genome whose ID equals `genome`, which is required when artifact is present. The host CLI
requires the graph already imported; it does not fetch inside the controller request. Requests
without `artifact` retain existing local selection semantics. Responses retain their existing
shapes; this spec adds no application error number. Reload keeps its request shape but its
source-selection semantics are pinned as described below.
Missing import is code101 with target naming the full reference; corrupt or mismatched imported
content is108 with detail/path. Existing closure, authority, widening and mock conflicts use
their existing codes. No malformed artifact falls back to local name resolution.

The controller resolves only the immutable imported graph, revalidates hashes before use, and
passes its exact manifests, implementation files, provider bindings and genome mock declarations
to the ordinary resolver. It never contacts the registry, executes an install script, substitutes
a same-name installed release, or lets a cell choose a host cache path. Mutable cache replacement
cannot change the bytes between verification and execution: retain opened immutable objects or
an equivalent verified owned copy for the lifetime of the prepared attachment. A corrupt cache
refuses; it is not repaired by an ambient network fetch.

For every attached member, spec008's sole per-cell journal records an AttachmentSource
`{kind:"registry",registry_id,root:ReleaseRef,member:ReleaseRef}` alongside mock and complete
effects. Root is the imported graph selected by this request; member is the exact plasmid
within it. For a reused shared provider, require the same registry ID/member and retain its
original root/source. A local provider or a different reference with the same manifest ID is
not interchangeable. No source side store or in-memory-only mapping substitutes for prepare,
commit/abort, finish and the complete DesiredPlasmid publication. Ordinary unregistered
attachments record `{kind:"local"}` without inventing registry provenance.

`plasmid.reload` reuses the attached member's recorded root graph and exact member, while its
normal mock override and generation-swap checks still apply. It accepts no new artifact parameter.
Use an explicit permitted detach/add or a new cell to select a different release. Reload cannot
consult a mutable version address or local namesake, even after restart. Missing/corrupt bytes
refuse before effects with101/108 and preserve the old attachment. A reused provider's own saved
source continues to govern that provider.

Recovery carries complete sources through spec008 replay, publication, comparison and controller
reconstruction. After recorded pending cleanup and before serving, the recovery orchestrator
validates every settled imported source and retains verified declaration/implementation objects;
it never fetches from a registry. Unavailable/corrupt source yields spec001's artifact_source
startup diagnostic without killing the surviving cell or guessing a replacement. Exact cleanup
uses recorded operations/inverses and does not require retiring source bytes. Imported graphs
are not automatically evicted or mutated; removing a profile never removes them. Source bytes
are content storage, not a second durable record of attachment ownership/generation.

A graph only supplies candidates. The ordinary trusted operator path, any applicable external
authoring approval, exact grant preparation, cell ownership and rollback checks still decide
whether it can take effect. Curator status, a digest, a lock or successful import never bypasses
one of those checks. This specifies no new approver or approval artifact and gives a cell no
ability to approve its own widening. If a required runtime/approval facility is unavailable,
attach refuses through the existing control contract rather than reporting a synthetic success.

An accepted imported plasmid becomes usable through the same tool resolution and enforcement
path as a local plasmid. An imported genome creates a cell from its full pinned closure and
preserved mock declarations. Conflicting same-ID graphs within one cell refuse before effects;
using one graph in two cells is allowed and has separate CellOwner/grant records. Detaching one
cell's plasmid must not withdraw the other cell's tools or grants. Recovery uses specs008/017's
durable complete operations, not a mutable registry lookup. Catalog updates, yanks and service
outages neither rewrite a live attachment nor constitute runtime revocation.

These are required integration behaviors, not statements that the current status-only controller
or a parsed declaration already implements them. Registry implementation may be planned with
runtime prerequisites, but registry **use** is not delivered merely by completing fetch/import.
A runtime or approval prerequisite that is still absent remains explicit incomplete acceptance;
no fake backend, no-op attach or documentation claim closes it.

## Acceptance

### Complete product matrix

Run one real service in private resources, an ordinary namespace publisher, an authorized curator,
and fresh host clients. For **each** of the following rows, register a new release, discover/show
it from a different client, fetch/import its verified graph, and exercise the indicated real
runtime use. The observed catalog population and authenticated publisher must match the row.
A user-created package is not treated as curated and a genome is not flattened into a catalog
entry for one of its members.

| Kind | Population | Register witness | Use witness |
| --- | --- | --- | --- |
| plasmid | curated | curator publishes a new complete package | attach it to a real cell and call its declared tool |
| plasmid | user | publisher publishes its own package without curator authority | attach it to a real cell and call its declared tool |
| genome | curated | curator publishes a genome with at least two pinned plasmids | create a real cell from it and exercise both plasmids with the declared mock mode |
| genome | user | publisher publishes its own two-plasmid genome | create a real cell from it and exercise both plasmids with the declared mock mode |

These four rows each require register and use: eight obligations, not four metadata insertions.
Use a component-free real capability/tool only if the runtime genuinely supports it; do not invent
a callable implementation to avoid the SDK/runtime prerequisite. OS-boundary proof uses a real
supported backend and workload process, not `ToolRegistry` alone.

### Identity, publication and authentication

- Two publishers may publish the same name/version in distinct namespaces. Fully qualified
  selection never chooses the wrong one; a mismatched registry ID or expected digest refuses.
- Ordinary publishers cannot write another namespace, register/yank curated entries, or turn a
  user row curated. Expired, revoked and missing tokens refuse without a catalog change. A
  curator without the requested namespace also refuses. Exercise both kinds.
- A repeated identical registration after a lost reply returns the original receipt/generation.
  Concurrent different publications to one version have one winner and one version_conflict;
  the losing package never appears in search. Restart after each persistence boundary never
  exposes a descriptor whose required bytes or dependencies are missing.
- A curated release referencing a user release refuses. A user genome with curated and user
  members retains both exact origins through show, lock and use. Registering a new version never
  changes a previously pinned genome.
- Yank is authenticated, terminal and retains exact content/history. New online resolution of a
  yanked root or provider refuses, but existing imported content and an already running cell are
  not deleted or silently detached. Offline output explicitly lacks a fresh catalog check.
- Reject invalid/ambiguous aliases before profile mutation. Inspect reports all upload bytes
  and the exact destination without contacting the service. Alter descriptor or file after
  inspect: register with its expected digest refuses without uploading or publishing a prefix.
  Exercise every command's closed JSON envelope, complete-result rule and typed failure exits,
  including a failed later search page and a missing-token refusal.

### Graph, bytes and filesystem boundaries

- A three-level provider graph resolves exactly once per digest. Missing/mismatched providers,
  cycles, repeated bindings, unsupported kinds, conflicting manifest IDs, wrong id/version,
  unknown/duplicate fields and every size/depth/count overflow refuse before publication/import.
- Changed descriptor bytes, a corrupted component and a truncated recording are each detected.
  The client cannot turn failed hash verification into a fetched/imported result. Registration
  never reads a secret reference as a host credential or follows a package-supplied URL.
- Traverse paths, absolute paths, drive prefixes, file/ancestor collisions, case-fold collisions,
  symlinks and intermediate symlink swaps cannot read or overwrite an outside sentinel. Failed
  or cancelled materialization and a racing existing destination preserve the destination.
- Test offline fetch with a complete pinned cache, a missing provider and a corrupted file.
  Only the complete verified graph succeeds; no case uses the network or changes the version.
- Catalog pagination remains on one captured generation while releases are registered/yanked.
  Expired/restarted cursors and failed later pages report errors, never a complete prefix.
  Human rendering cannot emit remote terminal control sequences; machine output preserves data.
- Off-host plaintext, TLS identity failures, redirect targets and source-ID substitution refuse.
  Request-size and slow-input deadlines are observed on real connections. Tokens are absent from
  captured public responses, ordinary logs, errors and imported files.

### Runtime and proof limits

- Compare the imported graph selected by the real controller with every digest in the fetched
  lock. A service outage or newer same-name release cannot change its provider selection.
  A missing/corrupt import fails without fallback and without a cell/grant/tool prefix.
- Restart with the service unavailable and competing same-name imports present: journal source,
  full desired publication and reload all retain the original exact member/provider graph.
  Missing/corrupt settled imports block serving without namesake fallback; loss of a retiring
  source does not block original-inverse cleanup. Source changes only with the journal's
  transaction decision; changing source alone at equal generation is a desired_conflict.
- Attach the same imported release to two cells. Exercise its actual boundary access, detach it
  from one, then prove the first is denied and the second still works with distinct cell owners.
  A same-ID different-release conflict within one cell refuses without disturbing the first.
- A denied attach uses the normal authority/widening refusal and leaves existing grants unchanged.
  Curated and user packages face the same check. Preserve spec001's mock propagation and
  spec011's complete-closure/no-provider-grant-inheritance requirements.
- Capture the successful register/search/fetch/import and real-use commands, exact digests,
  observed rollback/revocation results and platform in native implementation evidence. Repeat
  storage/transport/CLI cases on macOS and Linux. Real cell enforcement is required on every
  platform claimed supported; a platform without that result remains explicitly unsupported
  for registry use, not certified by the portable catalog suite.

## Delivery boundaries

This is a specification, not a registry deployment, endorsement of particular packages, or proof
that implementation exists. At the authoring base, manifest descriptions and ToolRegistry lookup
are implemented; neither provides catalog/authentication/content distribution, a genome parser,
or the real public attach/cell-new implementation required above. The implementation must own
those integrations or retain their explicit prerequisites; reaching catalog-only behavior does
not satisfy intent014.

Hosting choice, account signup, payments, popularity ranking, semantic-version ranges, automatic
updates, cross-registry federation, mutable tags, signatures beyond authenticated service receipts,
and a submission moderation queue are not required here. These exclusions do not remove either
artifact kind, either population, either register/use direction, or any security check above.
Curator provisioning is ordinary operator administration, not an agent deciding the project
owner's authoring approval gate. The existing spec011 gate question and component interface remain
outside this spec, and absent prerequisites cannot be disguised as successful runtime use.
