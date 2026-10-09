---
id: 024
title: The guest workload, and how cell.exec reaches it
status: draft
intents: [009, 003, 011]
---

## Behavior

A cell runs nothing of its own until it is asked to. When the guest boots, its trusted PID1
mounts the workload root, loads the guest policy, creates the workload's mount and network
namespaces and its control group, and says hello. It starts no workload. Every workload process
starts from `cell.exec`: the controller passes the request to the cell's `membraned`, which
passes it to PID1 over the private control channel as a new `exec` verb. PID1 starts the process
as an ordinary unprivileged Linux user inside the workload root, keeps the last 64 KiB of its
standard output and standard error, and gives it an ID. `exec.status` says how the process
ended, and `exec.output` returns what it printed. The process runs until it exits, the workload
is stopped, or the cell ends.

The process sees an ordinary, non-interactive Linux login: a user with a passwd entry, a
writable home directory, `HOME`, `LANG` and a fixed `PATH`, standard input on `/dev/null`, and no
terminal. It has no capabilities and cannot gain any. Things it may not do, such as opening the
control channel, creating a user namespace, mounting or tracing PID1, fail with an ordinary
error; the process is never killed for trying. When a plasmid is attached, its mount and its
proxied hosts appear inside the namespaces the process already runs in, without a restart. When
one is removed, the removal does not wait for the workload: the membrane revokes what backed the
grant, and a reference the process still holds into it fails with an ordinary error from then
on. A started process is not a capability: it is not journaled or observed as a grant, and
removing a plasmid does not stop it. It survives a controller restart when recovery re-adopts the
cell, because its state lives in PID1.

This serves three intents. Intent 009, a command line an agent can drive, needs a way to run a
command in a cell, learn how it ended and read what it printed; the control channel had no such
verb. Intent 003, cells come back after a crash, needs running work to survive the controller.
Intent 011, isolation the model never has to know about, decides the shape: no plasmosome
variables, no special errors, no restart when reach changes. This spec meets intent 011 for
non-interactive harnesses only. A terminal, and harnesses that need one, are outside this spec
and are an owner question. Subject spawns stay refused with code 110 until host-side attestation
exists, and output streaming stays reserved, as spec 001 already says.

**Platform.** The product is macOS first: Darwin arm64 is the only runtime host, and the guest is
Linux under libkrun. Linux/KVM host support is deferred, not dropped. PID1's hello reports the
effective guest policy, so no repo-built cell reaches ready, and no `exec` can run, before owner
decision O-7 is settled and its policy is loaded. Owner gates O-1 and O-6 also block a real boot.
Each acceptance item names its minimum level: the model (PID1's bookkeeping, or the controller
and membrane against a 4090 test double), the Darwin pinned runtime with a booted repo-built
guest, or a Linux host, which is deferred. The guest is always Linux; only a Linux host is
deferred.

## Contract

### 1. Public verbs

`cell.exec` takes `kernel`, `cell`, `argv`, and optional `deadline_ms`, `request_key` and
`subject`. There is no environment, directory or user parameter.

- `argv` is a nonempty array of NUL-free strings whose first element is nonempty. Anything else
  is `-32602`. A request line over spec 001's frame limit never reaches the verb: the connection
  answers `-32600`, as spec 001 §1 says.
- `deadline_ms` is an integer from 1 to 600,000, default 10,000. It bounds the whole request; the
  membrane and PID1 get what remains of it.
- `request_key` is 1 to 64 characters from `A-Z a-z 0-9 . _ -`. While PID1 retains an entry made
  with that key, a request with the same key and the same `argv` starts nothing and returns that
  entry's ID and current state. The same key with a different `argv` is refused with
  `key_conflict`. A client that retries after a lost reply uses the same key and runs the
  command once.
- `subject`, in any form, is code 110 `{verb: "cell.exec"}`. Nothing reaches the guest.
- A cell that is not `ready` is 105 `{from: <cell state>, to: "ready"}`. A cell that is unknown,
  retired or unaccounted (spec 023) is 101 `cell <id>`.
- Success is `{exec_id, state}`, where `state` is `running` for a new process. The reply means
  the process exists. It does not mean the program was found; see section 4.

`exec.status` and `exec.output` take `kernel`, `cell`, `exec_id` and optional `deadline_ms`
(as above). They are answered while the cell is `ready` or `draining`. `exec.status` returns one
of:

