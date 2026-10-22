# plasmosome-membrane

The per-cell supervisor. One instance per cell, running on the host, outside the cell it guards.

A cell is a hardware-isolated virtual machine. Everything crossing its boundary — the VM process
itself, the network path, the broker daemons that police egress and DNS — is a child of this
supervisor. It is the membrane: the selective barrier that decides what enters and leaves.

Its central obligation is unglamorous and absolute: **nothing it starts may outlive it
unnoticed.** An orphaned child in this architecture is a process still holding capabilities that
were supposed to be revoked, so every spawn path here is paired with a reap.

## What's inside

| Module | Responsibility |
| --- | --- |
| `vmm` | The VM process lifecycle: fork, observe, kill, and reap on drop |
| `readiness` | Is a broker actually serving? Readiness is an *answered query*, never a running pid or an existing file — a process can be alive and useless |
| `brokers` | A cell's broker set, each one a `vmm::VmmChild`, asked again on every call |
| `cell_args` | Spec 023 §4's argv, `--config-fd=3 --lock-fd=4 --cell=<id>`, matched exactly, so `membraned` can tell it from a usage error |
| `cell_start` | Spec 023 §5's first two steps: `read_config` reads the config record from descriptor 3 under a cap, and `adopt_lock` checks that descriptor 4 is this cell's locked `supervisor.lock`; see "Starting a cell" below |
| `daemon` | `membraned`: spawns the configured brokers and answers `membrane.status` on a private control socket |
| `control` | The ndjson control-protocol envelope the daemon serves |
| `exec` | Resolving and preparing broker commands for `vmm::VmmChild` to run |
| `runtime` | Parts of the spec 001 §4.2 hardware runtime. So far the `RemoveOutcome` of removing a path, which is core's `private_socket::RemoveOutcome`, the exact 4090 hello requests with the strict judge of their replies (`hello`), which re-exports `ControlLane`, `BootToken` and `BootTokenError` from `plasmosome-backend`'s `guest_protocol`, the supervisor's own observation of a helper's Darwin sandbox (`confinement`, below), and three modules re-exported from elsewhere: the SHA-256 `Digest` from `plasmosome-backend` as `runtime::digest`, and from `plasmosome-core` the canonical absolute `RecipePath` as `runtime::path` and the `RuntimeRecipe` as `runtime::recipe`. The JSON reader that refuses duplicate keys is core's too, reachable inside this crate as `runtime::strict_json`. Core's README, "The runtime recipe and the config record", describes them. `launch` execs the VMM helper a recipe names with the exact descriptors spec 001 §4.2 sets out; see "Launching the VMM helper" below. `artifacts` verifies the pinned artifacts with core's file judge and keeps each one open; see "Verifying the pinned artifacts" below. `cell_directory` checks a cell's private directory with core's `PrivateDir` and makes the writable root copy in it; see "The cell directory and the writable root" below. `closure` audits the Mach-O load names of the verified helper and libraries before exec; see "Auditing the helper's load names" below. `initramfs` checks that the verified initramfs binds the verified guest policy; see "Checking the initramfs binding" below. `endpoints` binds the control and data listeners and reads the helper's diagnostics; see "The runtime's listeners and diagnostics" below. `handshake` runs the host side of the shim's two hellos and its data association; see "The shim's handshake" below. `vmm_helper` holds the decisions the VMM helper of spec 001 §4.2 makes: its exit codes, the judges of its argv and environment, descriptors, recipe, host limits (`judge_host_limits`), own path (`judge_self`), writable root, host policy, denial probes and loaded images, and its 14 libkrun calls with a runner that stops at the first failure. The helper binary that uses them is not built yet. Nothing here boots a cell yet |

## Use

```rust
use plasmosome_membrane::vmm::{VmmChild, VmmState};

let mut child = VmmChild::spawn(launcher)?;
assert_eq!(child.state()?, VmmState::Running);
child.kill()?;
```

Tests: `cargo test -p plasmosome-membrane`

## Command line

`membraned <config.json>` runs the daemon in the foreground. A sole `-h` or `--help` prints help
on stdout and exits 0. A sole `--version` prints `membraned` and the package version, such as
`membraned 0.1.0`, and exits 0. Neither reads a configuration. Any other single argument is the
configuration path, so a file named `--version` is passed as `./--version`.

Exactly `--config-fd=3 --lock-fd=4 --cell=<id>`, in that order with `<id>` a cell ID, is how
spec 023 §4 starts a cell's supervisor. Spec 025 §6 says `membraned` then starts as spec 023 §5
says, but cell mode is not built yet. Until native task `plasmosome-t8nq` builds it, that exact
form prints one line saying so and exits 2, with no usage text, by Main's decision (3) on native
task `plasmosome-rhn`. No argument, or any other list of more than one, prints usage on stderr
and exits 64. That includes those three arguments in another order, with a fourth, or with a
`--cell=` value that is not a cell ID. A configuration the daemon cannot read, or that
`daemon::parse_config` refuses, exits 2. A later failure exits 1: to bind or inspect the control
socket, to find or start a broker, or to accept connections while serving. The `-h`, `--help`,
`--version` and exit 64 rules are spec 025 §6's. Exits 2 and 1 are unchanged.

## Starting a cell

`cell_start` holds what a cell's `membraned` does first, before it binds a socket or creates
anything (spec 023 §5). `membraned`'s `main` does not call it yet; native task `plasmosome-t8nq`
wires it in.

