---
id: 024
title: The guest workload, and how cell.exec reaches it
status: accepted
intents: [009, 003, 011, 004]
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
one is removed, the removal does not wait for the workload: the grant's backing is revoked, and a
reference the process still holds into it fails with an ordinary error from then on. A write to a
stream whose peer is gone also raises `SIGPIPE`, as on any Linux system. A started
process is not a capability: it is not journaled or observed as a grant, and removing a plasmid
does not stop it. It survives a controller restart when recovery re-adopts the cell.

This serves intent 009, a command line an agent can drive: it can run a command in a cell, learn
how it ended and read what it printed. It serves intent 003: running work survives the
controller. Intent 011, isolation the model never has to know about, decides the shape: no
plasmosome variables, no special errors, no restart when reach changes. This spec meets intent
011 for non-interactive harnesses only; a terminal is an owner question. It serves intent 004 by
saying when a removed Mount grant counts as gone while a process still holds a reference into it,
and what a workload's call returns when any grant stops serving it.
Subject spawns stay refused with code 110 until host-side attestation exists, and output
streaming stays reserved, as spec 001 says.

**Platform.** macOS first: Darwin arm64 is the only runtime host, and the guest is Linux under
libkrun. A Linux host is deferred, not dropped. PID1's hello reports the effective guest policy,
so no repo-built cell reaches ready, and no `exec` runs, before owner decision O-7 is settled.
Owner gates O-1 and O-6 also block a real boot.

## Contract

### 1. Public verbs

`cell.exec` takes `kernel`, `cell`, `argv`, and optional `deadline_ms`, `request_key` and
`subject`. There is no environment, directory or user parameter.

- `argv` is a nonempty array of NUL-free strings whose first element is nonempty. Anything else
  is `-32602`. A request line over spec 001's frame limit is `-32600`, as spec 001 §1 says.
- `deadline_ms` is an integer from 1 to 600,000, default 10,000. It bounds the whole request; the
  membrane and PID1 get what remains of it.
- `request_key` is 1 to 64 characters from `A-Z a-z 0-9 . _ -`. A key belongs to its cell and is
  shared by every client of that cell. While PID1 retains an entry made with that key, a request
  with the same key and the same `argv` starts nothing and returns that entry's ID and current
  state; the same key with a different `argv` is refused with `key_conflict`. Once the entry is
  dropped (§4), the key is forgotten and a retry starts the command again.
- `subject`, in any form, is code 110 `{verb: "cell.exec"}`. Nothing reaches the guest.
- A cell that is not `ready` is 105 `{from: <cell state>, to: "ready"}` at once, before any
  wait; spec 023 says when a cell is `germinating`, `draining` or `dead`. A cell that is unknown,
  retired or set aside (spec 023) is 101 `cell <id>`.
- Success is `{cell, exec_id, state}`, where `state` is `running` for a new process. The reply
  means the process exists, not that the program was found (§4).

`exec.status` and `exec.output` take `kernel`, `cell`, `exec_id` and optional `deadline_ms`
(as above). They are answered while the cell is `ready` or `draining`. `exec.status` also takes
optional `wait`, default false. With `wait: true` it replies once the process is no longer
`running`, or when its deadline leaves only a reply reserve of the smaller of 1,000 ms and half
the deadline, with the state at that moment; reaching the deadline is not an error. Every result
names `cell`. `exec.status` returns one of:

```json
{"cell": "cell-1", "exec_id": "e-11", "state": "running"}
{"cell": "cell-1", "exec_id": "e-11", "state": "exited", "exit_code": 0, "duration_ms": 1823}
{"cell": "cell-1", "exec_id": "e-11", "state": "signaled", "signal": 9, "duration_ms": 40}
```

`exit_code` is 0 to 255. `signal` is the Linux signal number. `duration_ms` runs from the start of
the process to its reap, on the guest's monotonic clock.

`exec.output` returns the retained output of both streams:

```json
{"cell": "cell-1", "exec_id": "e-11",
 "stdout": {"bytes": [111, 107, 10], "total": 3, "truncated": false, "closed": true},
 "stderr": {"bytes": [], "total": 0, "truncated": false, "closed": true}}
```

`bytes` is the last 65,536 bytes or fewer the stream received, as byte values like spec 001's
data verbs. `total` counts every byte received. `truncated` is true when `total` is larger than
the bytes kept. `closed` is true once PID1 has read end-of-file on that stream: no more output
will come. Output can keep arriving after the started process exits, while a descendant holds the
stream open.

An `exec_id` that is not of the form `e-<n>` is `-32602`. One the cell never issued, or no longer
retains, is 101 `exec <id>`. Output that is no longer retained is 101 `output <id>`.

**Refusals carrying `detail`.** These are 105 with `from: "guest_request"` and exactly one of
these `detail` records. `to` is `"refused"`, except for `no_reply`.

| `detail` | When |
| --- | --- |
| `{kind: "invalid_argv"}` | PID1 found `argv` invalid. |
| `{kind: "exec_limit"}` | 256 processes PID1 started are still running. |
| `{kind: "spawn_failed", errno: i32}` | The process could not be created; `errno` is the Linux value. No ID is issued. |
| `{kind: "busy"}` | `exec` waited for a grant transaction until its deadline. Nothing was started. |
| `{kind: "key_conflict"}` | The `request_key` is retained with a different `argv`. |
| `{kind: "stopped"}` | The workload has been stopped (§7). |
| `{kind: "workload_remains", count: u32}` | `workload_stop` killed and reaped, and `count` processes still remain. |
| `{kind: "no_reply"}`, with `to: "unknown"` | A request was sent to the guest and no reply came within the deadline. An `exec` may have started; a retry with the same `request_key` learns its ID, or starts it once. |

