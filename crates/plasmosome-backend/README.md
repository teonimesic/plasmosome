# plasmosome-backend

The enforcement seam. One interface between *deciding* a capability and *making it real*. The
crate also holds vocabulary that guest, membrane and controller share: the SHA-256 `Digest`, spec
001's observation pages and spec 027's pinned software trees, in the `digest`, `paging` and
`software` modules, and spec 028's lifetime values and the clock they are read on, in `lifetime`
and `clock`.

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
each record here and in the ledger. `ObservationPage` refuses it through its own visitor until
plasmosome-8pr4 moves it onto `ObjectOnly`. Why the crate names no VMM or broker process is
covered in `AGENTS.md`.

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
`validate` refuses; neither reads the filesystem or resolves a name. `Capability` does not carry
these records yet.

A `Capability` checks its own strings with the same rules. `SessionFile.path`, `UdsSocket.path`,
`Mount.source` and `Mount.target` are canonical absolute paths; `host`, `route` and `name` are
exact selection names and only need to be NUL-free. A `UniverseOp` checks the capability it
creates. Both refuse an invalid value on decode and on encode, so every record that carries one
does too, and an invalid capability can be neither read from nor written to a log.
The trait's `apply` and `plant` refuse an invalid value before any change, with
`BackendError::InvalidOperation` naming the address and the broken rule; `FakeBackend` does.
`grant` does not check yet.

A software tree is a plasmid's files, pinned before attach has any effect. `software::pin`
reads a directory against every file the verified package lists, as spec 020's
`{path, digest, size}`. Each listed path must pass spec 020 §2's file path rule: relative, with
nonempty components that are neither `.` nor `..`, and no backslash, ASCII control character or
drive prefix such as `C:`. So `foo..bar` is a legal name and `../foo` is not. Paths must also be
unique, under ASCII case folding too, and none may be another's ancestor. `pin` then refuses,
naming the path, an entry it cannot read, a FIFO, socket or device, a symbolic link, a size or
digest mismatch, a listed file that is missing, a file or empty directory the listing does not
imply, and a command that names no file at the top of the tree. A size that differs is refused
before any byte is read. Each file is opened once, checked to be regular through that
descriptor, and hashed from it, and nothing is written. A refusal's path is relative to the
tree's root, and core's 108 carries that path and names the plasmid in its `detail`. A name that
is not UTF-8 is refused as unlisted. On an APFS volume that refusal cannot happen, because APFS
refuses such a name when the file is created (`EILSEQ`).

What `pin` guarantees about links differs by system. On both, a root that is itself a link is
refused. The root's own path may pass through a link, as macOS's `/var` does, so the root is
resolved once to its canonical path. That path is then opened without following a link, and it
must be the directory the first check saw, by device and inode. So a root swapped between the
check and the open, for a link or for anything else, is refused as `Link { path: "." }`, and
the open does not wait on a FIFO. Every directory is opened before it is listed, and every file
before it is read.

- **macOS.** Every open uses `O_NOFOLLOW_ANY`, which refuses a link in any component of the
  canonical path. So no file is read, and no directory opened, through a link, even one swapped
  in while `pin` runs, at the root or below it. A directory is listed by path after its open, so
  a directory swapped for a link in between is listed through, but nothing under it is opened.
- **Linux.** Every open uses `O_NOFOLLOW`, which refuses a link only as the last component. A
  directory, the root included, swapped for a link after its open is followed by later opens, so
  a file outside the root can be read. It is pinned only when its bytes match the listing.
  plasmosome-0v4o walks by directory descriptors instead.

The tree digest is the SHA-256 of every file sorted by path bytes, each as its path, a NUL, its
digest's 64 lowercase hexadecimal digits, a NUL, and a byte that is 1 for a command and 0
otherwise. That encoding is this crate's, not spec 027's; plasmosome-tbru specifies the journal
record and may adopt it.

`software::check_names` refuses a command name another attached plasmid's pinned tree provides,
or one the root image has in `/usr/local/bin`, `/usr/bin` or `/bin`. It checks the tree's own
names first, then other plasmids', then the image's. The caller says whether the volume holding
the names ignores case. If it does, two commands of the same tree that fold together are refused
too. A pair that ASCII folding joins, such as `Foo` and `foo`, never gets that far: `pin` refuses
it first as `Collides`. So this 102 is reached only by a pair that ASCII folding does not join,
such as `Éclair` and `éclair`.
The fold is Unicode lowercase only. It misses names that differ only in normalization, such
as NFC and NFD `café`, and names that only full case folding joins, such as `ß` and `SS` or `ﬁ`
and `fi`. Default APFS treats each of those pairs as one name.

Spec 027 leaves the declaration field for commands blocked on consumers, so a `CommandName` comes
only from `software::fixture::declare_commands`, or back from `PinnedTree::command_names`. The
fixture compiles for this crate's tests and under the `test-fixture` feature. Enable that
feature only from `[dev-dependencies]`. The workspace guard
`only_the_testkit_enables_the_backend_test_fixture` fails the build when any other crate turns it
on by a normal or build dependency, for any target, unless the edge comes through
`plasmosome-testkit`, which is itself dev-only.
Core maps a `Conflict` to code 102 and a `PinRefusal` to code 108. The 102 does not carry spec
027's `fix` yet, and neither does the 108 for `CommandNotInTree`, because the declaration field
they would name does not exist. plasmosome-tbru adds both. No product path reaches
`CommandNotInTree` today, since no product tree declares a command. The other 108s are faults in
the files, not in a declaration, so under spec 001 they carry no `fix`.

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
| `software` | Spec 027's software tree before attach: `pin` checks a directory against the verified package's paths, sizes and digests, and `check_names` decides the 102 name conflicts. Nothing is placed here |

Tests: `cargo test -p plasmosome-backend`