`read_config` reads descriptor 3 until EOF, or until it has read `MAX_RECORD_BYTES + 2` bytes, one
more than the largest record, so a writer that never stops cannot make it read more. The read has no
deadline: a writer that stops without closing its end holds it there, and the controller closes its
end once the record is written. It closes the descriptor and decodes the bytes with core's
`SupervisorConfig::decode`. A record of exactly 1,048,576 bytes before its newline is accepted, and
one of 1,048,577 is `TooLarge`. A record for another cell than argv's `--cell=` is `CellMismatch`.

`adopt_lock` takes `flock(LOCK_EX | LOCK_NB)` on descriptor 4 and sets its close-on-exec flag.
The handed-over descriptor shares the open file the controller locked, so the lock is taken at
once; a fresh open of the file while another holds it is `LockNotHeld`. It then opens
`cells/<cell>` from the record's instance root with core's `InstanceRoot::cell_dir`, which follows
no symlink, and refuses unless that directory's `supervisor.lock` has the descriptor's device and
inode. So another cell's lock, an unrelated file and a directory moved to `retired/` are all
refused. The returned `AdoptedLock` keeps the descriptor; nothing calls `LOCK_UN`.

## Readiness is an answered query

A supervisor is ready when its control socket **answers** a control-`status` request — not when
the process is alive, and not when a listener accepts. This is measured, not assumed: a
confinement-profile bug leaves a broker half-alive, its control socket dead while its data
listener still answers, and every weaker test calls that broker ready. So `readiness::probe`
treats accept-without-answer as not ready.

The broker set inherits the same rule. It answers ready only once every broker answers its own
control socket, and it asks again on every call rather than caching a past yes. One `status` call
has **one fixed budget** for the whole set: the clock starts once and nothing renews it, and each
probe — connection, request, reply, classification — spends the same allowance. A broker cannot
stretch the call by trickling its reply, dribbling an oversized one, or withholding the frame
terminator, and a verdict that finishes after the budget is refused, never blessed ready. A broker
the budget never reached is `deadline_spent`.

The probe connects with a nonblocking socket and stays nonblocking: waits are requested in slices
of at most 25ms of what is left, partial writes and interrupts never restart the clock, and a
connect that returns after the budget is discarded unread. A broker reply is the first
newline-terminated frame, and at most 1,048,576 raw bytes — the same magnitude as the §1 request
cap — may precede that newline; the first excess byte is refused as `malformed` without draining
the peer. Silence, an incomplete frame, or end of file before the newline is `timed_out`, even
when the unterminated bytes look like JSON. There is no unconditional wall-clock guarantee:
scheduling and syscall overshoot can exceed the budget, but they do not renew it, and any late
verdict is refused.

Shutdown reaches the probes. SIGINT or SIGTERM cancels the query in flight: the control
conversation closes with no reply and no invented wire state, and teardown begins without waiting
out the allowance.

## Observing a helper's sandbox

Spec 001 §4.2 asks the supervisor to check the helper's host confinement itself before readiness,
not to take the helper's or the guest's word for it. On Darwin, `runtime::confinement::observe(pid)`
asks the kernel through `sandbox_check`, from outside the helper. Off Darwin the result is always
`UnsupportedPlatform`. Nothing calls it yet; the cell composition in native task `plasmosome-7em`
will.

Pass the pid of a child this process has not reaped, such as the helper it spawned and still
holds. Until it is reaped, its pid cannot name another process. `observe` also reads the kernel's
unique id of the process before and after its queries, and the evidence keeps that id.

`Confined` claims only this: the kernel reports the process sandboxed, and `process-fork`,
`network-outbound`, `network-inbound`, `network-bind` and `file-write-create` each answer denied
when asked with no filter argument. A cell must not be reported ready on any other result, which
includes a supervisor that is sandboxed itself and a query the kernel cannot answer.

`Confined` does not show that the verified host policy is installed, and its evidence says so with
`PolicyBinding::PropertiesObserved`. Profiles much looser than the host policy read `Confined` too.
Measured on macOS 26.6.2, a child under each of these three could still do the following:

- `(version 1)(deny default)(allow file-read*)(allow file-write* (subpath "/"))(allow network* (remote ip))(allow network* (local ip))`.
  The child created, overwrote and unlinked files outside the work tree, connected over TCP to
  127.0.0.1 and bound a TCP socket. Only `fork` and running `/bin/echo` were denied. Asked with
  no filter, its four file and network operations answered denied although it allows them for
  every path under `/` and for any IP address.
- `(version 1)(allow default)(deny process-fork)(deny network*)(deny file-write-create)`.
  The child overwrote and unlinked existing files and ran `/bin/echo`. Operations outside the
  five are not asked.
- `(version 1)(allow default)` with a `deny` for exactly the five operations and nothing else.
  The child could do what it could under the second profile. A profile can always deny exactly
  the operations it is asked about, so asking more operations with no filter cannot settle O-3.

The macOS tests here apply all three and expect `Confined`.

A bare `(deny default)` profile reads `Confined` as well. Spec 001 §4.2 says "Substituting another
policy, even one called deny-default, must fail." This check alone cannot meet that: Darwin has no
call that returns a process's installed profile, and the profiles above are substitutes that pass.
Nor does it give the evidence §4.2 asks for, binding the verified policy and its installation to
the process; the unique id binds only the process. Owner gate O-3 on native task `plasmosome-qep`
decides what Darwin evidence is enough.