None of the three verbs writes a journal record or takes part in spec 008's transactions.

### 2. Waiting

- `exec.status` and `exec.output` never wait for a grant transaction or a drain, at the
  controller, at the membrane or on the 4090 lane. A waiting `exec.status` waits only for its
  process.
- `exec` waits for a grant transaction in progress on the same cell, at the controller and on the
  4090 normal lane, within its deadline. At the deadline it is refused with `busy`, and nothing
  was sent to the guest.
- `workload_stop` is a mutation, but it is exempt from waiting: `cell.kill` must not wait behind a
  stuck drain.
- At most 16 waiting `exec.status` requests per cell are outstanding at once, a provisional
  default. Beyond that, a request is answered at once as if `wait` were false. Waits use capacity
  of their own on the normal lane, never the capacity `install`, `activate` or `drain` need.
- No grant transaction waits for an `exec` or a status request. None of them enters the
  withdrawal lane, so none can delay a Force.

### 3. The path from the controller to PID1

The controller sends four new membrane methods on `membrane.uds`:

| Method | Params | Result |
| --- | --- | --- |
| `membrane.cell.exec` | `{cell, argv, request_key: String\|null, deadline_ms}` | `{cell, exec_id, state}` |
| `membrane.exec.status` | `{cell, exec_id, wait, deadline_ms}` | `cell` plus one `exec.status` result |
| `membrane.exec.output` | `{cell, exec_id, deadline_ms}` | `cell` plus one `exec.output` result |
| `membrane.workload.stop` | `{cell, deadline_ms}` | `{cell, stopped: true}` |

The membrane refuses another cell with 101, as spec 001 §4.1 says, and a cell that is not
`ready` or (for status, output and stop) `draining` with 105. Otherwise it relays the request to
PID1 on the normal lane of the original 4090 association and checks that the reply's `boot`
equals the original boot token. A different boot is an observation fault, `-32603`, never a
status. Spec 023's `cell.kill` sends `membrane.workload.stop`.

Four verbs are added to the 4090 table, all on the normal lane only:

| Method | Params | Result |
| --- | --- | --- |
| `exec` | `{argv: [String], request_key: String\|null, deadline_ms}` | `{boot, exec_id: String, state: ExecState}` |
| `exec_status` | `{exec_id: String, wait: bool, deadline_ms}` | `ExecStatus` |
| `exec_output` | `{exec_id: String, deadline_ms}` | `{boot, exec_id, stdout: Stream, stderr: Stream}` |
| `workload_stop` | `{deadline_ms}` | `{boot, stopped: true}` |

- `ExecState` is `running | exited | signaled`.
- `ExecStatus` is exactly one of `{boot, exec_id, state: "running"}`,
  `{boot, exec_id, state: "exited", exit_code: u8, duration_ms: u64}` and
  `{boot, exec_id, state: "signaled", signal: u8, duration_ms: u64}`.
- `Stream` is `{bytes: [u8], total: u64, truncated: bool, closed: bool}`, with at most 65,536
  bytes.
