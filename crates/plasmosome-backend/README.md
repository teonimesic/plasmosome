# plasmosome-backend

The enforcement seam. One interface between *deciding* a capability and *making it real*. The
crate also holds vocabulary that guest, membrane and controller share: the SHA-256 `Digest`
(`digest`), spec 001's observation pages (`paging`) and guest bridge records (`guest_protocol`),
spec 027's pinned software trees (`software`), and spec 028's lifetime values and the clock they
are read on (`lifetime` and `clock`).

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

A grant keeps the identity its caller chose, as recorded operations and planted observations do.
Under spec 017 the caller draws a fresh UUID-v4 `GrantId`, builds the complete `UniverseOp` and
its inverse, records them durably, and only then calls `grant(operation, kind)`. Its handle is the
operation's class and that identity, so the caller knows it before anything happens; the backend
never chooses or changes it. `grant` can fail. A retry of the same standing issued operation and
kind returns the original entry and changes nothing. Another owner, capability or kind at an
issued address returns `IdentityConflict`, and so does a grant at the address of an object that
was applied or planted: a grant never turns such an object into an issued record. Equal
capabilities remain separate holdings, and identity conflicts fail without replacing state.
`FakeBackend::plant_residue`, the trait's `plant`, and `CompositeBackend::new` are fallible.

`snapshot_os_state` returns the backend's complete account as an `EnforcementSnapshot`: its
standing holdings, the original issued record of each grant whose withdrawal authority remains,
and its unfinished effects as `IncompleteEffect`s. An applied or planted object has no record, and
a record is never rebuilt from an object. `EnforcementSnapshot::new` first refuses a row whose
capability fails `validate` with `InvalidOperation`, and a record whose handle class is not its
capability's class with `Fault`. For a row that breaks one rule, encoding and decoding name that
rule too. For a row that breaks two, the three can name different ones: decoding checks a nested
recipe before the capability, encoding checks a record's class before its capability, and `new`
checks the capability first. `new` then refuses an address repeated in the unfinished
effects, an address both standing and unfinished, a repeated issued handle, and a record whose
owner or capability disagrees with its row, each with `IdentityConflict`. It refuses a record with
neither a standing nor an unfinished row with `Fault`. Decoding runs the same checks. A caller
deciding that nothing is left asks `is_empty`, which is true only when all three collections are
empty.

The snapshot can fail. An observation that fails returns an error, never an empty or partial
account. It takes `&mut self`, so a backend can keep what it observed.
`FakeBackend::fail_next_observation` makes the next snapshot return `Fault` once, with no other
effect. `CompositeBackend` observes all three leaves when it is built and on every snapshot. A
leaf whose observation fails makes the whole snapshot fail, and so does a leaf that reports a
standing object, issued record or unfinished effect of a class it does not own; that `Fault`
names the leaf and the row. `BackendError::Fault` displays as `backend fault: {cause}`, because
it reports real observation errors as well as injected faults.