One option tells the first looser profile from the host policy: `file-write-create` asked with a
path filter. Measured on macOS 26.6.2, for a file outside the work tree, the first profile answers
allowed and the host policy denied. The path must exist. For a path that does not exist, the query
answers denied under every profile measured, `(allow default)` included, and so does the host
policy inside its own work tree. A create check that names a file not there yet therefore catches
nothing. The option does not catch the second or third looser profile: for a file outside the work
tree, both answer denied, as the host policy does. The network operations have no filter like
that.

For an existing file inside the work tree, the host policy answers allowed, while the second and
third looser profiles and `(deny default)` answer denied. Of the six profiles measured here (the
host policy, the three looser ones, `(allow default)` and `(deny default)`), only the host policy
gives that pair of answers, denied outside and allowed inside. A reviewer then measured two
profiles written to match it, and both were reproduced here:
`(version 1)(allow default)(deny process-fork)(deny network*)(deny file-write*)(allow file-write* (subpath W))`,
with `W` the work tree, and the same rules with the `allow` first. Each answered as the host
policy does to the sandboxed query, to the five, and to `file-write-create` for an existing and a
missing file both inside and outside the work tree. Yet its child still ran `/bin/echo`. So no
fixed set of queries, path-filtered ones included, settles O-3.

It also says nothing about descriptors the helper already holds, or about the images it loaded
(owner gate O-5).

A dead process answers denied to every query. Measured on macOS 26.6.2, a reaped pid, or a zombie
that had no sandbox, answers the queries `observe` makes just as a confined process does. So the
result is unverifiable unless `proc_pidinfo` reads the same unique id before and after the
queries; it cannot read a zombie's. The unique id comes from the flavor
PROC_PIDUNIQIDENTIFIERINFO, whose record XNU declares only in a private header, so the crate
declares the 56-byte record itself and its macOS tests check it against the running kernel.

`sandbox_check` is declared in no SDK header either, and spec 001 notes that `sandbox_init` is
deprecated and not a supported portable API. If a later macOS makes `sandbox_check` return -1, the
observation is unverifiable. If it changes the answers instead, the macOS tests here fail: they
expect the qualified policy confined, and allow-default, no sandbox and each of the five allowed
alone unconfined.

## Dropping a handle cleans up its managed group

`vmm::VmmChild` owns one direct child and the process group that child creates with `setsid`.
Trusted host helpers must stay in that group, retain signal permissions, and add no members from
the first terminal group-signal attempt through inspection and reap. Dropping the handle stops
the leader, applies the group cleanup disposition, and reaps the direct child. `kill` performs
the same leader teardown synchronously. A group signal does not prove that scheduling has
finished every worker.

The handle must be its direct child's only consuming waiter, and the process must leave `SIGCHLD`
waitable. Once another waiter consumes the status, every lifecycle method records lost authority
and refuses to signal the released PID or PGID; workers may remain and Drop can only report them
on stderr as a best-effort diagnostic. Explicit lifecycle calls return `SupervisionError` when
the caller needs an inspectable result. Closed stderr can lose a Drop diagnostic, and blocked
stderr can delay it.

On Darwin, a terminal group signal can report `EPERM` when only zombies remain. While the direct
leader is still waitable, the handle accepts that disposition only when a complete SDK process
table contains that leader exactly once and every member is a zombie. The table walk is not an
atomic snapshot, so complete query visibility and the no-new-members requirement are part of the
trusted-host contract. This does not cover descendants that leave the group, change credentials,
or keep forking. `mem::forget` leaks an unfinished handle.

## A fork waits while descriptors are created

Darwin cannot create a pipe, a socket or an accepted connection already close-on-exec, so each one
exists for a moment before `FD_CLOEXEC` is set, and a fork in that moment hands it to the child.
`VmmChild::spawn` forks holding the read side of one process-wide lock. The readiness probe
creates its socket holding the write side, `HelperExec::prepare` creates the helper's two pipes the
same way, and so does `RuntimeListener` for its socket and each connection it accepts. A spawn
waits while any of them is created; on Linux all are close-on-exec from the start, so the wait
matters only on Darwin. Creation under the write side returns the values that
own its descriptors, and only those descriptors are checked: if any is not already close-on-exec,
all of them are closed before the lock is released and creation fails. A value lends the lock its
descriptor through `OwnsDescriptor`, as a raw descriptor, so making a copy of it needs `unsafe`.
A descriptor created under the lock and kept anywhere else is not checked, so the code that
creates it must mark it close-on-exec before the lock is released. A spawn from the thread that
holds the write side returns `SpawnError::DescriptorLockHeld` instead of waiting on itself.

The lock covers only those paths. Forks that do not go through `VmmChild::spawn`, such as
`std::process::Command`, do not take it, and neither do the standard library's `bind` in
`daemon.rs` and `accept` in `control.rs`. Those two are safe only because `membraned` binds its
control socket, spawns its brokers and serves requests on one thread, so none of them can overlap
a fork. Code that creates descriptors or forks on a second thread must take the lock first.

## Verifying the pinned artifacts

Spec 001 §4.2 refuses a replaceable parent or a mutable artifact rather than trust it after hashing,
and asks that verification keep the selected inode and bytes, because the helper and libkrun open
artifacts by path. `runtime::artifacts::verify_artifact` checks one artifact that way, and
`verify_recipe_artifacts` checks all of a recipe's in field order, stopping at the first refusal.

