# plasmosome-backend

The enforcement seam. One interface between *deciding* a capability and *making it real*.

The controller says "this cell may reach api.example.com". Something must then edit a proxy's
allowlist, mount a directory, or install a credential — and that something differs by platform
and by capability. This crate is the line between the two halves.

It exists so the kernel's logic can be tested without a virtual machine. The in-memory backend
records what *would* have been granted; real backends do it. Both satisfy the same interface, so
lifecycle correctness, transaction rollback, and residue verification are tested at full speed
and full coverage, then run unchanged against the real thing.

The seam also carries an honest distinction: some grants are **hot** (a proxy map entry can
appear mid-run) and some are **generation-bound** (a VM's memory size cannot). The interface
makes a backend say which, rather than letting callers assume.

The seam crosses a process boundary, so its state travels as serde data: every type in that
vocabulary implements `Serialize`/`Deserialize` and holds no shared memory. Every record decodes
only from a JSON object. serde's derived decoders also accept a positional array and fill fields
by position, so two paths of one type could swap unnoticed; `ObjectOnly` refuses that shape for
each record here and in the ledger. Why the crate names no VMM or broker process is covered in
`AGENTS.md`.

Each successful grant gets a canonical RFC 4122 UUID-v4 identity and an exact handle made from
its universe class and that identity. Equal capabilities remain separate holdings. Recorded
operations and planted observations keep caller-supplied identities, while identity conflicts fail
without replacing state. `FakeBackend::plant_residue`, the trait's `plant`, and
`CompositeBackend::new` are fallible; the composite rejects initial observations assigned to the
wrong leaf.

Spec 017 gives every capability a complete, nonsecret recipe: what an adapter needs to create
the holding. The `recipe` module holds those records — `SessionFileRecipe`, `UdsRecipe`,
`ProxyRecipe`, `MountRecipe` and `BrokerLaunch`, with `FileAccess` and `ProxyTransport` — and
their structural rules: NUL-free strings, absolute paths with no empty, `.` or `..` component
and no trailing `/`, at most 65,536 bytes of file contents, a nonzero port, a destination that
is one DNS name or one IP literal spelled as Rust's `IpAddr` displays it and never in
IPv4-mapped or IPv4-compatible form (NAT64, SIIT and other prefixes that carry an IPv4 address
are left to the connect-time address policy), and a launch with an absolute program path and two
endpoints with different canonical spellings. A path that breaks these rules is refused, never
rewritten. Different spellings are not different files: a symlink, a hard link, a mount, or a
filesystem that ignores case or Unicode normalization, such as default APFS, can give one file
two names. Spec 017's preflight creates nothing, so it cannot compare endpoints that do not
exist yet; after binding both, the adapter compares the `(st_dev, st_ino)` that `lstat` reports
for the two paths, never `fstat` on the socket descriptors, and treats a match as a failed grant
under spec 017's incomplete-effect rules. Decoding refuses a missing, unknown or positional
field, and decoding JSON text also refuses a repeated field; a `serde_json::Value` or `Map` keeps only the
last copy of a repeated key. Decoding and encoding both refuse any value `validate` refuses;
neither reads the filesystem or resolves a name. `Capability` does not carry these records yet.

A `Capability` checks its own strings with the same rules. `SessionFile.path`, `UdsSocket.path`,
`Mount.source` and `Mount.target` are canonical absolute paths; `host`, `route` and `name` are
exact selection names and only need to be NUL-free. A `UniverseOp` checks the capability it
creates. Both refuse an invalid value on decode and on encode, so every record that carries one
does too, and an invalid capability can be neither read from nor written to a log.
`FakeBackend`'s `apply` and `plant` check before any change and return
`BackendError::InvalidOperation` naming the address and the broken rule. `grant` does not check
yet.

Every holding is owned by a `CellOwner { cell, plugin }`: the same plugin attached to two cells is
two owners, and no comparison looks at the plugin alone. It decodes only from a JSON object with
exactly those two fields. `CellId` lives here, and core uses this one type. An exact removal takes
the owner and a `DrainSpec`. It first resolves the holding at the removal's exact address with
that owner and full capability, and refuses anything else with `UnknownObject`. A graceful drain
that times out returns `DrainTimedOut` and keeps the holding, its issued record and every peer;
`Force` then withdraws only that holding. A successful removal of a granted holding, under either
policy, also retires its handle. A zero graceful deadline checks once: a drained holding is
released, an undrained one times out, and zero never forces. `revoke` follows the same rule.
`FakeBackend::mark_stuck` and `stall_graceful_drains_for_owner` make graceful withdrawals of one
address or one owner time out.

## What's inside

| Piece | Responsibility |
| --- | --- |
| `EnforcementBackend` | The trait: grant, revoke, and observe system state |
| `FakeBackend` | In-memory recorder — the test workhorse |
| `CompositeBackend` | Routes capability classes to the backend that owns them |
| Universe classes | What "system state" means for residue verification: sockets, mounts, processes, proxy entries, session files |
| `MockMode` | How a plasmid's calls are served: `simulate`, `capture` or `passthrough`, the default. It lives here, beside `CellId`, so the ledger can record a mode without depending on core |

Tests: `cargo test -p plasmosome-backend`