Spec 017 gives every capability a complete, nonsecret recipe: what an adapter needs to create
the holding. The `recipe` module holds those records — `SessionFileRecipe`, `UdsRecipe`,
`ProxyRecipe`, `MountRecipe` and `BrokerLaunch`, with `FileAccess` and `ProxyTransport` — and
their structural rules: NUL-free strings, canonical absolute paths with no empty, `.` or `..`
component and no trailing `/`, at most 65,536 bytes of file contents, a nonzero port, a destination
that is one DNS name or one IP literal spelled as Rust's `IpAddr` displays it and never in
IPv4-mapped or IPv4-compatible form (NAT64, SIIT and other prefixes that carry an IPv4 address are
left to the connect-time address policy), and a launch with a canonical absolute program path and
two endpoints with different canonical spellings. A path that breaks these rules is refused, never
rewritten. Different spellings are not different files: a symlink, a hard link, a mount, or a
filesystem that ignores case or Unicode normalization, such as default APFS, can give one file
two names. Spec 017's preflight creates nothing, so it cannot compare endpoints that do not
exist yet; after binding both, the adapter compares the `(st_dev, st_ino)` that `lstat` reports
for the two paths, never `fstat` on the socket descriptors, and treats a match as a failed grant
under spec 017's incomplete-effect rules. Decoding refuses a missing or unknown field and a
positional array, and decoding JSON text also refuses a repeated field; a `serde_json::Value` or
`Map` keeps only the last copy of a repeated key. Decoding and encoding both refuse any value
`validate` refuses; neither reads the filesystem or resolves a name. Three of these rules are
public so a declaration is checked by the same code: `canonical_path` checks a path,
`is_proxy_host` accepts a host only when it is a DNS name that `ProxyRecipe::validate` accepts,
written in lower case, and `canonical_ip_literal` accepts an IP literal only in the spelling that
`ProxyRecipe::validate` accepts. `plasmosome_core::manifest` uses them for a workspace `dst`, for
`[network]` hosts and for the address of each pin.

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
neither read from nor written to a log. Decoding names the first rule it meets instead. Each recipe
and launch checks itself while it decodes, so decoding names a broken recipe before a broken path
or name of the same capability, and a zero port before a NUL in its host. The value is refused on
every path either way.
The trait's `grant`, `apply` and `plant` refuse an invalid value before any change, with
`BackendError::InvalidOperation` naming the address and the broken rule; `FakeBackend` does, at a
fresh address and at an issued one alike.

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
check and the open is refused. It is `Link { path: "." }` when what is there now can be opened,
such as a link, a FIFO, a file or another directory, and `Unreadable { path: "." }` when it
cannot, such as a Unix socket or nothing at all. The open does not wait on a FIFO. Every
directory is opened before it is listed, and every file before it is read.

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
`no_crate_enables_the_backend_test_fixture` fails when any crate, the testkit included, turns it
on by a normal or build dependency, for any target. A `--workspace` build merges the features of
every member it selects, so one crate's edge would turn the fixture on for product code too, and
every gate builds that way. No crate needs the fixture yet. A crate that does must add, in the
same change, a `cargo check -p` for each product crate, and change the guard.
Core maps a `Conflict` to code 102 and a `PinRefusal` to code 108. No 102 carries a `fix`. Spec 027
§3 item 1 says so for a name another plasmid or the image holds; no spec names the 102 for two
commands of one tree. The 108 for `CommandNotInTree` has no `fix` yet, because the declaration field
it would name does not exist. plasmosome-tbru adds that `fix`, and none on the 102. No product path
reaches `CommandNotInTree` today, since no product tree declares a command. The other 108s are
faults in the files, not in a declaration, so under spec 001 they carry no `fix`.

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
address or one owner time out. `fail_before_effect_for_owner` makes every later `grant` and
`apply` of a valid operation owned by one owner return `Fault` before any change.

An operation that had its first effect and then failed is an incomplete effect. `grant` and
`apply` return `IncompleteEffect { class, id, detail }` at the operation's address. The detail is
diagnostic, never a selector. The snapshot lists exactly that operation in `incomplete`, with no
standing object and no issued record. While it is there, a `grant` or `apply` of the same
operation returns `IncompleteEffect` with the same class and id and the latest failure's detail,
and creates nothing, and another operation or a `plant` at that address returns
`IdentityConflict`. Both answers come before the `Fault` of `fail_before_effect_for_owner`. A
`revoke` of its handle returns `UnknownHandle`, because no record was issued. `apply_removal` with
the operation's owner and full removal clears it and nothing else; another owner or capability
returns `UnknownObject`, and a graceful timeout returns `DrainTimedOut` and keeps it. A withdrawal
that fails after it released something returns `IncompleteEffect` too: a standing holding moves to
`incomplete` and keeps its issued record, if it was granted, and an unfinished effect stays. A
later exact `apply_removal`, or a `revoke` of the original handle, clears both. `CompositeBackend`
routes each of these calls to the leaf that owns the class, and its snapshot joins every leaf's
unfinished effects. `FakeBackend::fail_after_effect_for_owner` makes every later `grant` and
`apply` of a valid operation owned by one owner, at an address that holds nothing, fail after its
first effect. An invalid operation still returns `InvalidOperation`, and
`fail_before_effect_for_owner` refuses first. `FakeBackend::fail_next_withdrawal` makes the next
withdrawal at one address fail after release, through `revoke` or `apply_removal` and under either
policy. A graceful timeout comes first and leaves it armed.

`DrainSpec::shutdown_deadline` is the deadline `membraned` sends with the 4090 `shutdown` when it
stops a guest under that drain (spec 023 §7). It is a tenth of the drain deadline, and at least
1 ms, below that deadline, in whole milliseconds. It is none with `Force`, or when under 1 ms
would be left: then no `shutdown` is sent and the helper is killed at the drain deadline. It
lives here, beside `DrainSpec`, because it reads only the drain's deadline and policy.

