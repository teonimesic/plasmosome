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
| `daemon` | `membraned`: spawns the configured brokers and answers `membrane.status` on a private control socket |
| `control` | The ndjson control-protocol envelope the daemon serves |
| `exec` | Resolving and preparing broker commands for `vmm::VmmChild` to run |
| `runtime` | Parts of the spec 001 §4.2 hardware runtime. So far the `RemoveOutcome` of removing a path, the exact 4090 hello requests with the strict judge of their replies (`hello`), and three modules re-exported from elsewhere: the SHA-256 `Digest` from `plasmosome-backend` as `runtime::digest`, and from `plasmosome-core` the canonical absolute `RecipePath` as `runtime::path` and the `RuntimeRecipe` as `runtime::recipe`. The JSON reader that refuses duplicate keys is core's too, reachable inside this crate as `runtime::strict_json`. Core's README, "The runtime recipe and the config record", describes them. `launch` execs the VMM helper a recipe names with the exact descriptors spec 001 §4.2 sets out; see "Launching the VMM helper" below. Nothing here verifies an artifact or boots a cell yet |

## Use

```rust
use plasmosome_membrane::vmm::{VmmChild, VmmState};

let mut child = VmmChild::spawn(launcher)?;
assert_eq!(child.state()?, VmmState::Running);
child.kill()?;
```

Tests: `cargo test -p plasmosome-membrane`

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
creates its socket holding the write side, and `HelperExec::prepare` creates the helper's two pipes
the same way, so a spawn waits while either is created; on Linux both are close-on-exec from the
start, so the wait matters only on Darwin. Creation under the write side returns the values that
own its descriptors, and only those descriptors are checked: if any is not already close-on-exec,
all of them are closed before the lock is released and creation fails. A descriptor created under the lock and kept anywhere else is not checked, so the code that
creates it must mark it close-on-exec before the lock is released. A spawn from the thread that
holds the write side returns `SpawnError::DescriptorLockHeld` instead of waiting on itself.

The lock covers only those three paths. Forks that do not go through `VmmChild::spawn`, such as
`std::process::Command`, do not take it, and neither do the standard library's `bind` in
`daemon.rs` and `accept` in `control.rs`. Those two are safe only because `membraned` binds its
control socket, spawns its brokers and serves requests on one thread, so none of them can overlap
a fork. Code that creates descriptors or forks on a second thread must take the lock first.

## Launching the VMM helper

`runtime::launch::HelperExec` runs the pinned VMM helper the way spec 001 §4.2 says: argv is
exactly `[helper.path, "--runtime-fd=3"]`, the environment is empty, descriptors 0 and 1 are
`/dev/null`, 2 is a diagnostic pipe whose read end the caller keeps, 3 is a pipe holding the
recipe's one ndjson line and then end of file, and no other descriptor survives the exec.
`HelperExec::prepare` does every allocation and every call that can fail in the parent. The child
`VmmChild::spawn` forks then calls only async-signal-safe functions: it resets every signal to its
default action, copies the four descriptors into place, closes every descriptor above 3, sets umask
077 and the working directory `/`, unblocks every signal last and calls `execve`.
It exits 125 when that setup fails and 127 when `execve` returns.

Closing every descriptor above 3 is one `close_range` call on Linux; `prepare` refuses when the
kernel does not offer it. Darwin has no such call, so the child closes each number below the larger
of `getdtablesize()` and `kern.maxfilesperproc`. On an Apple Silicon Mac with a limit of 245,760,
measured on 2026-10-20, that loop took about 31 ms with the machine lightly loaded and up to about
680 ms under heavy load. The kernel creates no descriptor at or above both limits, but one made
before either limit was lowered would survive.

An empty environment and the working directory `/` mean no `DYLD_*` or `LD_*` variable and no
relative path can add to where the helper's loader looks for libraries. They do not remove a search
path built into the helper or its libraries, such as the one owner gate O-6 of `plasmosome-qep`
names in the qualified libkrun. The launch checks no artifact either: the cell verifies the helper
and its libraries before it prepares the launch.

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
built: the modules present are `brokers`, `control`, `daemon`, `exec`, `readiness`, `runtime`
and `vmm`.
Native task `plasmosome-qep`, which boots one real cell from the qualified Darwin runtime bundle,
carries the first part of that work: the vsock endpoints and their handshake.
