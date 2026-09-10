---
id: 018
title: Owned process-group supervision
status: draft
intents: [006, 012]
---

## Behavior

A host process handle must not abandon workers merely because their leader exited first.
`plasmosome_membrane::vmm::VmmChild` cleans its managed process group before releasing the
leader's identity, whether exit is first seen by `state`, `wait_terminal`, `kill`, or `Drop`.
An observed exit and completed cleanup are different events; only the latter may become a
successful cached terminal result.

This is supervision of host VMM and broker processes, not containment of arbitrary guest code.
The handle owns one direct child and the process group that child creates. When another reaper
has already consumed that child's status, the chosen response is **no further signal**, with
lost authority and possible surviving workers reported. It never guesses ownership from a
numerical PID or PGID after losing it.

Callers use `state` or `wait_terminal` to observe natural termination, and `kill` for explicit
synchronous teardown; dropping an unfinished handle performs the same teardown as a fallback.
These rules serve approved intent006's isolation security and intent012's short capability
lifetimes. They fill the local process-handle boundary left open by accepted spec001 §4–5;
they do not define its reserved broker, shim, desired-state, or residue wire verbs.

## Contract

### Managed processes and host requirements

The **leader** is the direct child returned by `fork`. Before invoking `Launch`, the child
creates a new session and group with `setsid`, with PGID equal to its PID. A **managed worker**
is a host helper that remains in that group and within the supervisor's signal permissions.
These host programs must not move workers into other groups/sessions, change credentials or
security labels to evade signalling, or keep creating workers during teardown. The supervisor
must run without a policy that conceals managed signal recipients as nonexistent.

This is a deployment requirement on trusted host launchers, not cooperation required of an
untrusted agent inside a cell. A guest's containment is the cell's separate OS/VM boundary;
this API does not create, verify, or replace that boundary. A configured host command that
runs arbitrary hostile code directly is not made safe by wrapping it in `VmmChild`. Escaped
descendants, continuous hostile forking, and permission-changing descendants are not covered
by a process-group guarantee. A session can contain several groups; ancestry is not membership.
Public documentation must say this instead of promising to kill everything ever forked.

The embedding host must make the handle the exclusive consuming waiter for its direct child
from spawn until completion. Other child owners may wait for their own exact PIDs, but must not
use a catch-all waiter that can consume this child. `SIGCHLD` must not be explicitly ignored,
use `SA_NOCLDWAIT`, or have a handler that reaps this child; this disposition must remain stable.
These are process-wide requirements, not properties enforced by Rust's mutable borrow.
The shipped `membraned` must satisfy them; its SIGINT/SIGTERM shutdown handling remains allowed.

This retains the existing exclusive-reaper requirement rather than adding a helper process,
background thread, global wait registry, or Linux-only supervision backend. It does not claim
safety against a hostile in-process reaper that violates it during a lifecycle call. Such a
reaper can release identity between a successful peek and a signal; no portable sequence of
`waitid`, `getpgid`, `getsid`, `kill(pid, 0)`, or start-time checks closes that interval. An
isolated owning parent or an appropriate stable OS authority would be a different contract.
The already-lost branch below is nevertheless supported and must fail closed consistently.

### Public API and results

Keep `Launch::launch(self) -> !`, `VmmChild::spawn(impl Launch) -> Result<VmmChild, SpawnError>`,
`SpawnError::ForkFailed(io::Error)`, and diagnostic `pid(&self) -> i32`. Keep `VmmState` variants
`Running`, `Exited { code: i32 }`, `Signaled { signal: i32 }`, and `Lost`. Change observation and
teardown results together, without compatibility wrappers:

```rust
pub fn state(&mut self) -> Result<VmmState, SupervisionError>;
pub fn wait_terminal(&mut self, deadline: Duration) -> Result<VmmState, SupervisionError>;
pub fn kill(&mut self) -> Result<(), SupervisionError>;
```