The rule is core's, not a copy. `verify_artifact` walks the path from `/` with core's no-follow
walk, `Walk::from_root`, then judges and hashes the file with core's `artifact::verify_file`, the
code the controller runs before `cell.new` allocates a cell. So a file the controller admits is one
the membrane admits, as spec 023 §1 says. Core's README, "Artifacts", gives the rule. A refusal
under it is `ArtifactFault::Rule`, which holds core's fault and shows core's text; only
`ChangedSinceVerification` and `Replaced`, which `reconfirm()` gives, are the membrane's own. The
hash reads at most one byte past the size the file was checked at. Nothing bounds that size: a
process that opened the file for writing before it was locked can extend it first, and in a review
probe on macOS 26.6.2, in the dev profile, a writer that kept extending a 64 MiB file made its
verification take 43 to 340 s, against 3.9 to 5.5 s alone.

On macOS pass the canonical path: `/var` is a symlink, so `/var/folders/...` is refused at `/var`.
Measured on macOS 26.6.2 on 2026-10-21, the qualified bundle's kernel is refused at
`/usr/local/var`, which has mode 0775; that is owner gate O-1 on `plasmosome-qep`.

A `VerifiedArtifact` keeps the descriptor, the file's device and inode, its size and times, and its
digest. `file()` still reads the verified inode after the path is made to name another file.
`reconfirm()` opens the path again under the same checks. It refuses another inode as `Replaced`,
and a file whose size or times moved since it was verified as `ChangedSinceVerification`. Call it
just before the launch that relies on the path. Root and the effective uid can still clear `uchg`
and change a file, or replace a directory they own; that is owner gate O-4. A process that opened
the file for writing before it was locked can still write it too, because the mode and `uchg`
refuse only new opens. Measured on macOS 26.6.2, such a write moves the file's times, so
`reconfirm()` refuses a change made before it runs, but not one made after.

`reconfirm()` sees a change to the file's bytes only if the change moves the device, inode, size,
modification time or change time. A shared mapping made before the file was locked, from a
descriptor opened for writing, can change the bytes whatever protection it had when the file was
locked, because `mprotect` can add write later. Such a write moves the times only at `msync` or
`munmap`, if at all, so neither `reconfirm()` nor verification sees it. Measured on macOS 26.6.2 on
APFS on 2026-10-21, a write through such a mapping, writable at the lock or made writable after it,
changes what `pread` and a fresh open read at once, while the size and times stay as they were until
`msync` or `munmap`; at `munmap` of the mapping made writable after the lock, only the change time
moved. A mapping made from a descriptor opened read-only cannot gain write (`mprotect` fails with
`EACCES`), and neither can a mapping made after the lock: `mmap` with `PROT_WRITE` fails with
`EPERM`, and adding write with `mprotect` fails with `EACCES`. Hashing again in `reconfirm()` would
only narrow the window, because the mapping can write after any hash, so it does not.

## Checking the initramfs binding

Spec 001 §4.2 boots the guest from the pinned initramfs: an uncompressed Linux newc archive whose
protected regular entries `init` and `plasmosome/guest-policy` become `/init` and
`/plasmosome/guest-policy`, the second holding exactly the guest policy's bytes. Before launch the
supervisor checks that binding and refuses duplicate, escaping, symlinked or mismatching entries.
`runtime::initramfs::verify_initramfs` does that check. Pass it the verified initramfs and the
verified guest policy. It returns an `InitramfsBinding`: the size and SHA-256 of `init`, the
policy's SHA-256 and the number of entries. Otherwise it returns an `InitramfsRefusal` naming the
first rule the archive broke.

It reads the archive through the verified descriptor, never by path, and at most 256 MiB
(`MAX_INITRAMFS_BYTES`). A larger file is refused before any read, and so is a read that gets more
than the bound. Every entry must start with the magic `070701` and have 13 header fields of 8
hexadecimal digits in either case, a check field of 0, and zero bytes as padding. A name must end
in its only NUL, be UTF-8, be at most 4,096 bytes with that NUL, and be one plain relative
spelling: not absolute, with no empty, `.` or `..` part and no trailing `/`. Each name appears
once, after an earlier directory entry for its parent. Only regular files with one link and
directories pass; a symlink, a device, a FIFO, a socket or an unknown type is refused. `init` must
have an execute bit and some bytes. The last entry is a `TRAILER!!!` with no body and no symlink
mode, and only zero bytes may follow it.

Some rules follow how Linux unpacks the archive, in `init/initramfs.c` at 6.12.91, the guest
kernel's version, and at master as read on 2026-10-21. Line numbers below are 6.12.91's, then
master's. The kernel never stops at `TRAILER!!!`: there it only frees its hard-link table (372-375,
384-387), and its unpack loop reads any later `0` at a 4-byte boundary as the start of another
entry (519-525, 544-550). So the rule that only zero bytes follow the trailer is the one that keeps
a second archive from being unpacked over the first. The kernel reads an entry's name only when
the entry is a symlink (296-304, 308-316), a regular file, or has no body (305, 317). A directory
with a body is skipped unread, so nothing under it is made, and the audit refuses it. The audit
also refuses a trailer with a body, which only fails closed: the kernel skips that body to the
same next header the audit would read. A `TRAILER!!!` with a symlink mode takes the symlink branch
before its name is compared, so the kernel tries to make a link and never sees a trailer; the
audit refuses it as a symlink. The kernel keeps a mode in a 16-bit `umode_t` (181, 191), and the
audit reads the type from the same low 16 bits. The kernel also skips an entry whose name is over
4,096 bytes (294, 306), and it keeps the last of two entries with one name. It skips zero bytes
between entries; the audit refuses them, and the qualified archive has none. 6.12.91 parses a
header field with `simple_strtoul` and master with `hex2bin`; the audit takes exactly 8
hexadecimal digits, which both read the same way.

