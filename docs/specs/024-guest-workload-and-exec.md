---
id: 024
title: The guest workload, and how cell.exec reaches it
status: draft
intents: [009, 003, 011]
---

## Behavior

A cell runs nothing of its own until it is asked to. When the guest boots, its trusted PID1
mounts the workload root, loads the guest policy, creates the workload's mount and network
namespaces, and says hello. It starts no workload. Every workload process starts from
`cell.exec`. The controller passes the request to the cell's `membraned`, which passes it to PID1
over the private control channel as a new `exec` verb. PID1 starts the process as an ordinary
unprivileged Linux user inside the workload root and namespaces, and gives it an ID. `exec.status`
asks PID1 for that process's state through the same path. The process runs until it exits or the
cell ends.

The process sees an ordinary Linux machine. Its environment holds only a fixed `PATH`. Its
working directory is the workload root. Its standard descriptors are `/dev/null`. It has no
capabilities and cannot gain any. Things it may not do, such as opening the control channel,
creating a user namespace, mounting, loading BPF or tracing PID1, fail with an ordinary error,
the same as on any locked-down machine; the process is never killed for trying. When a plasmid
is attached, its mount and its proxied hosts appear inside the namespaces the process already
runs in, without a restart. A started process is not a capability: it is not journaled, it is not
withdrawn by removing a plasmid, and it does not appear in the guest's grant inventory. It
survives a controller restart, and `exec.status` keeps answering, because the state lives in PID1.

This serves three intents. Intent 009, a command line an agent can drive, needs a way to run a
command in a cell and learn how it ended; the control channel had no such verb. Intent 003,
cells come back after a crash, needs running work to survive the controller. Intent 011, isolation
the model never has to know about, decides the shape: the process gets no plasmosome variables
and no special errors, and needs no restart when its reach changes. Subject spawns stay refused
with code 110 until host-side attestation exists. Output streaming stays reserved, as spec 001
already says.

**Platform.** The product is macOS first: Darwin arm64 is the only runtime host, and the guest is
Linux under libkrun. Linux/KVM host support is deferred, not dropped. PID1's hello reports the
effective guest policy, so no repo-built cell reaches ready, and no `exec` can run, before owner
decision O-7 is settled and its policy is loaded. Owner gates O-1 and O-6 also block a real boot.
PID1's bookkeeping (IDs, retention, exit mapping) is testable without a guest. Evidence for each
acceptance item says which level it reached: the model, the Darwin OS mechanism, the Darwin
pinned runtime, or Linux, which is deferred.

## Contract

### 1. Public verbs

`cell.exec` keeps spec 001's parameters: `kernel`, `cell`, `argv` and optional `subject`. There is
no environment, directory, user or deadline parameter.

- `argv` is a nonempty array of NUL-free strings whose first element is nonempty, and the whole
  request fits spec 001's frame limit. Anything else is `-32602`.
- `subject`, in any form, is code 110 `{verb: "cell.exec"}`. Nothing reaches the guest.
- A cell that is not `ready` is code 105 `{from: <cell state>, to: "ready"}`.
- An unknown or retired cell is code 101 `cell <id>`.
- Success is `{exec_id, state: "running"}`, as spec 001 shows. The reply means the process
  exists. It does not mean the program was found; see section 3.
- A refusal from the guest is code 105 `{from: "guest_request", to: "refused", detail}`, the shape
  spec 001 §4.2 already uses on the private channels. `detail.kind` is `invalid_argv`,
  `exec_limit` or `spawn_failed`.

`exec.status` takes `kernel`, `cell` and `exec_id` and returns one of:

```json
{"exec_id": "e-11", "state": "running"}
{"exec_id": "e-11", "state": "exited", "exit_code": 0, "duration_ms": 1823}
{"exec_id": "e-11", "state": "signaled", "signal": 9, "duration_ms": 40}
```

`exit_code` is 0 to 255. `signal` is the Linux signal number. `duration_ms` runs from the start
of the process to its reap, on the guest's monotonic clock. An `exec_id` the cell never issued,
or no longer retains, is code 101 `exec <id>`. A cell that is not `ready` is 105 as above.

