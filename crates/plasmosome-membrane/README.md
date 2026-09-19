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
built: the modules present are `brokers`, `control`, `daemon`, `exec`, `readiness` and `vmm`.
They arrive in the next P1 step.