`SupervisionError` exposes `operation: SupervisionOperation`, `errno: i32`, and
`observed: Option<VmmState>`, and implements `Debug`, `Display`, and `std::error::Error`.
`SupervisionOperation` is `Observe`, `SignalLeader`, `SignalGroup`, or `Reap`.
The record and operation enum are copyable and equality-comparable. An OS error retains its
actual errno. An impossible successful wait response is an `Observe` or `Reap` error with
`EPROTO`, not a fabricated running or terminal state. `observed` preserves a known leader exit
when cleanup fails; it is not a successful cleanup result.

`Running` means no terminal event was available at the last successful nonblocking observation.
`Exited` preserves the natural eight-bit exit code, and `Signaled` preserves the terminating
signal, including core-dump termination as a signal rather than an exit code. A successful
terminal result means the group received its cleanup disposition and the direct child was
reaped. It is not an instantaneous barrier proving every grandchild has finished exiting.
SIGKILL is OS enforcement, but scheduling and uninterruptible kernel work can delay death.
Only the direct child is reaped by this handle; adopted descendants are reaped by the system.

`Lost` means a wait measured `ECHILD`: the handle no longer has a waitable direct child. It does
not assert that no worker exists, that the PGID was reused, or that no exit was previously known.
An initial loss is returned by `state`/`wait_terminal` as `Ok(Lost)`; `kill` returns the saved
`Observe`/`ECHILD` error. Loss after observing exit returns a saved error with that exit in
`observed`, including on subsequent calls. Both forms permanently prohibit PID operations.
The public PID is diagnostic only; exposing it does not transfer wait or signal authority.

### Identity and completion ordering

Before any signal, an unfinished operation observes only its exact direct child using
`waitid(P_PID, pid, ..., WEXITED | WNOHANG | WNOWAIT)`. Zero `siginfo_t` before every attempt,
including interrupted retries: older Darwin behavior can leave a WNOHANG output untouched.
A successful `si_pid == 0` means no event; a matching PID with a terminal `si_code` is an exit
observation, not permission to cache a completed state. Never decode output after `EINTR`.

A live direct child or its unreaped zombie retains its numerical identity. Under exclusive
reaping, retaining that child across the group signal prevents the leader PID from being
allocated to an unrelated process that could create a new group with the same number. `WNOWAIT`
retains waitability but supplies no lock after the syscall returns. This ownership interval,
not a later existence check, is the authority for all numerical signals in this contract.

If the leader has already terminated, send `SIGKILL` to `-pid` **before** consuming its exit
status. Never send a positive-PID signal to an already observed terminal leader. Then reap only
that leader and cache its actual status. This ordering is identical for all four entry points;
`state` and `wait_terminal` cannot consume the leader first and leave cleanup to `Drop`.

If `kill` or `Drop` first sees a running leader, send `SIGKILL` to `+pid`, wait non-consumingly
for its termination with `WEXITED | WNOWAIT`, then perform the group signal and direct-child
reap above. The final group signal occurs after the leader can no longer create its group or
fork a worker. This closes immediate teardown between parent-side fork return and child-side
`setsid`: a group signal attempted only before killing the leader is insufficient.
A successful positive signal is not enough to ignore a failed group signal.

Group signal success means a signal was sent to permitted current members. `ESRCH` while the
terminal leader is still owned means no group recipient was found; on the supported host this
is an allowed cleanup disposition, including a child killed before `setsid`. Neither result
proves arbitrary descendant absence. All other signal errors remain errors. Permission over
all managed members is a host prerequisite because POSIX group-kill success can mean that only
one member was permitted. A host with recipient-hiding policy cannot interpret ESRCH this way
and is outside this contract, rather than silently being advertised as supported.

On completed normal cleanup, repeated `state`/`wait_terminal` return the same terminal state;
`kill` succeeds as a no-op and `Drop` does nothing. No cached completion can cause another wait
or signal. A merely observed terminal leader with cleanup pending must not take this shortcut.

### Competing reapers and errors

If the first observation returns `ECHILD`, cache loss and send **neither** `+pid` nor `-pid`
any signal, now or later. All entry points use this same disposition, including `Drop` after
`state` cached `Lost`, and `kill` after `wait_terminal` cached it.