`cell.exec` and `exec.status` write no journal record and take no part in spec 008's
transactions. They do not wait for a grant transaction, and none waits for them.

### 2. The path from the controller to PID1

The controller sends two new membrane methods on `membrane.uds`:

| Method | Params | Result |
| --- | --- | --- |
| `membrane.cell.exec` | `{cell, argv, deadline_ms}` | `{cell, exec_id, state: "running"}` |
| `membrane.exec.status` | `{cell, exec_id, deadline_ms}` | `cell` plus the fields of one `exec.status` result |

The membrane refuses another cell with 101, as spec 001 §4.1 says. It refuses with 105 when its
cell is not `ready`. Otherwise it relays the request to PID1 on the normal lane of the original
4090 association, and checks that the reply's `boot` equals the original boot token. A different
boot is an observation fault, `-32603`, never a status.

Two verbs are added to the 4090 table, both admitted on the normal lane only:

| Method | Params | Result |
| --- | --- | --- |
| `exec` | `{argv: [String], deadline_ms}` | `{boot, exec_id: String, started: true}` |
| `exec_status` | `{exec_id: String, deadline_ms}` | `ExecStatus` |

`ExecStatus` is exactly one of `{boot, exec_id, state: "running"}`,
`{boot, exec_id, state: "exited", exit_code: u8, duration_ms: u64}` and
`{boot, exec_id, state: "signaled", signal: u8, duration_ms: u64}`. Every field of the chosen
variant is required and unknown fields refuse, like every other 4090 record.

The withdrawal lane refuses both. They queue behind an outstanding normal-lane request like any
other normal verb, within their deadline. Because they never enter the withdrawal lane, they cannot
delay a Force.

### 3. What PID1 does for `exec`

PID1 checks `argv` again and refuses a bad one with `invalid_argv`. It refuses with `exec_limit`
when 256 processes it started are still running. Then it starts the process:

1. a new session, in the workload mount and network namespaces of section 4;
2. root changed to `/workload`, working directory `/`;
3. uid 1000 and gid 1000, no supplementary groups, no capabilities in any set, no-new-privs;
4. descriptors 0, 1 and 2 on `/dev/null`, and no other descriptor;
5. the environment exactly `PATH=/usr/local/bin:/usr/bin:/bin`;
6. `argv[0]` searched on that `PATH` unless it contains a `/`.

If no file is found for `argv[0]`, the process exits with 127. If a file is found but cannot be
executed, it exits with 126. Those are the codes a shell gives, so a caller sees what it would see
anywhere. A failure to create the process at all is `spawn_failed` with the errno in `detail`,
and no ID is issued.

PID1 issues `exec_id` as `e-<n>`, where `n` counts up from 1 for this boot. A cell is never
relaunched, so an ID is never reused for that cell. PID1 retains every running entry and the 1,024
most recently finished entries; an older finished entry is dropped. PID1 reaps every child,
including descendants that were handed to it, and only the started process decides its entry's
state. Descendants may outlive the started process; they end with the cell.

### 4. The workload's namespaces

Before its hello, PID1 creates one workload mount namespace and one workload network namespace.
Every started process joins both. Neither is recreated while the cell lives.

- The mount namespace is rooted at `/workload`. It gives the ordinary interfaces a program
  expects, such as `/proc` and `/dev/null` and the random devices, and nothing that reaches PID1's
  own root, its policy file or the control and data channels.
- Managed mounts reach it by propagation: PID1 installs a Mount grant on its side, and the workload
  side, a slave of it, sees the mount. A process started before an attach sees the new mount
  without restarting. A detach removes it for everyone. Nothing the workload does propagates back.
- The network namespace holds only loopback until a ProxyMap is installed. The resolver at
  127.0.0.53 and the synthetic-prefix device of spec 001 §4.2 live in it, so a running process
  can reach a newly attached host without restarting.

### 5. What a started process cannot do

A started process cannot open `AF_VSOCK`, create a user namespace, mount, `setns` or `unshare`,
load BPF, or trace or signal PID1. Each attempt returns an ordinary error, such as `EPERM`,
`EACCES` or `EAFNOSUPPORT`, and the process keeps running. A policy that kills the process, for example with
`SIGSYS`, does not meet this. Spec 017's managed-file policy also applies: no file-backed mapping
or direct execution of a managed file. Which mechanism enforces all of this is owner decision
O-7; the denials are required whatever it is.

