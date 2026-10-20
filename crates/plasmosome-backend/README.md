# plasmosome-backend

The enforcement seam. One interface between *deciding* a capability and *making it real*. The
crate also holds vocabulary that guest, membrane and controller share: the SHA-256 `Digest` and
spec 001's observation pages, in the `digest` and `paging` modules, and spec 028's lifetime
values and the clock they are read on, in `lifetime` and `clock`.

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

The seam crosses a process boundary, so its state travels as serde data: every type in the seam's
vocabulary implements `Serialize`/`Deserialize` and holds no shared memory. In `paging`, only
`ObservationPage` is wire data. `FrozenAccount`, `CaptureSlot` and `PageAssembler` stay inside one
process, and `FrozenAccount` shares its bytes through an `Arc`. Every record decodes
only from a JSON object. serde's derived decoders also accept a positional array and fill fields
by position, so two paths of one type could swap unnoticed; `ObjectOnly` refuses that shape for
each record here and in the ledger, `ObservationPage` included. Every record also refuses a field
it does not name. Why the crate names no VMM or broker process is covered in `AGENTS.md`.

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
under spec 017's incomplete-effect rules. Decoding refuses a missing or unknown field and a
positional array, and decoding JSON text also refuses a repeated field; a `serde_json::Value` or
`Map` keeps only the last copy of a repeated key. Decoding and encoding both refuse any value
`validate` refuses; neither reads the filesystem or resolves a name.

Every `Capability` carries its complete recipe, and so does the `UniverseOp` that creates it:
`SessionFile`, `UdsSocket`, `ProxyMap` and `Mount` carry a `recipe`, and `Broker` carries a
`name` and a `launch`. A broker carries no process ID: a PID is observed after the broker
starts, never predicted. Its key is `broker/{name}`, so two brokers with one name and different
launches share a key and are still two different capabilities. The class keeps its name,
`BrokerPid`, and its label, `broker-pid`. A missing recipe or launch is refused on decode; no
code outside tests fills one in.

A `Capability` checks its own strings first, in field order, then its recipe or launch.
`SessionFile.path`, `UdsSocket.path`, `Mount.source` and `Mount.target` are canonical absolute
paths; `host`, `route` and `name` are exact selection names and only need to be NUL-free. A
`UniverseOp` checks the capability it creates, in the same order. Both refuse an invalid value on
decode and on encode, so every record that carries one does too, and an invalid capability can be
neither read from nor written to a log.
The trait's `apply` and `plant` refuse an invalid value before any change, with
`BackendError::InvalidOperation` naming the address and the broken rule; `FakeBackend` does.
`grant` does not check yet.

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
| `digest` | The SHA-256 `Digest`: exactly 64 lowercase hexadecimal digits, as text and in JSON. The membrane re-exports it |
| `lifetime` | Spec 028's lifetime values. A `Bound` is two readings: a UTC `WallMs` and a `Continuous` deadline tagged with its `BootSession`. It has passed at a reading when either of its two readings has, and a continuous deadline from another boot session is ignored. It keeps no state, so a caller that must keep it passed keeps a fence. Also the range checks for leases, idle limits and drain budgets, and RFC 3339 UTC text. These are not wire data yet: spec 008's journal record will give them their shape |
| `clock` | The `Clock` seam every lifetime decision reads time through: wall, continuous and wait readings, taken together. `ManualClock` lets a test move each one alone. The production clock is `SystemClock` in `plasmosome-core` |
| `paging` | Spec 001's observation pages, shared by guest, membrane and controller. On the responding side, a server keeps one `CaptureSlot` per connection. The slot holds the compact JSON account that the server's `collect` froze with `FrozenAccount::freeze`, and serves it in pages of 1 to 65,536 bytes until its deadline. On the requesting side, a caller's `PageAssembler` yields the account only when every page arrived in order from one capture and the bytes match the stated length and hash. `ObservationPage` decodes only from an object. Both sides pass the clock and own the IO |

Tests: `cargo test -p plasmosome-backend`