It reads the bytes the descriptor holds now and does not hash them again, so the limit stated for
`reconfirm()` above holds here too: a write through a shared writable mapping made before the file
was locked can change them unseen. Non-zero bytes appended after verification are read and refused
as data after the trailer; appended zero bytes pass, as the kernel skips them. The cell,
plasmosome-7em, which is not built yet, must call `reconfirm()` after this check, just before the
launch that opens the initramfs by path. Off macOS no artifact verifies, so `verify_initramfs`
runs only in tests, on artifacts that `artifacts::verified_for_test` builds.

## The cell directory and the writable root

Spec 001 §4.2 puts `writable_root`, `control_path` and `data_path` under an owned, nonreplaceable
private per-cell directory, and has the supervisor create the writable root exclusively from the
verified root image. `runtime::cell_directory` does both. In cell mode the directory is spec 023's
`cells/<cell>/runtime/`, which `membraned` creates through core's `CellDir::create_runtime_dir`.

`PrivateDirectory::open` is core's `PrivateDir::open`, the check `RuntimeListener::bind` makes of
its parent. A refusal is `CellDirectoryFault::Private` with core's error, which names the rule and
the component that broke it.

`WritableRoot::create` makes the name in the held directory with `O_CREAT | O_EXCL | O_NOFOLLOW`,
so any entry already there, a symlink included, is `Exists` and is left as it was. It copies
through the verified descriptor, not the path, reads at most one byte past the verified size, and
hashes the bytes as it writes them. Then the root image must still pass `reconfirm()`. Its
refusal is `Source` with whatever fault `reconfirm()` found: a path that names another file, a
size or times that moved, or a fault of any check it repeats, such as a cleared `uchg`. A refusal
after the new file's `fstat` removes the copy as `remove()` does, so a removal that fails, or that
finds another object at the name, leaves what is there, and a copy renamed away stays under its
new name. If that `fstat` fails, the empty file stays, since there is no identity to remove it
by. The copy is a plain copy, not an APFS clone: a clone of a `uchg` file can carry the flag, and
the copy must be writable.

`remove()` and dropping a `WritableRoot` unlink the name in the held directory only while it is
still the file that was made, by device and inode. A cell directory renamed after the copy still
has its copy removed, and whatever is then at the old path is left. The inspection and the unlink
are two steps. As spec 001 §1 says of sockets, this is ownership-correct cleanup, not protection
from another writer with authority over the directory, the same uid included.

Spec 023 §5 step 9 keeps `root.img` when `membraned` stops on SIGTERM, but a dropped
`WritableRoot` always removes its copy. The cell, plasmosome-7em, will need a way to give the copy
up without removing it. Nothing calls this module yet.

## Launching the VMM helper

`runtime::launch::HelperExec` runs the pinned VMM helper the way spec 001 §4.2 says: argv is
exactly `[helper.path, "--runtime-fd=3"]`, the environment is empty, descriptors 0 and 1 are
`/dev/null`, 2 is a diagnostic pipe whose read end the caller keeps, 3 is a pipe holding the
recipe's one ndjson line and then end of file, and no other descriptor survives the exec.
`HelperExec::prepare` does every allocation and every call that can fail in the parent. The child
`VmmChild::spawn` forks then calls only async-signal-safe functions: it resets every signal to its
default action, copies the four descriptors into place, closes every descriptor above 3, sets umask
077 and the working directory `/`, unblocks every signal last and calls `execve`.
It exits 125 when that setup fails and 127 when `execve` returns. On Linux, 127 can also come from
glibc's dynamic loader, after `execve` succeeded, when a library the helper needs is missing, so
there it is a hint rather than proof. The pinned libkrun's implicit init uses the same codes: it
exits 125, 126 or 127 when it fails after `krun_start_enter` (`include/libkrun.h` at v1.19.4,
lines 1377 to 1380). Spec 001 §4.2 has the helper call `krun_disable_implicit_init`, so a helper
that follows the spec never exits with them that way.

Closing every descriptor above 3 is one `close_range` call on Linux; `prepare` refuses when the
kernel does not offer it. Darwin has no such call, so the child closes each number below the larger
of `getdtablesize()` and `kern.maxfilesperproc`. `getdtablesize()` is the smaller of the soft
`RLIMIT_NOFILE` and `kern.maxfilesperproc`, so the loop always runs to `kern.maxfilesperproc`, and a
soft limit lowered below a descriptor the process still holds does not let that descriptor through;
a test holds one above a soft limit lowered to 1,024. `prepare` reads `kern.maxfilesperproc` and the
child closes up to that value later, so only a descriptor at or above the value `prepare` read can
survive: one made before root lowered the sysctl, or one made after root raised it once `prepare`
had run. Those two cases are reasoned, not measured, because measuring them means changing the
sysctl. What was measured, on an Apple Silicon Mac with `kern.maxfilesperproc` at 245,760 on
2026-10-20, is that the sysctl, not the soft limit, bounds the descriptor numbers: under soft limits
of 345,760, 1,048,576 and unlimited, `dup2` onto 245,759 succeeded and onto 245,760 failed with
`EBADF`. A test holds the last descriptor the table allows. On that Mac the loop took about 31 ms
with the machine lightly loaded and up to about 680 ms under heavy load.