### 6. Shutdown

On the 4090 `shutdown`, PID1 sends SIGTERM to the process group of every running started process.
When none remains, or when the request's deadline is reached, it sends SIGKILL to every remaining
process other than itself, reaps them, syncs the root disk and powers off. A process that ignores
SIGTERM cannot delay the end of the cell past that deadline.

### 7. What a started process is not

It is not one of spec 017's five classes. It has no `GrantId`, no journal record and no row in
`GuestObservation`, and residue snapshots do not list it. Removing a plasmid does not stop it; a
detach only takes away what that plasmid made visible. Spec 008's recovery does not reconstruct
it, and the controller keeps no record of it: after a controller restart, `exec.status` answers
from PID1 alone.

## Changes proposed to accepted specs

These are proposed text, not edits made in this PR.

**Spec 001 §3.8.** After the bullet about subject attestation and code `110`, add:

> - `argv` is a nonempty array of NUL-free strings with a nonempty first element. The process runs
>   in the guest as uid1000 with no capabilities, the environment exactly
>   `PATH=/usr/local/bin:/usr/bin:/bin`, working directory `/` of the workload root and `/dev/null`
>   on descriptors0–2 (spec024). There is no environment or directory parameter.
> - Until host-side attestation exists, every `subject` is110 and nothing reaches the guest.
> - A cell that is not ready is105 `{from:<cell state>, to:"ready"}`. A guest refusal is105
>   `{from:"guest_request", to:"refused", detail}`. An unknown or no longer retained `exec_id`
>   is101.
> - `exec.status` states are `running | exited | signaled`; `exited` carries `exit_code` and
>   `duration_ms`, `signaled` carries `signal` and `duration_ms`. A program that is not found
>   exits127, and one that cannot be executed exits126.
> - A started process is not a capability. It is not journaled, observed as a grant or stopped by
>   plasmid removal. It survives a controller restart and ends with the cell.

**Spec 001 §4.** After the `membrane.cell.kill` bullet, add:

> - `membrane.cell.exec` `{cell, argv, deadline_ms}` and `membrane.exec.status`
>   `{cell, exec_id, deadline_ms}` relay §3.8 to the4090 `exec` and `exec_status` verbs on the
>   original association's normal lane, check the reply's boot, and refuse with105 when the cell
>   is not ready (spec024).

**Spec 001 §4.2, PID1.** Replace "and only then starts the unprivileged workload chrooted into
`/workload` in its controlled namespaces." with:

> and only then creates the workload's controlled mount and network namespaces. It starts no
> workload at boot. Every workload process is started by a4090 `exec` (spec024), chrooted into
> `/workload` in those namespaces.

**Spec 001 §4.2, readiness.** Replace "Readiness requires both hellos and the data association
before workload launch." with:

> Readiness requires both hellos, the data association and the workload namespaces. No workload
> process exists before readiness.

**Spec 001 §4.2, lanes.** Replace "No install, activate, drain or shutdown may enter the
withdrawal lane." with:

> No install, activate, drain, shutdown, exec or exec_status may enter the withdrawal lane.

**Spec 001 §4.2, the 4090 table.** Add:

> | `exec` | `{argv:[String], deadline_ms}` | `{boot, exec_id:String, started:true}` |
> | `exec_status` | `{exec_id:String, deadline_ms}` | `ExecStatus` (spec024) |

**Spec 001 §4.2, workload confinement.** After "or authority to open AF_VSOCK, mount/setns, load
BPF or ptrace the shim.", add:

> Each such denial returns an ordinary error to the caller and never kills the process.

**Spec 001 §4.2, shutdown.** After "Shutdown requests orderly guest exit;", add:

> PID1 sends SIGTERM to every started process group, sends SIGKILL to every remaining workload
> process at the request's deadline, reaps them, syncs the root disk and powers off (spec024).

## Open questions

These need the owner. This spec does not decide them.