The reason is specific: the original native023 measurements showed a worker surviving its
reaped leader in PGID 60713, a successful group signal killing it, and a separate 433,910-fork
run not recycling the leader number while that group survived. Reaping does not immediately
make an extant worker group's ID reusable. But after its last member exits or leaves, the
number can be allocated to a stranger that creates a new group. A check followed by a later
signal cannot distinguish that replacement safely. This contract deliberately forgoes recovery
of the measured surviving group when leader authority is lost, rather than risking that signal.
Those are recorded historical measurements, not new verification of this proposal.

`ECHILD` encountered after a successful peek also ends all further PID operations, saves a
loss error with any known exit, and reports a violated exclusive-reaper requirement. Detection
after signalling cannot undo a signal or prove the preceding race was safe. No stronger
concurrent-reaper claim may be made from that case.

`state` does not wait for a running child. A nonblocking observation or reap interrupted with
`EINTR` returns an error and retains unfinished ownership. `wait_terminal` polls `state` with
an elapsed-time budget, sleeping at most the remaining budget; it retries `EINTR` only within
that budget and returns other errors immediately. On timeout it returns the last observation,
including an error if that was the last attempt, not a fabricated `Running`. A zero budget
still permits one nonblocking observation. No deadline cancels an owned child or discards its
pending cleanup. These durations are polling budgets, not hard kernel scheduling guarantees.

`kill` is synchronous for leader termination and reap. Blocking waits retry `EINTR`; persistent
other errors are returned rather than spun on. An error retains any still-owned child and known
exit for a later call or `Drop` to finish. Nonblocking reap returning no event after an exit
peek is incomplete (`Reap`/`EPROTO`), not successful completion. Each retry of unfinished work
must re-establish the same exact-child observation before signalling; a saved exit alone is not
a new ownership check. A reap returning `ECHILD` never authorizes another signal.

`Drop` invokes the same teardown. It does not panic or turn a cleanup error into success. If a
terminal leader cannot have its group signalled, Drop still attempts to reap that direct child,
reports possible workers, and never signals after releasing identity. If signalling a live
leader fails, Drop performs at most a nonblocking follow-up observation/reap, not an indefinite
wait for a child it failed to stop. If a non-EINTR wait error prevents reaping, it reports the
possible unreaped child as well. No guarantee is made for `mem::forget`, parent abort/SIGKILL,
or an uninterruptible child that prevents a successful blocking teardown from returning.

Drop-only failures, including a cached loss, attempt a non-panicking parent-side stderr
diagnostic containing the diagnostic PID, operation, errno, any observed exit, possible live
leader/workers, and whether further signalling is forbidden. Exact prose is not an API.
Reporting is best effort: closed stderr can lose it and blocked stderr can delay it; it is
not durable residue accounting. Callers needing an inspectable result must explicitly call
`kill` or observe termination before dropping. No unimplemented residue wire verb may be cited
as if it supplied that evidence, and a daemon's ordinary return is not proof of exceptional
cleanup success merely because its destructors ran.

### Startup and scope limits

`spawn` reports fork success, not completed session setup or successful exec. The child must
check `setsid`; on failure it calls `_exit(71)` before `Launch`, without logging, allocation,
or unwinding. This code is observable as ordinary exit 71, not a distinct `SpawnError`, and
is intentionally indistinguishable from a launched program choosing 71. POSIX fork's PID
allocation rule excludes the documented setsid EPERM cases on the normal path; this is a
fail-closed policy, not a claim that a real conforming-kernel failure can be reproduced.
The existing pre-exec async-signal-safe/no-panic `Launch` requirement remains in force.

This specification does not add proactive polling to `BrokerSet::status`, a cleanup timer,
controller recovery, a spawn handshake, a per-descendant enumeration/reaper, cgroups, pidfds,
or new control-protocol verbs. Cleanup starts when a lifecycle method or destructor runs.
It changes the local observation/kill result contract and guarantees cleanup of the ordinary
leader-first managed-worker cases; it does not label a group signal as general containment.

### Source basis