An empty environment and the working directory `/` mean no `DYLD_*` or `LD_*` variable and no
relative path can add to where the helper's loader looks for libraries. They do not remove a search
path built into the helper or its libraries, such as the one owner gate O-6 of `plasmosome-qep`
names in the qualified libkrun. The launch checks no artifact either: the cell, plasmosome-7em,
which is not built yet, must verify the helper and its libraries with `runtime::artifacts` before it
prepares the launch. That checks only the files the recipe lists. `runtime::closure` audits the
names the helper and those libraries load; see the next section.

## Auditing the helper's load names

Spec 001 §4.2 asks that every library the helper loads be a listed, verified library or part of
Darwin's sealed system, and that each be found without any search. `runtime::closure::audit_closure`
checks what the files can show of that before exec. Pass it the verified helper and the verified
libraries in recipe order. It reads each through its verified descriptor, at most 512 MiB, and
reads the load commands of the 64-bit image for the host CPU: a thin file, or the one member of a
fat file for that CPU. A fat file with two members for the host CPU is refused, because the audit
cannot know which one dyld would pick.

Every name in an `LC_LOAD_DYLIB`, `LC_LOAD_WEAK_DYLIB`, `LC_REEXPORT_DYLIB`, `LC_LAZY_LOAD_DYLIB`
or `LC_LOAD_UPWARD_DYLIB` command, in the helper and in each library, must resolve by rule:

- `@executable_path/` joins the helper's directory, and `@loader_path/` the directory of the image
  that holds the command. The join is lexical, and the result must equal a listed path exactly. A
  run of `..` straight after the prefix is allowed, because it leaves directories the artifact
  walk verified, as in `@executable_path/../lib/libkrun.1.19.4.dylib`. A `..` after a directory
  the name itself writes is refused (`DotDotAfterNamedDirectory`): dyld resolves it on the file
  system, through that directory even when it is a symlink, and nothing verified it. Measured on
  macOS 26.6.2, `@executable_path/sub/../libt.dylib` with `sub` a symlink loaded a library from
  outside the bundle. An absolute name with such a `..` is refused the same way.
- An absolute name that equals a listed path is that library.
- Any other absolute name must be clean ASCII under `/usr/lib/` or `/System/Library/`, and the
  deepest part of it that exists must be on the sealed system volume; a symlink that leads nowhere
  is refused. That volume is the one `statfs` reports mounted at `/` and read-only, whose
  capabilities, read with `getattrlist(ATTR_VOL_CAPABILITIES)` on its mount point, report
  `VOL_CAP_FMT_SEALED` as valid and set. When that call fails, or does not report the capability,
  the volume counts as not sealed. The check is needed because those prefixes are not all sealed.
  On macOS 26.6.2, `/usr/share/firmlinks` puts seven directories under `/System/Library` on the
  data volume, and sealed symlinks such as `/usr/lib/cron` lead to it too.
- `@rpath/` names, names with no leading `/` or `@`, and any other `@` name are refused: dyld finds
  them by searching.

Any `LC_RPATH` is refused, because dyld also searches it for a leaf name passed to `dlopen`. So is
`LC_DYLD_ENVIRONMENT`, a dynamic linker other than `/usr/lib/dyld`, and a load command whose
`LC_REQ_DYLD` bit says dyld must understand it but whose kind the audit does not know. A listed
library that nothing names is allowed. The evidence records each resolution, each system name
relied on, and `kern.osversion` and `kern.osproductversion`, which name the OS build. Off macOS
the audit refuses `Unsupported`.

The volume check judges where a name's symlinks lead at the time of the audit, not each step on
the way. A name can pass whose path goes through a directory someone can write and comes back to
the sealed volume, and whoever can write that directory can change where it leads afterwards. For
example, `/usr/lib/cron` leads to `/private/var/at`, owned by `daemon` with mode 0755, so a process
running as `daemon` could make `/private/var/at/x` a link to a sealed file, let `/usr/lib/cron/x`
pass, and then replace it. Measured on macOS 26.6.2 (25G83), no step of such a chain is writable
by the supervisor's user: of the 11,459 symlinks under `/usr/lib` and `/System/Library`, 64 live
ones end off the sealed volume, which with the seven firmlinked directories makes 71 entry points,
and none of the 5,740 directories under them, nor any of their ancestors, is writable by that
user. A tighter check would cost little. Of the 3,649 install names in that Mac's shared cache,
3,599 pass the volume check today. Refusing any symlink on the way would refuse 19 more, all
under the private `TextToSpeechKonaSupport.framework`; requiring every existing step of a name to
be on the sealed volume would refuse none of the 3,599. The whole audit passes 3,113 of them,
because the other 486 are under `/System/iOSSupport/`, outside both system prefixes. The audit
keeps this limit for now, and plasmosome-0xun owns the tighter check.