1. **O-7, the guest policy mechanism.** The denials in section 5 and spec 017's managed-file
   policy need a mechanism, and hello needs it loaded. `exec` cannot run on a repo-built cell
   until this is settled.
2. **Output.** A started process writes to `/dev/null`. An agent can read results only through
   a mounted workspace or the exit code. Should `exec.status` carry a bounded capture of output,
   or should spec 001's reserved streaming come first?
3. **An initial workload.** Should `cell.new` be able to start one process at boot, such as a
   harness, or is a `cell.exec` right after `cell.new` enough?
4. **Signalling one process.** There is no `exec.signal`. A runaway process ends only with the
   cell. Should one exist?
5. **The environment.** Intent 011 says a harness should not need to change, and many expect
   `HOME`, `TERM` or `LANG`. Should the fixed environment grow, or should the image's publisher
   supply it?
6. **Subject attestation.** Every `subject` is 110 today. What would make a subject spawn
   admissible?

## Acceptance

Each item names the broken implementation it catches. Started processes report through their exit
code, because there is no output channel.

1. `["true"]` ends `exited` 0, `["false"]` ends `exited` 1, and `["sh", "-c", "kill -TERM $$"]`
   ends `signaled` 15. Catches: folding signals into exit codes, or the reverse.
2. `["no-such-program"]` replies `running` and then ends `exited` 127. A file without execute
   permission ends `exited` 126. Catches: refusing at `cell.exec`, or an entry that stays
   `running`.
3. A probe exits 0 only when its uid and gid are 1000, it has no supplementary groups, its
   effective, permitted and bounding capability sets are empty, no-new-privs is set, its
   environment is exactly the fixed `PATH`, its directory is `/`, and descriptors 0 to 2 are
   `/dev/null` with nothing else open. Catches: leaking PID1's environment, a descriptor or a
   capability.
4. A probe calls `socket(AF_VSOCK)`, `unshare(CLONE_NEWUSER)`, `mount`, `setns`, `bpf`,
   `ptrace(PTRACE_ATTACH, 1)` and `kill(1, SIGTERM)`, and exits 0 only if each returned an
   error. A policy mutant that uses `SIGSYS` makes the probe end `signaled`, and the test fails.
   Catches: a missing denial, and a denial the workload can notice by being killed.
5. A `subject` is 110, and the next `cell.exec` gets the next ID in sequence, so the guest saw
   nothing. Catches: forwarding a subject spawn to the guest.
6. `cell.exec` on a cell that is `germinating` (a stalled test helper) and on a `dead` cell is
   105 with that state. Catches: queuing an exec until the cell is ready.
7. Retention: after 1,025 finished execs, the first ID is 101 and the last 1,024 answer. The
   257th concurrent running exec is 105 `exec_limit`, and the 256 others keep running. Catches:
   unbounded tables and reused IDs.
8. Start a long-running process, SIGKILL the controller, restart it. `exec.status` still says
   `running`, and the process is alive. Catches: exec state kept in the controller, and a
   controller exit that ends started processes.
9. A probe started before an attach waits for a file in the workspace target and exits 0 when it
   appears. Attach the Mount: the probe ends `exited` 0 without a restart. After a detach, a new
   probe cannot see the file. Catches: a per-process mount snapshot, and a mount installed only
   in PID1's own namespace.
10. Send `exec` on the withdrawal lane: it refuses. Stall the normal lane with an outstanding
    request: a Force removal still completes. Catches: an exec admitted on the withdrawal lane.
11. A process that ignores SIGTERM: `cell.kill` still completes within its deadline, and the
    guest powers off. Catches: shutdown that waits on the workload.
12. In a fresh ready cell, a probe counts uid-1000 processes and exits 0 only if it is the only
    one. Catches: a workload started at boot.
13. After several execs, the journal is unchanged, `membrane.residue.snapshot` and
    `GuestObservation` list none of them, and removing every plasmid leaves them running. Catches:
    treating a started process as a grant.

## Out of scope

- Output streaming, which stays reserved in spec 001.
- An initial workload at boot.
- Signalling or stopping one started process.
- Subject spawns and attestation.
- How the cell and its supervisor start and end. That is spec 023.