`guest_protocol::GuestObservation` is the strict guest inventory from spec 001 §4.2. Every
inventory is required, even when empty. Decoding checks object and binding identities and stream
end structure; callers changing public fields call `validate` before use. `projections` visits
all ten kinds through `ProjectionRef`, and `grants` returns each kind, projection ID and grant,
including streams. Neither accessor includes software references. `SoftwareObservation` requires
both `trees` and `held`; `TreeReference.exec_id` is required even when null. While spec 027
reserves the software journal record and attach wire, `SoftwareTree` has no values and `trees`
accepts only an empty array. Host association, authority and policy checks remain the caller's.

## What's inside

| Piece | Responsibility |
| --- | --- |
| `EnforcementBackend` | The trait: grant, revoke, and observe the complete account |
| `EnforcementSnapshot` | A backend's complete account: standing holdings, issued records and unfinished effects, checked when built and when decoded |
| `FakeBackend` | In-memory recorder — the test workhorse |
| `CompositeBackend` | Routes capability classes to the backend that owns them |
| Universe classes | What "system state" means for residue verification: sockets, mounts, processes, proxy entries, session files |
| `MockMode` | How a plasmid's calls are served: `simulate`, `capture` or `passthrough`, the default. It lives here, beside `CellId`, so the ledger can record a mode without depending on core |
| `digest` | The SHA-256 `Digest`: exactly 64 lowercase hexadecimal digits, as text and in JSON. The membrane re-exports it |
| `lifetime` | Spec 028's lifetime values. A `Bound` is two readings: a UTC `WallMs` and a `Continuous` deadline tagged with its `BootSession`. It has passed at a reading when either of its two readings has, and a continuous deadline from another boot session is ignored. It keeps no state, so a caller that must keep it passed keeps a fence. Also the range checks for leases, idle limits and drain budgets, and RFC 3339 UTC text. These are not wire data yet: spec 008's journal record will give them their shape |
| `clock` | The `Clock` seam every lifetime decision reads time through: wall, continuous and wait readings, taken together. `ManualClock` lets a test move each one alone. The production clock is `SystemClock` in `plasmosome-core` |
| `paging` | Spec 001's observation pages, shared by guest, membrane and controller. On the responding side, a server keeps one `CaptureSlot` per connection. The slot holds the compact JSON account that the server's `collect` froze with `FrozenAccount::freeze`, and serves it in pages of 1 to 65,536 bytes until its deadline. On the requesting side, a caller's `PageAssembler` yields the account only when every page arrived in order from one capture and the bytes match the stated length and hash. `ObservationPage` decodes only from an object. Both sides pass the clock and own the IO |
| `software` | Spec 027's software tree before attach: `pin` checks a directory against the verified package's paths, sizes and digests, and `check_names` decides the 102 name conflicts. Nothing is placed here |
| `guest_protocol` | What crosses spec 001 §4.2's 4090 control lanes, shared by the guest and the host so neither accepts a line the other refuses. `ControlLane` is exactly `"normal"` or `"withdrawal"` in JSON, and `BootToken` exactly 64 lowercase hexadecimal digits; the membrane's `runtime::hello` re-exports both, with `BootTokenError`. The envelope: `encode_request`, `encode_success` and `encode_failure` write compact lines with no newline, echoing a `RequestId`'s JSON text unchanged. `split_request` reads one request line as exactly `{id, method, params}` with params an object, and refuses anything else with -32700 or -32600 as spec 001 §1 says, under a `null` id when the line's id cannot be trusted. A refusal's message keeps at most 1,024 bytes. The reader bounds the line. `decode_control_request` checks lane admission and the seven control methods' closed params; `encode_control_request` writes them and returns serialization errors. `InstallParams.address` is required but nullable, non-null exactly for a proxy map; `ProxyAddress` restricts it to the canonical guest pool. `split_reply` reads closed success or error envelopes with exact u64 ids and bounded fault text. `decode_control_reply` checks the expected result, boot and grant; `encode_control_result` writes it. `Yes` accepts only true. Observe pages still need the caller’s assembler. Workload methods are not here yet |

Tests: `cargo test -p plasmosome-backend`