- [POSIX fork](https://pubs.opengroup.org/onlinepubs/9799919799/functions/fork.html): unique child identity, active-PGID exclusion, independent scheduling, and pre-exec restrictions.
- [POSIX waitid](https://pubs.opengroup.org/onlinepubs/9799919799/functions/waitid.html): WNOWAIT, exact-child waits, WNOHANG, ECHILD, EINTR, and automatic reaping.
- [POSIX kill](https://pubs.opengroup.org/onlinepubs/9799919799/functions/kill.html): group membership, permission success, recipient-hiding policy, and ESRCH.
- [POSIX setsid](https://pubs.opengroup.org/onlinepubs/9799919799/functions/setsid.html): initial session/group and its failure conditions.
- [Apple XNU wait implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_exit.c): the prior native023 source investigation found that WNOWAIT bypasses reap but releases the wait lock on return; this is not identification of an installed kernel.
- [Linux wait documentation](https://man7.org/linux/man-pages/man2/waitpid.2.html): Linux status and waitability semantics; Darwin and Linux runtime evidence must be reported separately.

## Acceptance

1. **Drop first:** a leader exits naturally with a witnessed live managed worker; dropping its still-unobserved handle causes worker-pipe EOF within the fixture deadline and leaves no waitable direct child.
2. **State first:** `state` first observes that exit, returns the natural exit code after cleanup/reap, and the worker-pipe witness reaches EOF; later Drop does not reoperate on the released identity.
3. **Wait first:** `wait_terminal` first observes the same leader-first case and satisfies the same worker, natural-status, and direct-reap assertions before later Drop.
4. **Kill first:** `kill` first sees an already exited leader, cleans the worker group, reaps the leader, and preserves its natural terminal status for later observations.
5. **Regression evidence:** the leader-first Drop, state, and wait cases are watched failing against pre-fix source and passing after repair, with exact source revisions and outputs recorded; historical native023 results do not substitute for this proof.
6. **Lost first and cached:** an external waiter consumes the leader before each lifecycle entry point; state/wait report Lost, kill reports ECHILD, and Drop reports possible workers without signalling; repeat after cached loss and witness the worker surviving until safe fixture release.
7. **Released identity:** repeated successful terminal and lost operations do not affect an independently owned sentinel; a deliberate signal-after-loss mutation fails the lost-worker case, without host PID-exhaustion or guessed-PID cleanup.
8. **Ownership race limit:** source review verifies the retained-child-through-signal interval and shipped-host reaper/disposition requirements; controlled post-peek loss reports a violation and prohibits subsequent signals, without claiming to prove safety against arbitrary concurrent reaping or actual PGID reuse.
9. **Startup gap:** immediate kill/Drop, including a deliberately held pre-setsid child in an isolated instrumented fixture, completes without a surviving managed worker or direct-child zombie; evidence distinguishes that instrumentation from a real setsid failure.
10. **Failure and retry:** observation, group-signal, and reap failures retain the operation/errno/known exit, cannot become successful cached completion, and either retry with owned identity or become permanent loss; Drop's persistent-error path reports residue and never waits indefinitely after a failed live-leader signal.
11. **Status and budgets:** running WNOHANG, normal exit, signal/core termination, EINTR, zero/expired polling budgets, and interrupted incomplete reap preserve their defined observations; bounded signal pressure proves actual delivered signals and interrupted blocking waits rather than an unexercised retry branch.
12. **Reporting and integration:** public-consumer and real `membraned` shutdown scenarios demonstrate leader-first worker cleanup and readable loss/error diagnostics; closed-report-channel limitations are disclosed, and existing broker partial-spawn/drop and readiness behavior remain intact.
13. **Accurate boundary:** public API, membrane ownership documentation, existing competing-reaper regression, and affected callers agree on managed groups, exclusive reaping, no-signal loss, synchronous kill, and the lack of implemented residue observation; none promises escaped-descendant containment.
14. **Portable proof:** lifecycle fixtures run separately on macOS and Linux with kernel/platform and source revisions recorded; bounded worker-pipe EOF proves no live fixture worker, exact-child ECHILD proves direct reap only, and safe descriptor-controlled cleanup leaves no owned fixture child unreaped.