```json
{"exec_id": "e-11", "state": "running"}
{"exec_id": "e-11", "state": "exited", "exit_code": 0, "duration_ms": 1823}
{"exec_id": "e-11", "state": "signaled", "signal": 9, "duration_ms": 40}
```

`exit_code` is 0 to 255. `signal` is the Linux signal number, including signals that dump core.
`duration_ms` runs from the start of the process to its reap, on the guest's monotonic clock.

`exec.output` returns the retained output of both streams:

```json
{"exec_id": "e-11",
 "stdout": {"bytes": [111, 107, 10], "total": 3, "truncated": false},
 "stderr": {"bytes": [], "total": 0, "truncated": false}}
```

`bytes` is the last 65,536 bytes or fewer the stream received, as byte values like spec 001's
data verbs. `total` counts every byte received. `truncated` is true when `total` is larger than
the bytes kept. Output keeps arriving after the started process exits, for as long as a
descendant holds the stream open.

An `exec_id` that is not of the form `e-<n>` is `-32602`. One the cell never issued, or no longer
retains, is 101 `exec <id>`. Output that is no longer retained is 101 `output <id>`.

**Refusals carrying `detail`.** These are 105 with `from: "guest_request"`, `to: "refused"` and
exactly one of these `detail` records:

| `detail` | When |
| --- | --- |
| `{kind: "invalid_argv"}` | PID1 found `argv` invalid. |
| `{kind: "exec_limit"}` | 256 processes PID1 started are still running. |
| `{kind: "spawn_failed", errno: i32}` | The process could not be created; `errno` is the Linux value. No ID is issued. |
| `{kind: "busy"}` | `exec` waited for a grant transaction until its deadline. Nothing was started. |
| `{kind: "key_conflict"}` | The `request_key` is retained with a different `argv`. |
| `{kind: "stopped"}` | The workload has been stopped (section 7). |

When an `exec` was sent to the guest and no reply came within the deadline, the reply is 105
`{from: "guest_request", to: "unknown", detail: {kind: "no_reply"}}`. The process may have
started. A retry with the same `request_key` learns its ID, or starts it once.

None of the three verbs writes a journal record or takes part in spec 008's transactions.

### 2. Waiting

- `exec.status` and `exec.output` never wait for a grant transaction or a drain, at the
  controller, at the membrane or on the 4090 lane.
- `exec` waits for a grant transaction on the same cell that is in progress, at the controller
  and on the 4090 normal lane, within its deadline. At the deadline it is refused with `busy`, and
  nothing was sent to the guest.
- No grant transaction waits for an `exec` or a status request.
- None of them enters the withdrawal lane, so none can delay a Force.

### 3. The path from the controller to PID1

The controller sends four new membrane methods on `membrane.uds`:

| Method | Params | Result |
| --- | --- | --- |
| `membrane.cell.exec` | `{cell, argv, request_key: String\|null, deadline_ms}` | `{cell, exec_id, state}` |
| `membrane.exec.status` | `{cell, exec_id, deadline_ms}` | `cell` plus one `exec.status` result |
| `membrane.exec.output` | `{cell, exec_id, deadline_ms}` | `cell` plus one `exec.output` result |
| `membrane.workload.stop` | `{cell, deadline_ms}` | `{cell, stopped: true}` |

The membrane refuses another cell with 101, as spec 001 §4.1 says, and a cell that is not
`ready` or (for status, output and stop) `draining` with 105. Otherwise it relays the request to
PID1 on the normal lane of the original 4090 association and checks that the reply's `boot`
equals the original boot token. A different boot is an observation fault, `-32603`, never a
status. `membrane.workload.stop` is what spec 023's `cell.kill` sends first.

Four verbs are added to the 4090 table, all on the normal lane only:

| Method | Params | Result |
| --- | --- | --- |
| `exec` | `{argv: [String], request_key: String\|null, deadline_ms}` | `{boot, exec_id: String, state: ExecState}` |
| `exec_status` | `{exec_id: String, deadline_ms}` | `ExecStatus` |
| `exec_output` | `{exec_id: String, deadline_ms}` | `{boot, exec_id, stdout: Stream, stderr: Stream}` |
| `workload_stop` | `{deadline_ms}` | `{boot, stopped: true}` |

- `ExecState` is `running | exited | signaled`.
- `ExecStatus` is exactly one of `{boot, exec_id, state: "running"}`,
  `{boot, exec_id, state: "exited", exit_code: u8, duration_ms: u64}` and
  `{boot, exec_id, state: "signaled", signal: u8, duration_ms: u64}`.