The audit does not look everywhere dyld does. When an absolute name is not on disk, dyld also
tries `/System/Volumes/Preboot/Cryptexes/OS` followed by the name. All but four names in the
shared cache, such as `/usr/lib/libSystem.B.dylib`, are not on disk; the four on disk are
`/usr/lib/dyld` and three libraries under `/usr/lib/system/`. The shared cache itself lives in
that cryptex. That tree is on the Preboot volume, which is read-write and not sealed. `OS/usr/lib` and
`OS/System/Library` are owned by root with the `restricted` flag, as are all but one of the
directories under them (`OS/System/Library/Extensions` has only `sunlnk`). Their parent
`/System/Volumes/Preboot/Cryptexes` is mode 0777 with no flags and no sticky bit, and
`Cryptexes/OS` is owned by root, mode 0755, with `sunlnk`. So against the supervisor's own user
the prefix relies on root ownership, and on `sunlnk` stopping a rename of `OS`, not on SIP alone.
Refusing that path would refuse every system library, so the audit does not check it. Whether
dyld would take a file at the cryptex path over the shared cache is not measured, nor whether
`kern.osversion` changes when only the cryptex is updated.

The audit sees load commands only. It does not see the images the running helper loads
(`ImageInventory::NotIndependentlyObserved`, owner gate O-5 of `plasmosome-qep`), nor a name that
code passes to `dlopen` (`RuntimeLoads::NotAudited`). The pinned libkrun's `krun_create_ctx` loads
`libkrunfw.5.dylib` that way on every context. Measured on macOS 26.6.2 on 2026-10-21, with the
working directory `/` and an empty environment, dyld looked for that leaf name relative to the
working directory, then at `/System/Volumes/Preboot/Cryptexes/OS` followed by the name, then in
each `LC_RPATH` entry with and without that prefix, then in `/usr/lib`. The qualified libkrun's
`LC_RPATH` makes the audit refuse it today; `plasmosome-jab` plans the bundle that deletes it.

## The runtime's listeners and diagnostics

Spec 001 §4.2 has the supervisor create the control and data listeners before it launches the
helper, and keep them. `runtime::endpoints::RuntimeListener::bind` makes one at a recipe path with
core's `private_socket`, the boundary `plasmosomed` binds its control socket with; core's README,
"Private recovery sockets", describes it. `PrivateDir::open` walks the parent from `/` without
following a symlink, and requires every ancestor owned by root or this uid and writable by neither
group nor other, and the parent itself owned by this uid with no group or other bits.
`PrivateListener::bind` then works in that held directory, in this order. It refuses any entry
already at the path's name with `AddressInUse` and leaves it as it was: a live or stale socket, a
file, a directory, a FIFO or a symlink, dangling or not. It binds the socket at a temporary name in
that directory, sets it to 0600 and makes it listen there, and only then renames it to the path's
name with a rename that never replaces an entry. So a client that finds the path finds a 0600 socket
that accepts, and no connection lands while the umask's mode holds. A hard kill inside the bind can
leave a dot-named temporary socket in the directory, which the next bind ignores. A path too long
for a socket address is `PathTooLong`. Each refusal comes back as `EndpointRefusal::Private` with
core's error, which names the rule broken and the component that broke it. The bind holds the spawn
lock's write side, because on Darwin core makes the socket close-on-exec in a second call. Closing
the listener closes it, then removes the socket file only while the entry at that name in the held
directory is still that socket, and says what it found as a `RemoveOutcome`. Dropping it does the
same. A child that another thread is spawning holds a copy of the listener until its exec, so the
socket can take connections until then. The cell, plasmosome-7em, which is not built yet, is to
create that private directory.

`accept_ready` takes one pending connection without blocking, holding the same write side. Core
reads the peer's effective uid from the kernel and closes a peer of another uid unread: that is
`Connection::Refused` with core's refusal. A trusted stream is made nonblocking, and the peer's
process id is read right after the accept. What that pid names differs by platform. On Linux it is
`SO_PEERCRED`'s: the process that called `connect`, recorded then. On Darwin it is
`LOCAL_PEERPID`'s: the last process that created, connected, sent on or received on the peer's
socket before the read, a read still waiting included. A process that inherited the connection and
sent one byte before the accept is the one reported, and so is one that only waits in a read on it
(both measured on macOS 26.6.2, the second in 20 of 20 runs). So on Darwin a match with the
helper's pid shows that the helper was the last process to use that socket, not that it connected,
and a genuine helper whose child sent first does not match.

When the peer's credentials or pid cannot be read, the connection is `Refused` with
`PeerCredentials` and closed. On Darwin a peer that connected and exited before the accept is that
case, with `ENOTCONN`; on Linux the same peer is `Identified` with the pid it had. So a process
that connects and leaves gets a result of its own and cannot make the listener look broken.
`accept_ready` returns an error when the listener's accept fails, and in two cases after it took a
connection: core cannot put a trusted stream back into blocking mode, or the spawn lock finds the
accepted stream not close-on-exec. That stream is then closed.

`DiagnosticTranscript` reads the read end of the diagnostic pipe that `HelperExec::prepare` returns,
on a thread of its own, so a helper that writes a lot never blocks on descriptor 2. Start it before
`VmmChild::spawn`, so a thread that cannot start refuses the launch before anything forks. It keeps
the first 1,048,576 bytes and counts the rest. `finish(wait)` waits up to `wait` for end of file,
then tells the thread to stop. The thread then reads what the pipe holds without waiting, so bytes
written before `finish` are kept even with no wait. It checks for the stop every 25 ms, so `finish`
typically returns within `wait` plus 25 ms; on a loaded machine the scheduler adds to that. `eof` is
true only when every holder of the write end has closed it.

## The shim's handshake