- Every field is required and unknown fields refuse, like every other 4090 record. A guest
  refusal uses spec 001 §4.2's `{code: 105, message, from: "guest_request", to: "refused",
  detail}` with §1's `detail` records.
- `exec_status` and `exec_output` are not mutations. They and `workload_stop` are answered while a
  normal-lane mutation is outstanding, each under its own request ID. `exec` waits for an
  outstanding `install`, `activate` or `drain` to settle, within its deadline.

### 4. What PID1 does for `exec`

PID1 checks `argv` again and refuses a bad one with `invalid_argv`. It applies `request_key`
(§1). It refuses with `exec_limit` when 256 processes it started are still running, and with
`stopped` after `workload_stop`. Then it creates the process with `clone3` and
`CLONE_INTO_CGROUP`, directly inside the workload control group (§6), so a refusal by `pids.max`
fails the creation with `EAGAIN`: that is `spawn_failed`, and no ID is issued.

In the new process, before `execve`, PID1's child:

- starts a new session, joins the workload mount and network namespaces with `setns`, and changes
  root and directory into the workload root (§5), ending in `/home/workload`;
- resets every signal disposition to its default and empties the signal mask;
- sets every resource limit to the value the kernel gave PID1 at boot, except `RLIMIT_CORE` 0 and
  an `RLIMIT_NOFILE` hard limit of at most 4,096; sets a umask that grants no write to group or
  others; and leaves `oom_score_adj` at 0 or above;
- becomes uid and gid 1000 with no supplementary groups and all five capability sets empty, then
  sets no-new-privs, then loads the seccomp filter of §8;
- after those, puts descriptor 0 on `/dev/null` and 1 and 2 on the write ends of two pipes PID1
  reads, with no other descriptor open;
- searches `PATH` for `argv[0]` inside the workload root (an `argv[0]` containing `/` is used as
  a path), taking the first regular file uid 1000 may execute, or else the first file that
  exists, then calls `execve` with the environment exactly `PATH=/usr/local/bin:/usr/bin:/bin`,
  `HOME=/home/workload` and `LANG=C.UTF-8`.

The order that matters: the namespaces, group and root change come before capabilities are
dropped; no-new-privs comes before seccomp; descriptors are set after both. If the search finds
no file, the process exits with 127. Any `execve` failure exits with 126: no execute permission, an
unknown format, a missing interpreter, an argument list too long, or a managed file refused by
spec 017's policy. There is no fallback that runs an unknown format through a shell. The
host-side `ExecCommand` in the membrane crate is not this path.

PID1 issues `exec_id` as `e-<n>`, where `n` counts up from 1 for this boot. A cell is never
relaunched, so an ID is never reused for that cell. PID1 retains every running entry and a bounded
number of finished ones, at least 1,024, dropping the oldest finished entry with its key. It keeps
output for running entries and a bounded number of finished ones, at least 128. An entry keeps its
ID, key, a SHA-256 digest of its `argv`, its state fields and its output while retained, and
nothing else.

PID1 drains both pipes continuously, keeping the last 65,536 bytes of each, so a writer never
blocks on a full pipe. It reaps every child, including descendants handed to it. Only the started
process decides its entry's state. Descendants may outlive it; they end with `workload_stop` or
the cell. PID1's own descriptor limit is above two per possible workload process (twice
`pids.max`) plus its own reserve, so orphaned streams cannot exhaust it.

### 5. The workload's namespaces and root

Before its hello, PID1 creates one workload mount namespace and one workload network namespace.
Every started process joins both. Neither is recreated while the cell lives. There is no PID
namespace, on purpose: a started process sees PID1 as pid 1 and sees other guest processes in
`/proc`, and §8 denies it any way to act on them.

- Inside the workload mount namespace, PID1 moves the `/workload` mount over `/` with `MS_MOVE`,
  so the initramfs root, `/init` and `/plasmosome/guest-policy` are not reachable by any path.
  Each started process then has its root changed to that root. This holds as long as no uid-1000
  process ever runs outside that root and no directory descriptor from outside it reaches the
  workload; PID1 keeps both true.
- The namespace has a fresh `/proc`; a `/dev` holding only `null`, `zero`, `full`, `random`,
  `urandom`, the `fd`, `stdin`, `stdout` and `stderr` links, and a `shm` tmpfs (mode 1777); and
  nothing else mounted, `/sys` included. There is no `/dev/pts`, `/dev/vda` or `/dev/vsock`.
- Managed mounts reach it by propagation: PID1 installs a Mount grant on its side, and the
  workload side, a slave of it, sees the mount. A process started before an attach sees the new
  mount without restarting. Nothing the workload does propagates back.
- The network namespace holds only loopback until a ProxyMap is installed. The resolver at
  127.0.0.53 and the synthetic-prefix device of spec 001 §4.2 live in it, so a running process can
  reach a newly attached host without restarting.

**The image contract.** The workload root image must provide a passwd entry `workload` with uid
and gid 1000 and home `/home/workload`, that directory owned by 1000 with mode 0700, a group entry
for gid 1000, and a writable `/tmp` with mode 1777. PID1 checks these before hello. A root image
without them refuses the boot as a launch failure, and the cell never reaches ready.

### 6. The workload control group

Before hello, PID1 creates one cgroup v2 group for the workload. Every started process is created in
it (§4); PID1 stays outside it and runs no helper process. The guest kernel must provide the pids
and memory controllers; PID1 checks them before hello and refuses the boot without them. The pinned
kernel's configuration has both.

- `pids.max` is 4,096 and `memory.max` is the guest's memory less a reserve for PID1, the larger
  of 128 MiB and a tenth of the guest's memory. These are provisional defaults the task may tune.
- What does not change: the reserve covers PID1's own worst case, which includes its output
  buffers (up to (256 + 128) × 2 × 64 KiB = 48 MiB), the guest file server and the proxy flows.

So a fork bomb or a memory hog fails or is killed inside the group, and PID1 keeps answering. A
fork refused by `pids.max` gets `EAGAIN`, an ordinary error.

### 7. Stopping the workload, and shutdown

**`workload_stop`.** PID1 marks the workload stopped, so a later `exec` is refused with `stopped`.
It sends SIGTERM to every process in the workload group, descendants in their own sessions
included. When the group is empty, or at the deadline less a reply reserve of the smaller of
1,000 ms and half the deadline, it writes 1 to the group's `cgroup.kill`, reaps every process,
and replies before the deadline. If processes remain after that reap, it refuses with
`workload_remains`. A process that ignores SIGTERM therefore cannot make the reply late.

**`shutdown`.** PID1 stops the workload as `workload_stop` does, ending early enough to sync the
root disk before the shutdown deadline, then syncs and powers off. The host enforces the deadline
from outside (spec 023): if the guest has not ended by then, the helper is killed.

### 8. What a started process cannot do

Each attempt below, made with arguments that would succeed without the denial, returns the stated
error, and the process keeps running. A policy that kills the process, for example with
`SIGSYS`, does not meet this.

| Attempt | Error | Mechanism |
| --- | --- | --- |
| `socket` or `socketpair` with any family but `AF_UNIX`, `AF_INET`, `AF_INET6` or `AF_NETLINK`, `AF_VSOCK` included, compared on the low 32 bits of the argument | `EAFNOSUPPORT` | seccomp allowlist |
| any system call under a non-native ABI (on aarch64, AArch32) | `ENOSYS` | seccomp filter on the architecture |
| `io_uring_setup` | `EPERM` | `kernel.io_uring_disabled=2` |
| `unshare(CLONE_NEWUSER)`, `clone(CLONE_NEWUSER)`, `clone3` with `CLONE_NEWUSER` | `ENOSPC` | `user.max_user_namespaces=0` |
| creating any other namespace | `EPERM` | credentials |
| `mount`, `fsopen`, `open_tree` with `OPEN_TREE_CLONE`, `move_mount` | `EPERM` | credentials |
| `setns` on a descriptor of its own mount or network namespace | `EPERM` | credentials |
| `bpf(BPF_PROG_LOAD)` of a valid socket filter | `EPERM` | `kernel.unprivileged_bpf_disabled=2` |
| `ptrace(PTRACE_ATTACH, 1)` | `EPERM` | credentials |
| `kill(1, SIGTERM)`, and `pidfd_send_signal` on a pidfd for PID1 | `EPERM` | credentials |

`unshare(CLONE_FS | CLONE_FILES)` and other calls that create no namespace stay allowed. PID1
sets the three sysctls before hello. With io_uring disabled there is no second path to a vsock
socket that the seccomp filter cannot see. An x86_64 guest would also need x32's system call bit
checked; only aarch64 is built today.

None of these needs owner decision O-7: seccomp, sysctls and credentials are enough. Spec 017's
managed-file policy, which forbids mapping or directly executing a managed file, does need a
mechanism that sees files, and that is O-7. A mechanism built on LSM hooks needs
`CONFIG_SECURITY`, which the pinned kernel leaves unset. Because hello requires the guest policy
to be loaded, `exec` stays gated on O-7 as a whole.

### 9. When a grant stops serving a running process

A removal never waits for the workload and never stops a started process. Destructive withdrawal
of a Mount grant, safe or Force, revokes the grant's backing, in one of two ways:

- **The grant has its own connection.** PID1 aborts it. The guest kernel then refuses every use
  of a reference into it, and the mount is detached.
- **A peer shares the connection at the same target** (spec 017's equal peers). The guest file
  server refuses every request on the withdrawn grant's inodes and handles. The mount stays,
  because the peer still serves it. The target's root belongs to neither grant, so a working
  directory at the target itself keeps working, and a fresh lookup reaches the peer. The mount
  is detached only when the last holding at the target goes.

Either way, the host closes the grant's source root and every host handle opened for it. Every
operation started after the removal replies, through a reference a process still holds into the
withdrawn grant, fails with one named error in both branches: a read, write, `stat`, directory
listing or lookup, through a working directory or an open descriptor. The expected value is
`ENOTCONN`; the implementing task measures it on the pinned runtime and records it here before
item 10 runs. An operation in flight at the revocation ends with an error too, within the
removal's deadline; the expected value is `ECONNABORTED`, measured and recorded the same way.
`close()` succeeds. The process keeps running.
After the last holding at a target goes, a lookup by path there fails with `ENOENT`. All of this
rests on spec 017's direct IO, zero cache timeouts and refusal of file-backed mappings
(`017:416-424`), which need O-7 (§8); without them, cached pages and mappings outlive the grant.

**A revoked binding.** Observation reports the binding as `revoked` while references to it
remain. It counts as absent, so `plasmid.remove` completes while a process sits in the target. It
accounts for its own guest object, the aborted connection or the withdrawn grant's inodes on a
shared connection, so that object is never unknown. It is not a holding at its target: when the
surviving peer is later removed, that peer's connection counts as its own and is aborted. Once the
removal has completed, the binding is exempt only from host-authority association, residue,
drift and readiness: no host authority remains to match it, and it never blocks readiness or a
later attach at the same target. The host side is checked against the supervisor's own inventory
keyed by GrantId (gates, source roots, handles, listeners and flows), as spec 001 §4.2's absence
check requires (`001:1040-1041`). It must hold nothing for that grant; a holding found there has
no journal record and is an unknown holding (`008:400-402`). The amendments to spec 001 and spec
008 below say the same.

**A binding that stands while the host refuses it.** A grant can stop serving while its guest
binding stands: a fence (draft spec 028), a graceful removal's reversible pause, or a Force
removal before guest cleanup. The host gate refuses (spec 001 §4.2's `grant_inactive`), and the
guest turns that into an ordinary error:

Every error in this table is expected, not yet measured: the implementing task measures each on
the pinned runtime and records it here, as for Mount above.

| Grant | Refused while standing: a fence, or Force before guest cleanup | During a graceful removal's reversible pause |
| --- | --- | --- |
| Mount | An operation through a reference bound to the grant: `ENOTCONN`, as above. A fresh lookup when the only grant at the target is refused: `ENOENT`. | Each operation, a fresh lookup included, fails at once with `ENOTCONN`, so a file that still stands never looks deleted. A failed operation destroys nothing, so after a restore new operations succeed. |
| UdsSocket or Broker stream | The stream is closed: a read returns end-of-file, and a write fails with `EPIPE`. A new connection is closed the same way as soon as it is accepted. | Nothing is closed, since the pause may be undone. Data waits, and a blocked read or write returns when the pause is restored, or gets the outcome on the left when the removal goes ahead. |
| ProxyMap flow | An established TCP flow is reset, so its next call fails with `ECONNRESET`. A new TCP connect fails with `ECONNREFUSED`, never a silent drop. A UDP datagram on a refused flow gets an ICMP port unreachable, so a connected socket's next call fails with `ECONNREFUSED`. DNS answers NXDOMAIN (`001:1068-1069`). | As for a stream. |

Two outcomes have kernel caveats. A Unix stream read can fail with `ECONNRESET` instead of
returning end-of-file, when PID1 closes its end while the workload's bytes sit unread. An
unconnected UDP socket is not told of the ICMP error unless it set `IP_RECVERR`, so its receive
waits, as for any lost datagram.

A write to a closed stream also raises `SIGPIPE`, as for any stream whose peer has gone, and a
process that has not ignored it ends. The same holds for a TCP flow: after the reset, the first
write fails with `ECONNRESET` and a later one raises `SIGPIPE`. The guest could prevent that only
by changing the process's signal dispositions, which §4 resets to the defaults, or by dropping
writes silently, so it does neither; a reader sees end-of-file and no signal. SessionFile handles
belong to the spec that realizes SessionFile in the guest.

**No call waits forever.** Each workload call on a refused, paused or removed grant returns
within a bound:

- one that starts after the gate closed fails at once, except on a paused stream or flow;
- one on a paused stream or flow waits, and returns by the pause's deadline at the latest, when
  the pause is restored or the removal goes ahead. A pause expires at its original host deadline
  (`001:993-995`), so the guest needs no deadline of its own;
- one already in progress when its grant is revoked or refused returns by that removal's
  deadline.

Host IO that cannot be interrupted by then keeps the removal incomplete, as spec 001 §4.2 says,
and the workload's call does not wait for it.

### 10. What a started process is not

It is not one of spec 017's five classes. It has no `GrantId`, no journal record and no row in
`GuestObservation`, and residue snapshots do not list it. Removing a plasmid does not stop it;
a detach takes away only what that plasmid made visible. Spec 008's recovery does not reconstruct
it, and the controller keeps no record of it. After a controller restart, `exec.status` answers
from PID1 when recovery re-adopts the cell. A quarantined or set-aside cell is 101 to every
verb, so its processes cannot be queried until it is adopted.

## Changes proposed to accepted specs

These are proposed text. The PR that accepts this spec edits no other accepted document. Each
amendment is applied by a later reviewed change to the document it amends, before any task that
relies on it. Where spec 023 proposes text for the same lines, the text below is the same.

**Spec 001 §1, the 105 row.** Replace "`from`, `to`; private recovery methods additionally carry
the typed `recovery` refusal in §4.1" with:

> `from`, `to`; `detail`, a record of a kind spec 001 defines, including spec 024's; `cell`
> whenever the refusal concerns a created cell (spec 023); private recovery methods additionally
> carry the typed `recovery` refusal in §4.1

**Spec 001 §3.8.** Replace "Run a command inside the cell. Requests an E13-style subject spawn
when a `subject` is named." with "Run a command inside the cell as its unprivileged workload user
(spec 024). A named `subject` requests an E13-style subject spawn, refused with 110 until
host-side attestation exists." In the example request, replace `"subject": "git"` with
`"request_key": "push-1"`. In both example results, replace `{"exec_id": "e-11",` with
`{"cell": "cell-1", "exec_id": "e-11",`. After the `exec.status` example, add:

> ```json
> {"id": 11, "method": "exec.output",
>  "params": {"kernel": "work", "cell": "cell-1", "exec_id": "e-11"}}
> {"id": 11, "result": {"cell": "cell-1", "exec_id": "e-11",
>   "stdout": {"bytes": [111, 107, 10], "total": 3, "truncated": false, "closed": true},
>   "stderr": {"bytes": [], "total": 0, "truncated": false, "closed": true}}}
> ```

After the bullet about subject attestation and code `110`, add:

> - The parameters, including `exec.status`'s `wait`, the workload's identity and environment,
>   exit codes, retained output and the refusals are spec 024's. Every result names `cell`. A
>   started process is not a capability: it is not journaled, observed as a grant or stopped by
>   plasmid removal.

**Spec 001 §4.** After the `membrane.cell.kill` bullet, add:

> - `membrane.cell.exec`, `membrane.exec.status`, `membrane.exec.output` and
>   `membrane.workload.stop` relay to the 4090 `exec`, `exec_status`, `exec_output` and
>   `workload_stop` verbs on the original association's normal lane (spec 024).

**Spec 001 §4.2, PID1.** Replace "then starts the unprivileged workload chrooted into `/workload`
in its controlled namespaces." with "then creates the workload's controlled mount and network
namespaces and its control group. It starts no workload at boot. Every workload process is
started by a 4090 `exec` (spec 024), with `/workload` moved over `/` in those namespaces."

**Spec 001 §4.2, workload confinement.** After "or authority to open AF_VSOCK, mount/setns, load
BPF or ptrace the shim.", add "Each such denial returns an ordinary error to the caller and never
kills the process (spec 024)."

**Spec 001 §4.2, lanes.** Replace "No install, activate, drain or shutdown may enter the
withdrawal lane." with "No install, activate, drain, shutdown, exec, exec_status, exec_output or
workload_stop may enter the withdrawal lane." Replace "one normal request may be outstanding
without blocking withdrawal dispatch." with "several normal requests may be outstanding at once;
spec 024 says which wait for an outstanding mutation, and none blocks withdrawal dispatch." Replace
"Readiness requires both hellos and the data association before workload launch." with
"Readiness requires both hellos, the data association, the workload namespaces and the workload
control group; no workload process exists before readiness." After "later normal-lane mutations
wait for that request to settle or return a bounded refusal without effects.", add "`exec_status`
and `exec_output` are not mutations; they and `workload_stop` never wait for one, and `exec` waits
within its deadline and then refuses as busy (spec 024)."

**Spec 001 §4.2, the 4090 table.** Add:

> | `exec` | `{argv:[String], request_key:String\|null, deadline_ms}` | `{boot, exec_id:String, state:ExecState}` |
> | `exec_status` | `{exec_id:String, wait:bool, deadline_ms}` | `ExecStatus` (spec 024) |
> | `exec_output` | `{exec_id:String, deadline_ms}` | `{boot, exec_id, stdout:Stream, stderr:Stream}` (spec 024) |
> | `workload_stop` | `{deadline_ms}` | `{boot, stopped:true}` |

**Spec 001 §4.2, shutdown.** After "Shutdown requests orderly guest exit;", add "PID1 first stops
the workload as `workload_stop` does, leaving time to sync the root disk (spec 024);".

**Spec 001 §4.2, guest removal (`001:1030-1031`).** After "the shim returns guest_removed:true
only after fresh observation proves the selected guest bindings absent.", add "A `revoked` Mount
binding counts as absent (spec 024)."

**Spec 001 §4.2, observation (`001:1203-1211`).** Replace "GuestAdmission is exactly `staged |
active | draining | closed`." with:

> GuestAdmission is exactly `staged | active | draining | closed | revoked`. `revoked` is a Mount
> binding whose backing is revoked under spec 024: the guest serves nothing for its grant and the
> host side is closed, while workload references to it may remain, and every use of them fails.
> A `revoked` binding accounts for its own physical object, the aborted connection or the
> withdrawn grant's inodes on a shared connection, so the inventory rules below still apply to
> that object. Once its removal has completed, it is exempt only from host-authority
> association, residue, drift and readiness: it needs no retained host authority, and it never
> blocks readiness or a later attach at the same target. The host checks, against its own
> inventory keyed by GrantId (gates, source roots, handles, listeners and flows), that it holds
> nothing for that grant, as withdrawal's absence check requires; a holding found there is an
> unknown holding under spec 008.

and after "Closed/staged attachments remain visible until physically removed;", add "a `revoked`
binding also remains visible until its last reference goes, and counts as absent for removal;".

**Spec 001 §5.** Replace "beyond §4.2's selected launch, observation, data and shutdown contract"
with "beyond §4.2's selected launch, observation, data and shutdown contract and spec 024's exec
and workload verbs".

**Spec 008, observation (`008:400-402`).** After "not empty projections or guessed grant IDs.",
add "A guest binding that spec 024 reports as `revoked`, after its removal completed, accounts
for its own guest object and is not drift, and recovery matches no journal record to it. A host
holding still found for that grant has no journal record and is unknown (spec 001 §4.2)."

**Spec 008, readiness (`008:552-553`).** After "an empty instance is ready only after the same
startup requirements.", add "A `revoked` guest binding (spec 024), after its removal
completed, is none of these."

**Spec 017, mounts (`017:411-412`).** After "For mounts, lazy detach alone does not establish
absent connections/handles or closed host access.", add:

> A Mount grant whose backing is revoked does count as absent, even while workload references to
> it remain: every use of them fails with one ordinary error, no host access remains, and
> observation reports it as revoked (spec 024).

**Spec 011, detach (`011:195-206`).** After "the check runs where it can still change the outcome,
because a check that cannot is telemetry and this contract does not rest on one.", add:

> A reference whose backing has been revoked, so that every use of it fails with an ordinary
> error, refused by the guest kernel when the grant had its own connection and by the guest file
> server when a peer shares it, reaches nothing and is not a surviving reference (spec 024).

## Open questions

These need the owner. Where this spec needs an answer to work, it sets a conservative default.

1. **O-7, the guest policy mechanism.** Spec 017's managed-file policy needs it, and hello needs a
   loaded policy. `exec` cannot run on a repo-built cell until this is settled.
2. **A terminal, and interactive harnesses.** No `exec` has a terminal (the default), so a harness
   that needs one does not run. Should a later spec add a pseudo-terminal and an interactive
   stream, or is intent 011 met by non-interactive harnesses?
3. **An initial workload.** Should `cell.new` be able to start one process at boot, such as a
   harness, or is a `cell.exec` right after `cell.new` enough (the default)?
4. **Subject attestation.** Every `subject` is 110 today. What would make a subject spawn
   admissible?

## Acceptance

Each item names the broken implementation it catches, and its minimum level: **model** (PID1's
bookkeeping in isolation, or the controller and membrane against a 4090 test double), or
**guest** (the Darwin pinned runtime with a booted repo-built guest, which needs O-1, O-6 and
O-7). Items marked **mount** also need the Mount adapter and its guest filesystem server.

1. **Guest.** `["true"]` ends `exited` 0, `["false"]` `exited` 1, `["sh", "-c", "kill -TERM $$"]`
   `signaled` 15, and `["sh", "-c", "kill -ABRT $$"]` `signaled` 6. Catches: folding signals into
   exit codes, or the reverse.
2. **Guest.** `["no-such-program"]` replies `running`, then ends `exited` 127. A file in the image
   without execute permission, a script whose interpreter is missing, a text file with no `#!`,
   and a single argument of 200 KiB each end `exited` 126. A program reached through an absolute
   symlink inside the image runs, and so does an executable `/usr/bin/x` behind a non-executable
   `/usr/local/bin/x`. Catches: refusing at `cell.exec`, searching `PATH` from PID1's root, a
   missing interpreter reported as a missing program, a shell fallback, and a search that stops
   at the first file.
3. **Guest.** A probe exits 0 only when all of these hold: uid and gid 1000, no supplementary
   groups, and `getpwuid(1000)` gives `/home/workload`; all five capability sets empty and
   no-new-privs set; the environment exactly the three variables; its directory
   `/home/workload`, writable; a marker file present only in the image at `/`; `getsid(0)` equal
   to its pid; every signal at its default disposition and an empty mask; `RLIMIT_CORE` 0, an
   `RLIMIT_NOFILE` hard limit of at most 4,096, a umask with no group or other write, and
   `oom_score_adj` of 0 or more; descriptor 0 on `/dev/null`, 1 and 2 on pipes, nothing else open;
   and membership of the workload group. A test hook that joins the workload mount namespace
   from PID1's side, without changing root, reads its mountinfo: the image is mounted at `/`, and
   `/init` is absent. Catches: a missing `MS_MOVE` or root change, and leaking PID1's state.
4. **Guest.** A probe makes each attempt in §8's table with well-formed arguments, and exits 0
   only if each returned exactly the stated error. That includes a vsock connect to CID 2 on ports
   4090 and 4091, `socket(0x1_0000_0028, SOCK_STREAM, 0)`, and `io_uring_setup`. It also checks
   that `unshare(CLONE_FS | CLONE_FILES)` and an `AF_INET` socket succeed. For each denial, a
   guest mutant without it makes that call succeed and the probe fail. A mutant that answers with
   `SIGSYS` makes the probe end `signaled`. Catches: a probe that fails for malformed arguments,
   a family check on all 64 bits, a seccomp-only vsock denial with io_uring open, and killing a
   process for trying.
5. **Model.** A `subject` is 110, and a 4090 test double that fails the test on any `exec`
   request sees none. Catches: forwarding a subject spawn to the guest.
6. **Model.** `cell.exec` on a `germinating` cell (spec 023's held-hello helper) is 105 `{from:
   "germinating"}`; on a `draining` cell (a `cell.kill` held at its withdrawal step) it is 105
   `{from: "draining"}` at once, not `busy` or `stopped`, while `exec.status` and `exec.output`
   still answer; on a `dead` cell 105 `{from: "dead"}`; on a retired cell 101. Catches: queuing
   an exec until the cell is ready, and refusing status during a kill.
7. **Model.** With PID1's finished-entry bound N and output bound M: keep `e-1` running and finish
   N + 1 others. `e-1` still answers `running`, `e-2` is 101, and the last N answer. The output of
   the M + 1st most recent finished entry is 101 `output <id>`. The 257th concurrent running exec
   is `exec_limit`, and the 256 others keep running. Catches: one table for running and finished
   entries, unbounded tables, and reused IDs.
8. **Guest.** Start a long-running process, SIGKILL the controller, restart it. Recovery adopts
   the cell, `exec.status` still says `running`, the process is alive, and `exec.output` returns
   what it printed before the restart. Catches: exec state or output kept in the controller, and
   a controller exit that ends started processes.
9. **Guest, mount.** A probe started before an attach waits for a file in the workspace target and
   exits 0 when the Mount appears. A second probe, also started before, keeps looking the file up
   by path; after the last holding at the target is removed, its lookups fail with `ENOENT`. A
   probe started after that cannot see the file. Catches: a per-process mount snapshot, a mount
   installed only in PID1's namespace, and a detach that misses running processes.
10. **Guest, mount.** With the error measured and recorded in §9 first:
    `["sh", "-c", "cd /workspace && exec probe"]`, where the probe holds an open file there and
    loops on reading it and on `stat(".")`. `plasmid.remove workspace` completes without waiting
    for the probe. Then each call started after the reply fails with the recorded error, `close()`
    succeeds, the probe still runs, and observation reports the binding `revoked`. The
    supervisor's own inventory holds no gate, source root, handle, listener or flow for the
    removed grant. SIGKILL the controller and restart it while the probe still holds them:
    recovery adopts the cell ready, and adding `workspace` again succeeds at the same target. A
    host that kept the source root open fails this item. Repeat with two equal Mount grants
    at one target: the probe opens a file and enters a directory below the target through the
    grant with the smaller GrantId, and that grant is removed. Those references fail with the
    same error, while the target's root, a fresh lookup and an open handle through the peer still
    work. Catches: a removal that waits on the workload, a lazy detach that leaves the backing
    reachable, a host that keeps the grant's access, two errors for one state, a revoked binding
    that fails recovery or blocks a re-attach, and a detach that takes a peer's mount.
11. **Model.** A 4090 test double holds a `drain` reply. `exec.status`, `exec.output` and
    `workload_stop` answer at once. `exec` waits, then is 105 `busy` at its deadline, and the
    double saw no `exec`. Then the double holds an `exec` reply: a Force removal and a new attach
    both complete. Send `exec` on the withdrawal lane: it is refused. Catches: status that waits
    behind a drain, and an exec on a path that blocks withdrawal or grants.
12. **Guest, mount.** A probe traps SIGTERM, writes a marker into a Mount, and ignores SIGTERM
    after that; its child calls `setsid` and does the same. `cell.kill` with `deadline_ms` 10,000
    replies within 10 seconds, both markers exist, and the guest powered itself off before the
    host had to kill the helper. Repeat by sending `membrane.cell.kill` directly, with no workload
    stop first: the same holds through `shutdown`. `workload_stop` with a 2,000 ms deadline
    replies before it. Catches: SIGKILL without SIGTERM first, missing descendants in their own
    sessions, a reply after the deadline, and a shutdown that relies on the host's kill.
13. **Guest.** In a fresh ready cell, the first started probe lists every process. It exits 0 only
    if every other process is PID1, one of PID1's threads, or a kernel thread, and the workload
    group holds only the probe. Catches: a workload started at boot, under any uid or label.
14. **Guest.** After several execs, the journal is unchanged, `membrane.residue.snapshot` and
    `GuestObservation` list none of them, and removing every plasmid leaves them running.
    Catches: treating a started process as a grant.
15. **Model.** `argv` of `[]`, `[""]`, a string containing NUL, or a non-array is `-32602`; a line
    over the frame limit is `-32600`; an `exec_id` of `x` is `-32602`; `e-999`, never issued, is
    101 `exec e-999`. A PID1 fake given a bad `argv` directly refuses with `invalid_argv`.
    Catches: argument checks left to the guest, or only to the controller.
16. **Model.** A 4090 test double that replies with a different `boot` makes `cell.exec`,
    `exec.status` and `exec.output` fail with `-32603`. A reply missing a field, or carrying an
    unknown one, also fails. Catches: a membrane that never compares the boot, and lax records.
17. **Guest.** `["sh", "-c", "sleep 600 & exit 3"]` ends `exited` 3 within a second, with
    `closed` false while the `sleep` holds the streams. `membrane.workload.stop` then ends it, the
    streams report `closed`, a following `cell.exec` is 105 `{from: "draining"}` from the
    membrane, and a 4090 `exec` sent to PID1 directly is 105 `stopped`. Catches: an entry that
    waits for descendants, an end of output a client cannot see, and an exec admitted after the
    stop at either layer.
18. **Model.** PID1's startup sends hello only after the workload namespaces, the control group and
    the image checks succeeded. A fake that fails any of them gets no hello. Catches: readiness
    before the workload can run.
19. **Guest.** `["sh", "-c", "printf out; printf err >&2"]`, read once both streams report
    `closed`, gives `stdout` "out" and `stderr` "err", each with `total` 3 and not truncated. A
    process writing 10 MiB to stdout finishes, with `total` 10,485,760, `truncated` true and the
    last 65,536 bytes kept. `exec.output` answers while a process still runs. Catches: unbounded
    buffers, a writer blocked on a full pipe, and mixed streams.
20. **Model.** PID1 given the same `request_key` and `argv` twice starts one process and returns
    one ID, and again after that process has exited; the same key with another `argv` is
    `key_conflict`. Through a 4090 test double that drops the first reply, the client gets 105
    `{from: "guest_request", to: "unknown", detail: {kind: "no_reply"}}`, and a retry with the key
    gets the first ID. A PID1 fake whose reap leaves one process gives `workload_remains` with
    `count` 1. Catches: a retry that runs the command twice, and refusals outside the closed set.
21. **Guest.** `["sh", "-c", ":(){ :|:& };:"]` runs. Meanwhile another exec's `exec.status`
    answers within one second and a Force removal completes. `workload_stop` ends the bomb. With
    the group at `pids.max`, a new `exec` is `spawn_failed` with `errno` 11 and issues no ID. A
    process that allocates beyond `memory.max` is killed inside the group while PID1 keeps
    answering. 600 runs of item 17's pattern leave PID1 answering. Catches: a workload with no
    group limits, a spawn charged outside the group, no reserve for PID1, and PID1 running out of
    descriptors.
22. **Guest.** `exec.status` with `wait: true` on `["sleep", "1"]` replies `exited` 0 after about
    a second; on `["sleep", "600"]` with `deadline_ms` 2,000 it replies `running` before 2
    seconds. Every result of `cell.exec`, `exec.status` and `exec.output` names `cell`. With 16
    waits outstanding, a 17th answers at once with `running`, and an attach and a safe removal
    still complete. Catches: a wait that ignores its deadline, a deadline reported as an error, a
    result a client cannot attribute, and waits that starve grant transactions.
23. **Guest, with the host side of 4091 replaced by a test double.** Every error is the one
    recorded in §9.
    - The double refuses a standing binding's gate. A read through a bound Mount handle fails at
      once, and a fresh lookup at a target whose only grant is refused fails with `ENOENT`. An
      established stream gives end-of-file to a read. A process writing to it with the default
      disposition ends `signaled` 13, and one that ignores `SIGPIPE` gets `EPIPE`. A TCP flow is
      reset, and a new TCP connect fails at once.
    - The double receives a Mount read and never answers it, and the removal proceeds: the read
      returns with the recorded in-flight error (expected `ECONNABORTED`) by the removal's
      deadline.
    - The double pauses a Mount gate: a read through an open handle and a fresh lookup each fail
      at once with `ENOTCONN`. After the restore, the same open handle reads again.
    - The double pauses a stream's gate: a blocked read does not fail, and returns data once the
      double restores the gate. Paused again and then removed, the read returns end-of-file by
      the pause deadline.

    Catches: a silent drop, a dropped new connect, a call that waits forever, a Mount connection
    aborted for a pause, a standing file reported deleted, and a stream closed for a pause that
    is then undone.

## Out of scope

- Output streaming, which stays reserved in spec 001. Bounded capture is not streaming.
- A terminal, and interactive harnesses (open question 2).
- An initial workload at boot (open question 3).
- Signalling one started process, and environment variables or image entries beyond §4 and §5;
  a later spec may add them.
- Subject spawns and attestation.
- How attached software becomes visible inside a cell (`011:489-493`). It is bound by two rules
  here: `PATH` is fixed to three directories, and spec 017 forbids executing a managed file. So a
  plasmid's software can be found by name only if it lands, as files that are not managed files,
  in one of those directories.
- SessionFile handles held across a removal (§9).
- How the cell and its supervisor start and end, and how `cell.kill` divides its deadline. That
  is spec 023.