- `Stream` is `{bytes: [u8], total: u64, truncated: bool}`, with at most 65,536 bytes.
- Every field is required and unknown fields refuse, like every other 4090 record. A guest
  refusal uses spec 001 §4.2's `{code: 105, message, from: "guest_request", to: "refused",
  detail}` with section 1's `detail` records.
- `exec_status`, `exec_output` and `workload_stop` are not mutations. PID1 answers them while a
  normal-lane mutation is outstanding, each under its own request ID. `exec` waits for an
  outstanding `install`, `activate` or `drain` to settle, within its deadline.

### 4. What PID1 does for `exec`

PID1 checks `argv` again and refuses a bad one with `invalid_argv`. It applies `request_key`
(section 1). It refuses with `exec_limit` when 256 processes it started are still running, and
with `stopped` after `workload_stop`. Then it creates the process. In the new process, before
`execve`, in this order:

1. a new session;
2. the workload control group (section 6), the workload mount namespace and the workload network
   namespace;
3. root and working directory inside the workload root (section 5), then working directory
   `/home/workload`;
4. every signal disposition set to its default, and an empty signal mask;
5. every resource limit set to the value the kernel gave PID1 at boot, except `RLIMIT_NOFILE`
   at 1,024 soft and 4,096 hard and `RLIMIT_CORE` at 0; umask 022; `oom_score_adj` 0;
6. uid 1000 and gid 1000, no supplementary groups, the effective, permitted, inheritable, ambient
   and bounding capability sets all empty, and no-new-privs set;
7. the seccomp filter of section 8;
8. descriptor 0 on `/dev/null`, descriptors 1 and 2 on the write ends of two pipes that PID1
   reads, and no other descriptor;
9. the environment exactly `PATH=/usr/local/bin:/usr/bin:/bin`, `HOME=/home/workload` and
   `LANG=C.UTF-8`;
10. `execvp(argv[0], argv)`, so the `PATH` search happens inside the workload root, after the
    root change. An `argv[0]` containing `/` is used as a path.

If no file exists for `argv[0]`, the process exits with 127. Every other `execve` failure exits
with 126: no execute permission, an unknown format, a missing interpreter, an argument list too
long, or a managed file refused by spec 017's policy. Those are the codes a shell gives. A
failure to create the process at all is `spawn_failed`, and no ID is issued. The host-side
`ExecCommand` in the membrane crate is not this path: it refuses a missing program before
forking.

PID1 issues `exec_id` as `e-<n>`, where `n` counts up from 1 for this boot. A cell is never
relaunched, so an ID is never reused for that cell. PID1 retains every running entry, and the
1,024 most recently finished entries; an older finished entry is dropped with its key. It keeps
output for running entries and the 128 most recently finished. An entry keeps its ID, key, a
SHA-256 digest of its `argv`, its state fields and its output while retained, and nothing else.

PID1 drains both pipes continuously, keeping the last 65,536 bytes of each, so a writer never
blocks on a full pipe. It reaps every child, including descendants that were handed to it. Only
the started process decides its entry's state. Descendants may outlive the started process; they
end with `workload_stop` or the cell.

### 5. The workload's namespaces and root

Before its hello, PID1 creates one workload mount namespace and one workload network namespace.
Every started process joins both. Neither is recreated while the cell lives. There is no PID
namespace, on purpose: a started process sees PID1 as pid 1 and sees other guest processes in
`/proc`, and section 8 denies it any way to act on them.

- Inside the workload mount namespace, PID1 moves the `/workload` mount over `/` with `MS_MOVE`,
  so the initramfs root, `/init` and `/plasmosome/guest-policy` are not reachable by any path.
  Each started process then has its root changed to that root and its directory changed inside
  it. This holds as long as no uid-1000 process ever runs outside that root and no directory
  descriptor from outside it reaches the workload; PID1 keeps both true.
- The namespace has a fresh `/proc`; a `/dev` holding only `null`, `zero`, `full`, `random`,
  `urandom`, the `fd`, `stdin`, `stdout` and `stderr` links, and a `shm` tmpfs (mode 1777); and
  nothing else mounted, `/sys` included. There is no `/dev/pts`.
- Managed mounts reach it by propagation: PID1 installs a Mount grant on its side, and the
  workload side, a slave of it, sees the mount. A process started before an attach sees the new
  mount without restarting. A detach removes the name for every process. Nothing the workload
  does propagates back.
- The network namespace holds only loopback until a ProxyMap is installed. The resolver at
  127.0.0.53 and the synthetic-prefix device of spec 001 §4.2 live in it, so a running process can
  reach a newly attached host without restarting.

**The image contract.** The workload root image must provide a passwd entry `workload` with uid
and gid 1000 and home `/home/workload`, that directory owned by 1000 with mode 0700, a group entry
for gid 1000, and a writable `/tmp` with mode 1777. PID1 checks these before hello. A root image
without them refuses the boot as a launch failure, and the cell never reaches ready.

### 6. The workload control group

Before hello, PID1 creates one cgroup v2 group for the workload and puts every started process in
it before `execve`. PID1 and its own helpers stay outside it.

- `pids.max` is 4,096.
- `memory.max` is the guest's memory less a reserve for PID1: the larger of 128 MiB and a tenth
  of the guest's memory.
- The guest kernel must provide the pids and memory controllers. PID1 checks them before hello,
  and refuses the boot as a launch failure without them.

So a fork bomb or a memory hog in the workload fails or is killed inside the group, and PID1
keeps answering. A fork refused by `pids.max` gets `EAGAIN`, an ordinary error.

### 7. Stopping the workload, and shutdown

**`workload_stop`.** PID1 marks the workload stopped, so later `exec` is refused with `stopped`.
It sends SIGTERM to every process in the workload group, descendants that started their own
sessions included. When the group is empty, or at the request's deadline, it writes 1 to the
group's `cgroup.kill`, reaps every process, and replies. If processes remain at the deadline, it
refuses with `{kind: "workload_remains", count: u32}`. Spec 023's `cell.kill` sends this first,
so the workload has stopped before any attachment is withdrawn.

**`shutdown`.** PID1 runs `workload_stop` with the shutdown deadline less a reserve for writing
the disk: the smaller of 2,000 ms and half the deadline. Then it syncs the root disk and powers
off. The host enforces the deadline from outside (spec 023): if the guest has not ended by then,
the helper is killed. So a process that ignores SIGTERM cannot delay the end of the cell past the
host's deadline.

### 8. What a started process cannot do

Each attempt below, made with arguments that would succeed without the denial, returns the stated
error, and the process keeps running. A policy that kills the process, for example with
`SIGSYS`, does not meet this.

| Attempt | Error | Mechanism |
| --- | --- | --- |
| `socket(AF_VSOCK, ...)` and `socketpair(AF_VSOCK, ...)` | `EAFNOSUPPORT` | seccomp filter on the family argument |
| any system call under a non-native ABI | `ENOSYS` | seccomp filter on the architecture |
| `io_uring_setup` | `EPERM` | `kernel.io_uring_disabled=2` |
| `unshare(CLONE_NEWUSER)`, `clone(CLONE_NEWUSER)`, `clone3` with `CLONE_NEWUSER` | `ENOSPC` | `user.max_user_namespaces=0` |
| creating any other namespace | `EPERM` | credentials |
| `mount`, `fsopen`, `open_tree`, `move_mount` | `EPERM` | credentials |
| `setns` on a descriptor of its own namespace | `EPERM` | credentials |
| `bpf(BPF_PROG_LOAD)` of a valid socket filter | `EPERM` | `kernel.unprivileged_bpf_disabled=2` |
| `ptrace(PTRACE_ATTACH, 1)` | `EPERM` | credentials |
| `kill(1, SIGTERM)`, and `pidfd_send_signal` on a pidfd for PID1 | `EPERM` | credentials |

`unshare(CLONE_FS | CLONE_FILES)` and other calls that create no namespace stay allowed. PID1
sets the three sysctls before hello. With io_uring disabled there is no second path to a vsock
socket that the seccomp filter cannot see.

None of these needs owner decision O-7: seccomp, sysctls and credentials are enough. Spec 017's
managed-file policy, which forbids mapping or directly executing a managed file, does need a
mechanism that sees files, and that is O-7. Because hello requires the guest policy to be loaded,
`exec` stays gated on O-7 as a whole.

### 9. When a grant is removed under a running process

A removal never waits for the workload, and never stops a started process. For a Mount grant,
destructive withdrawal, safe or Force, revokes the grant's backing:

1. The guest filesystem server stops serving every inode and handle bound to that grant. When the
   grant has its own connection, PID1 aborts that connection. When a peer shares the connection,
   the server fails every request on the grant's inodes and handles instead, and never aborts the
   shared connection (spec 017).
2. The host closes the grant's source root and every host handle opened for it.
3. The mount is detached, so the name is gone in both namespaces.

From then on, every use of a reference a process still holds into that mount, such as its working
directory, an open file or an open directory, fails with one named error. The expected value is
`ENOTCONN`; the exact value must be measured on the pinned runtime and is then asserted. The
process keeps running. Observation reports the grant's backing as revoked, and that state counts
as absent under spec 017, because the reference reaches nothing. So `plasmid.remove` completes
while a process sits with `cd /ws`.

The same question for SessionFile handles belongs to the spec that realizes SessionFile in the
guest.

### 10. What a started process is not

It is not one of spec 017's five classes. It has no `GrantId`, no journal record and no row in
`GuestObservation`, and residue snapshots do not list it. Removing a plasmid does not stop it;
a detach takes away only what that plasmid made visible. Spec 008's recovery does not reconstruct
it, and the controller keeps no record of it. After a controller restart, `exec.status` answers
from PID1 when recovery re-adopts the cell. A cell that recovery leaves quarantined or
unaccounted is 101 to every verb, so its processes cannot be queried until it is adopted.

## Changes proposed to accepted specs

These are proposed text, not edits made in this PR. `docs/specs/README.md` does not say that the
accepting PR applies them, and spec 001 says its text changes in a pull request with the
reasoning written down. So the PR that accepts this spec must carry these edits, or this spec
stays draft. Spec 023 changes the same 105 row of spec 001 §1, so the row below is the merged
text both specs propose.

**Spec 001 §1, the 105 row.** Replace the structured fields with:

> `from`, `to`; `detail` on a guest refusal, exactly one of spec 024's detail records; `cell`
> when a `cell.new` or `cell.kill` refusal leaves a cell in place (spec 023); private recovery
> methods additionally carry the typed `recovery` refusal in §4.1

**Spec 001 §3.8.** Replace "Run a command inside the cell. Requests an E13-style subject spawn
when a `subject` is named." with:

> Run a command inside the cell as its unprivileged workload user (spec 024). A named `subject`
> requests an E13-style subject spawn, which is refused with 110 until host-side attestation
> exists.

Replace the example request with:

> ```json
> {"id": 9, "method": "cell.exec",
>  "params": {"kernel": "work", "cell": "cell-1", "argv": ["git", "push"],
>             "request_key": "push-1"}}
> ```

After the `exec.status` example, add:

> ```json
> {"id": 11, "method": "exec.output",
>  "params": {"kernel": "work", "cell": "cell-1", "exec_id": "e-11"}}
> {"id": 11, "result": {"exec_id": "e-11",
>   "stdout": {"bytes": [111, 107, 10], "total": 3, "truncated": false},
>   "stderr": {"bytes": [], "total": 0, "truncated": false}}}
> ```

After the bullet about subject attestation and code `110`, add:

> - `argv` is a nonempty array of NUL-free strings with a nonempty first element; anything else
>   is -32602. Optional `deadline_ms` (default 10,000) bounds the request. Optional `request_key`
>   makes a retry return the same `exec_id` instead of starting a second process.
> - The process runs in the guest as uid 1000 with no capabilities, in the workload root, with
>   the environment exactly `PATH=/usr/local/bin:/usr/bin:/bin`, `HOME=/home/workload` and
>   `LANG=C.UTF-8`, working directory `/home/workload`, standard input on `/dev/null`, and no
>   terminal. A program that is not found exits 127, and one that cannot be executed exits 126.
> - Until host-side attestation exists, every `subject` is 110 and nothing reaches the guest.
> - A cell that is not ready is 105 `{from: <cell state>, to: "ready"}`; `exec.status` and
>   `exec.output` are also answered while the cell is draining. A guest refusal is 105
>   `{from: "guest_request", to: "refused", detail}`. An unknown or no longer retained `exec_id`
>   is 101.
> - `exec.status` states are `running | exited | signaled`. `exec.output` returns the last 65,536
>   bytes of each stream with its total. Neither waits for a grant transaction.
> - A started process is not a capability. It is not journaled, observed as a grant or stopped by
>   plasmid removal. It survives a controller restart when recovery re-adopts the cell, and ends
>   with the workload or the cell.

**Spec 001 §4.** After the `membrane.cell.kill` bullet, add:

> - `membrane.cell.exec`, `membrane.exec.status`, `membrane.exec.output` and
>   `membrane.workload.stop` relay §3.8 and spec 023's workload stop to the 4090 `exec`,
>   `exec_status`, `exec_output` and `workload_stop` verbs on the original association's normal
>   lane, check the reply's boot, and refuse with 105 when the cell's state does not admit them
>   (spec 024).

**Spec 001 §4.2, PID1.** Replace "and only then starts the unprivileged workload chrooted into
`/workload` in its controlled namespaces." with:

> and only then creates the workload's controlled mount and network namespaces and its control
> group. It starts no workload at boot. Every workload process is started by a 4090 `exec`
> (spec 024), with `/workload` moved over `/` in those namespaces.

**Spec 001 §4.2, workload confinement.** After "or authority to open AF_VSOCK, mount/setns, load
BPF or ptrace the shim.", add:

> Each such denial returns an ordinary error to the caller and never kills the process; spec 024
> lists each attempt, its error and its mechanism, io_uring and user namespaces included.

**Spec 001 §4.2, lanes.** Replace "No install, activate, drain or shutdown may enter the
withdrawal lane." with:

> No install, activate, drain, shutdown, exec, exec_status, exec_output or workload_stop may enter
> the withdrawal lane.

Replace "Readiness requires both hellos and the data association before workload launch." with:

> Readiness requires both hellos, the data association, the workload namespaces and the workload
> control group. No workload process exists before readiness.

After "later normal-lane mutations wait for that request to settle or return a bounded refusal
without effects.", add:

> `exec_status`, `exec_output` and `workload_stop` are not mutations and never wait for one;
> `exec` waits within its deadline and then refuses as busy (spec 024).

**Spec 001 §4.2, the 4090 table.** Add:

> | `exec` | `{argv:[String], request_key:String\|null, deadline_ms}` | `{boot, exec_id:String, state:ExecState}` |
> | `exec_status` | `{exec_id:String, deadline_ms}` | `ExecStatus` (spec 024) |
> | `exec_output` | `{exec_id:String, deadline_ms}` | `{boot, exec_id, stdout:Stream, stderr:Stream}` |
> | `workload_stop` | `{deadline_ms}` | `{boot, stopped:true}` |

**Spec 001 §4.2, shutdown.** After "Shutdown requests orderly guest exit;", add:

> PID1 first stops the workload as `workload_stop` does, leaving part of the deadline to sync the
> root disk, then powers off (spec 024).

**Spec 001 §5.** Replace "beyond §4.2's selected launch, observation, data and shutdown contract"
with:

> beyond §4.2's selected launch, observation, data and shutdown contract and spec 024's exec and
> workload verbs

**Spec 017, mounts (`017:411-412`).** After "For mounts, lazy detach alone does not establish
absent connections/handles or closed host access.", add:

> A Mount grant whose guest connection was aborted, or whose every inode and handle the guest
> server now refuses, with its host source root and handles closed, does count as absent even
> while workload references to it remain: every use of them fails (spec 024).

**Spec 011, detach (`011:195-206`).** After "the check runs where it can still change the outcome,
because a check that cannot is telemetry and this contract does not rest on one.", add:

> A reference whose backing the kernel has revoked, so that every use of it fails with an
> ordinary error, reaches nothing and is not a surviving reference (spec 024).

## Open questions

These need the owner. Where this spec needs an answer to work, it sets a conservative default.

1. **O-7, the guest policy mechanism.** Spec 017's managed-file policy needs it, and hello needs a
   loaded policy. `exec` cannot run on a repo-built cell until this is settled.
2. **A terminal, and interactive harnesses.** No `exec` has a terminal (the default), so a harness
   that needs one does not run. Should a later spec add a pseudo-terminal and an interactive
   stream, or is intent 011 met by non-interactive harnesses?
3. **An initial workload.** Should `cell.new` be able to start one process at boot, such as a
   harness, or is a `cell.exec` right after `cell.new` enough (the default)?
4. **Signalling one process.** There is no `exec.signal`. A runaway process ends only with
   `workload_stop` or the cell (the default). Should one exist?
5. **Subject attestation.** Every `subject` is 110 today. What would make a subject spawn
   admissible?
6. **More of the environment and image.** The contract is the three variables and section 5's
   image entries (the default). Should it grow, for example `TERM`, `/sys`, or variables a
   genome declares?

## Acceptance

Each item names the broken implementation it catches, and its minimum level: **model** (PID1's
bookkeeping in isolation, or the controller and membrane against a 4090 test double), or
**guest** (the Darwin pinned runtime with a booted repo-built guest, which needs O-1, O-6 and
O-7). Items marked **mount** also need the Mount adapter and its guest filesystem server.

1. **Guest.** `["true"]` ends `exited` 0, `["false"]` `exited` 1, `["sh", "-c", "kill -TERM $$"]`
   `signaled` 15, and `["sh", "-c", "kill -ABRT $$"]` `signaled` 6. Catches: folding signals into
   exit codes, including the core-dump ones, or the reverse.
2. **Guest.** `["no-such-program"]` replies `running`, then ends `exited` 127. A file in the image
   without execute permission, a script whose interpreter is missing, and a single argument of
   200 KiB each end `exited` 126. A program reached through an absolute symlink inside the image
   runs. Catches: refusing at `cell.exec`, and searching `PATH` from PID1's root.
3. **Guest.** A probe exits 0 only when all of these hold: uid and gid 1000, no supplementary
   groups, and `getpwuid(1000)` gives `/home/workload`; all five capability sets empty and
   no-new-privs set; the environment exactly the three variables; its directory
   `/home/workload`, writable; `/init` and `/plasmosome/guest-policy` absent, and a marker file
   present only in the image at `/`; `getsid(0)` equal to its pid; every signal at its default
   disposition, `SIGPIPE` included, and an empty mask; `RLIMIT_NOFILE` 1,024/4,096 and
   `RLIMIT_CORE` 0; umask 022; `oom_score_adj` 0; descriptor 0 on `/dev/null`, 1 and 2 on pipes,
   nothing else open; and membership of the workload group. A PID1 mutant that skips any one of
   these makes the probe fail. Catches: a missing root change, and leaking any part of PID1's
   state.
4. **Guest.** A probe makes each attempt in section 8's table with well-formed arguments, and
   exits 0 only if each returned exactly the stated error, including a vsock connect to CID 2 on
   ports 4090 and 4091 and `io_uring_setup`. It also checks that `unshare(CLONE_FS |
   CLONE_FILES)` succeeds. For each denial, a guest mutant without it makes that call succeed and
   the probe fail. A mutant that answers with `SIGSYS` makes the probe end `signaled`. Catches: a
   probe that fails for malformed arguments, a seccomp-only vsock denial with io_uring open, and
   killing a process for trying.
5. **Model.** A `subject` is 110, and a 4090 test double that fails the test on any `exec`
   request sees none. Catches: forwarding a subject spawn to the guest.
6. **Model.** `cell.exec` on a `germinating` cell (spec 023's held-hello helper) is 105 `{from:
   "germinating"}`; on a `draining` cell (a `cell.kill` held at its withdrawal step) 105 `{from:
   "draining"}`, while `exec.status` and `exec.output` still answer; on a `dead` cell (a guest
   that crashed while `membraned` runs) 105 `{from: "dead"}`; on a retired cell 101. Catches:
   queuing an exec until the cell is ready, and refusing status during a kill.
7. **Model.** Keep `e-1` running and finish 1,025 others: `e-1` still answers `running`, `e-2` is
   101, and the last 1,024 answer. The output of the 129th most recent finished entry is 101
   `output <id>`. The 257th concurrent running exec is `exec_limit`, and the 256 others keep
   running. Catches: one table for running and finished entries, unbounded tables, and reused
   IDs.
8. **Guest.** Start a long-running process, SIGKILL the controller, restart it. Recovery adopts
   the cell, `exec.status` still says `running`, the process is alive, and `exec.output` returns
   what it printed before the restart. Catches: exec state or output kept in the controller, and
   a controller exit that ends started processes.
9. **Guest, mount.** A probe started before an attach waits for a file in the workspace target and
   exits 0 when the Mount appears. A second probe, also started before, keeps looking the file up
   by path; after the detach its lookups fail with `ENOENT`. A probe started after the detach
   cannot see the file. Catches: a per-process mount snapshot, a mount installed only in PID1's
   namespace, and a detach that misses running processes.
10. **Guest, mount.** `["sh", "-c", "cd /workspace && exec probe"]`, where the probe holds an open
    file there and loops on reading it and on `stat(".")`. `plasmid.remove workspace` completes
    without waiting for the probe. Afterwards each of those calls fails with the measured error,
    the probe is still running, and observation counts the grant absent. Repeat with `cell.kill`
    instead: the workload is stopped first, the withdrawal then completes, and the cell is
    retired. Catches: a removal that waits on the workload, a lazy detach that leaves the backing
    reachable, and a kill that withdraws before stopping the workload.
11. **Model.** A 4090 test double holds a `drain` reply. `exec.status` and `exec.output` answer at
    once. `exec` waits, then is 105 `busy` at its deadline, and the double saw no `exec`. Then the
    double holds an `exec` reply: a Force removal and a new attach both complete. Send `exec` on
    the withdrawal lane: it is refused. Catches: status that waits behind a drain, and an exec on
    a path that blocks withdrawal or grants.
12. **Guest, mount.** A probe traps SIGTERM, writes a marker into a Mount, and ignores SIGTERM
    after that; its child calls `setsid` and does the same. `cell.kill` with `deadline_ms`
    10,000 replies within 10 seconds, both markers exist, and the guest powered itself off before
    the host had to kill the helper. Catches: SIGKILL without SIGTERM first, missing descendants
    in their own sessions, and a shutdown that relies on the host's kill.
13. **Guest.** In a fresh ready cell, the first started probe lists every process. It exits 0 only
    if the list holds itself, PID1 and its threads, kernel threads and the shim's declared helpers,
    and the workload group holds only the probe. Catches: a workload started at boot, under any
    uid.
14. **Guest.** After several execs, the journal is unchanged, `membrane.residue.snapshot` and
    `GuestObservation` list none of them, and removing every plasmid leaves them running.
    Catches: treating a started process as a grant.
15. **Model.** `argv` of `[]`, `[""]`, a string containing NUL, or a non-array is `-32602`; a line
    over the frame limit is `-32600`; an `exec_id` of `x` is `-32602`; `e-999`, never issued, is
    101 `exec e-999`. Catches: argument checks left to the guest.
16. **Model.** A 4090 test double that replies with a different `boot` makes `cell.exec`,
    `exec.status` and `exec.output` fail with `-32603`. A reply missing a field, or carrying an
    unknown one, also fails. Catches: a membrane that never compares the boot, and lax records.
17. **Guest.** `["sh", "-c", "sleep 600 & exit 3"]` ends `exited` 3 within a second, while a later
    probe still sees the `sleep`. `membrane.workload.stop` then ends it, and a following `exec` is
    105 `stopped`. Catches: an entry that waits for descendants, and an exec admitted after the
    stop.
18. **Model.** PID1's startup sends hello only after the workload namespaces, the control group and
    the image checks succeeded. A fake that fails any of them gets no hello. Catches: readiness
    before the workload can run.
19. **Guest.** `["sh", "-c", "printf out; printf err >&2"]` gives `stdout` "out" and `stderr`
    "err", each with `total` 3 and not truncated. A process writing 10 MiB to stdout finishes,
    with `total` 10,485,760, `truncated` true and the last 65,536 bytes kept. `exec.output`
    answers while a process still runs. Catches: unbounded buffers, a writer blocked on a full
    pipe, and mixed streams.
20. **Model.** PID1 given the same `request_key` and `argv` twice starts one process and returns
    one ID; the same key with another `argv` is `key_conflict`. Through a 4090 test double that
    drops the first reply, a client retry with the key gets the first ID. Catches: a retry that
    runs the command twice.
21. **Guest.** `["sh", "-c", ":(){ :|:& };:"]` runs. Meanwhile another exec's `exec.status`
    answers within one second and a Force removal completes. `workload_stop` ends the bomb. A
    process that allocates beyond `memory.max` ends `signaled` 9 while PID1 keeps answering. A
    spawn refused under `pids.max` is `spawn_failed` with `errno` 11 and issues no ID. Catches: a
    workload with no group limits, and no reserve for PID1.

## Out of scope

- Output streaming, which stays reserved in spec 001. Bounded capture is not streaming.
- A terminal, and interactive harnesses (open question 2).
- An initial workload at boot.
- Signalling one started process.
- Subject spawns and attestation.
- How attached software becomes visible inside a cell (`011:487-491`). It is bound by two rules
  here: `PATH` is fixed to three directories, and spec 017 forbids executing a managed file. So a
  plasmid's software can be found by name only if it lands, as files that are not managed files,
  in one of those directories.
- SessionFile handles held across a removal (section 9).
- How the cell and its supervisor start and end. That is spec 023.