Spec 001 §4.2 says a cell is guest-ready only on the trusted shim's actual handshake, not on a
fork, a socket file or a recorded pid. `runtime::handshake::handshake` is the host side of it. Pass
it the control and data listeners the cell bound before the launch, the helper's pid, the expected
guest policy digest, one `ProbeBudget` and a check of the helper's liveness. It returns an
`Association` that holds the normal lane, the withdrawal lane and the data connection, and closes
all three when dropped. Nothing calls it yet; the cell composition in native task `plasmosome-7em`
will.

It accepts the first control connection and checks its peer pid, sends the normal hello and judges
the reply with `hello::judge_normal`. It accepts the second control connection, checks it, sends
the withdrawal hello and judges that reply with `judge_withdrawal` and the normal lane's boot
token. Only then does it accept one data connection and check its peer. A data connection that
arrives early stays in the data listener's queue until both hellos have passed.

One allowance covers every accept, write and read, and nothing renews it. Before every wait the
handshake checks the budget, with shutdown winning over expiry, then the liveness check. A helper
that ended, or whose state cannot be read, fails at once. Waits last at most 25 ms of what is left.
The budget is checked again after each accept and after each reply is judged, so a step that
finished after the allowance is `TimedOut` at that step's stage. The check after the data accept
refuses a handshake that finished after its allowance.

A reply is read with the readiness probe's reader, through a limit of 1,048,576 bytes before the
newline plus one byte. One byte more is `FrameTooLarge`, before any parsing. End of file before
the newline is `Closed`. Bytes after the newline in the same read are `UnexpectedBytes`: the shim
sends one line and then waits, and the reader would drop those bytes. Only bytes that same read
returns are refused. A line that arrives after that read, or one after a frame of exactly 1,048,576
bytes, stays queued on its lane for the next reader.

Every accepted stream's peer pid must equal the helper's, and the pid is read again after each
reply. A pid of 0 or below never matches; Linux reports 0 for a peer outside the supervisor's pid
namespace. A stream from another pid is `ForeignPeer`, and a peer that `accept_ready` refused is
`PeerRefused`. So is a lane whose pid cannot be read after its reply: on Darwin a lane that hung up
right after its reply fails there with `ENOTCONN`, once no other process holds its socket. Each
fails the handshake, because nothing but the helper should reach a private endpoint. On Linux the
pid is the connector's, so the read after a reply adds nothing. On Darwin it names the last process
to use the peer's socket, as above. A match after a reply shows that the helper used the socket
last once the reply had arrived, so a reply sent by another process fails, unless the helper used
the socket after it. Measured on macOS 26.6.2, after the accept the pid followed a child that only
sent, in 20 of 20 runs.

These checks do not stop every process of the supervisor's own uid. That is the same-uid limit the
plasmosome-qep plan records as owner gate O-4, whose handling is "proceed; document". A process
that holds a copy of a lane's socket, inherited or passed to it, can still do this:

- On Linux, with a copy of each lane's socket, it can read both requests and send both replies,
  because `SO_PEERCRED` keeps the connector for the life of the socket.
- On Darwin its reply passes if the helper then sends or receives on that socket before the host
  reads the pid. In a review of this change on macOS 26.6.2, a forked child sent a forged normal
  reply, the connector then read on its end, and the reply passed the pid read and was judged, in
  2 of 2 runs.
- On both, nothing checks the sender once `handshake` returns, so `Association.peer` does not show
  who wrote later bytes on any of the three streams.

Any process of that uid can also connect to either listener first and fail the handshake with
`ForeignPeer`.

The `Association` stands for both hellos and the data association. Spec 001 §4.2 also asks for
the workload namespaces and the workload control group before readiness, and this handshake does
not observe them. On a failure it closes every stream it accepted. A connection still waiting in a
listener's queue closes when the caller closes that listener. When `handshake` returns an
`Association`, it has not checked that the helper still runs or that the three streams are still
open; the caller observes the helper.

## Control-socket ownership

`membraned` refuses an occupied control-socket path — a live socket, a stale file or a symlink —
and never unlinks it: the start exits naming the path, and clearing it is the caller's job. Once
bound, it records the socket's device and inode; when it returns — cleanly, on a later error, or
through an unwinding panic — it drops every broker and removes the path only if a no-follow
inspection finds that exact socket still there, as one best-effort attempt. A replacement a
caller settles at the pathname after start — a regular file or a leaf symlink, target present or
dangling — is left exactly as the caller left it, and the original socket renamed elsewhere stays
there. A start whose identity capture fails refuses before any fork, without unlinking anything.
The inspect-then-unlink pair is not atomic, so this is ownership-correct cleanup under a
coordinated namespace with trusted, stable ancestors — not defense against another writer with
authority over the directory, the same UID included. The same lifecycle holds for the
controller's socket; spec 001 §1 states it once for both daemons.

Killing `membraned` with `SIGKILL` runs no destructors, so its brokers can keep running and its
socket path remains. There is no implemented residue observation or recovery mechanism.

The division of labour is a design rule held in review, not by a test: VMs, shims and brokers
belong here, and the controller (`plasmosome-core`) must never own them. Both halves are written
down per crate — `crates/plasmosome-membrane/AGENTS.md` and `crates/plasmosome-core/AGENTS.md` —
not in the root file.

## Not here yet

The netstack shim and the vsock bridges belong to this crate by that rule, and none of it is
built: the modules present are `brokers`, `cell_args`, `cell_start`, `control`, `daemon`, `exec`,
`readiness`, `runtime` and `vmm`.
Native task `plasmosome-qep`, which boots one real cell from the qualified Darwin runtime bundle,
carries the first part of that work: the vsock endpoints and their handshake.
