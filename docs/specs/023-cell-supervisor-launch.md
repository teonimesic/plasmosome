---
id: 023
title: Starting, owning and ending a cell's supervisor
status: draft
intents: [003, 009]
---

## Behavior

`cell.new` must produce a running cell, and nothing in the accepted specs says how. Spec 001 says
each cell has its own supervisor, `membraned`, which owns the virtual machine and survives
controller death. Spec 008 says a restarted controller reconnects to those supervisors. Neither
says who starts a supervisor, with what input, or what happens to a cell directory whose
supervisor is gone. This spec fills that gap. The controller allocates a cell ID, creates the
cell directory and its `supervisor.lock`, takes that lock, and hands the locked descriptor to a
`membraned` it starts in its own session, with a configuration record on a pipe and an empty
environment. `cell.new` answers once the guest is ready and the genome is attached. `cell.kill`
stops the workload, withdraws everything the cell holds, has `membraned` shut the guest down
within a stated deadline, waits for the lock to free, and moves the directory to
`<root>/retired/<cell>`.

Two files make a missing supervisor safe to reason about. `supervisor.lock` is held without a
gap from the moment the controller creates it until `membraned` exits, so a free lock proves
that no supervisor holds the directory. The `launched` marker is created before `membraned`
creates any runtime resource, records the host's boot session, and is removed only after a
clean teardown. A restarted or serving controller combines the two, one cell at a time. A free
lock with no marker and nothing held is a cell that never ran or ended cleanly: it is retired.
A free lock with a marker from an earlier boot is a cell whose host restarted, so its guest is
gone: it is retired and its recorded holdings are reported as residue. A free lock with a marker
from this boot may be a guest still running without its supervisor, and a held lock that never
answers is a hung supervisor. Each of those makes that one cell unaccounted: it is set aside and
reported, readiness is false, and the rest of the instance keeps serving. The existence of
`membrane.uds` decides nothing: every exit route `membraned` runs removes it, and a SIGKILL
leaves it.

This serves intent 003, cells come back after a crash: a cell must survive the controller, and a
restarted controller must tell a live cell, an ended one and an unaccounted one apart without a
PID file, and without one bad cell taking the instance down. It serves intent 009, a command
line an agent can drive: `cell.new` returns a cell that is ready to use, or a named refusal that
names any cell it left behind, never a cell that may or may not be booting. The controller never
starts a second supervisor for an existing cell directory. Recovery reconnects; it never
relaunches. Relaunching a cell after a host or supervisor crash stays an owner question.

**Platform.** The product is macOS first: Darwin arm64 is the only runtime host, and the cell is
a Linux guest run by libkrun. Linux/KVM host support is deferred, not dropped. On any other host
`cell.new` refuses before allocating anything (section 4). A real guest reaches ready only after
owner gates O-1 and O-6 are answered and O-7 is settled, because the guest policy must be loaded
before its hello. Section 9 and acceptance item 16 also wait on O-2, the host policy. Until
then, the lock, marker, launch and retirement rules are proved with a test helper at the Darwin
OS mechanism level. Evidence for each acceptance item says which level it reached: the model,
the Darwin OS mechanism, the Darwin pinned runtime, or a Linux host, which is deferred. Genomes
need spec 022 accepted first.

## Contract

### 1. Deployment input

The controller's strict JSON configuration gains an optional key, `cell_runtime`:

```json
"cell_runtime": {
  "membraned": {"path": "/usr/local/lib/plasmosome/membraned", "sha256": "<64 hex>"},
  "template": {"version": 1, "vcpus": 2, "memory_mib": 2048, "...": "..."},
  "start_deadline_ms": 60000,
  "stop_deadline_ms": 30000
}
```

- `membraned` is an Artifact as spec 001 §4.2 defines it.
- `template` is a RuntimeRecipe without `writable_root`, `control_path` and `data_path`. Every
  other RuntimeRecipe field is required and follows spec 001 §4.2. The controller adds the three
  per-cell paths (section 3). A template that carries any of the three, or lacks any other field,
  is invalid configuration.
- `start_deadline_ms` is a positive integer: the budget for `cell.new` to reach ready with its
  genome attached.
- `stop_deadline_ms` is optional, a positive integer, default 30,000. It is the default budget of
  `cell.kill`, and the cleanup budget after a failed start.
- With an ID of ten digits, `<root>/cells/<cell>/membrane.uds` and the two bridge socket paths
  must each fit in 103 bytes, the Darwin `AF_UNIX` limit. Otherwise `cell_runtime` is invalid
  configuration.

Invalid configuration refuses startup, as spec 001 §4.1 already says. Without `cell_runtime`,
startup, recovery, `cell.kill` and every observation still work; only `cell.new` refuses
(section 4). A running cell never needs its deployment input again: `membraned` retains the
recipe it was given.

The same immutability rule spec 001 §4.2 applies to the helper applies to `membraned`. Its file
and every directory above it must be owned by root or the controller's UID, must not be writable
by group or others, and must carry no ACL entry granting write to another principal. A file that
fails this is refused, not trusted after hashing. The controller verifies the rule and the
SHA256 before allocating anything; a failure refuses `cell.new` with 108, `path` the artifact
path, and no cell directory exists. Spec 022 keeps every Mount source away from these files.

### 2. Cell IDs

The controller allocates the ID. It is `cell-<n>`, where `n` is one more than the largest such
number found under `<root>/cells/` or `<root>/retired/`, or 1 when there is none. An ID present
in either directory is never allocated again. `n` has at most ten digits; past that, `cell.new`
refuses with 105 `{from: "ids_exhausted", to: "germinating"}`.

Allocation is serialized inside the controller by one mutex, held from the scan until the
directory exists. The directory is created with an exclusive `mkdir`. If that returns `EEXIST`,
the controller scans again and tries the next number. `controller.lock` serializes controllers,
not requests, so it does not provide this.

### 3. The cell directory

```
<root>/cells/<cell>/          created by the controller, exclusive, mode 0700, synced
  supervisor.lock             created and locked by the controller, then held by membraned
  launched                    created by membraned; holds the boot session
  created                     written by the controller once cell.new completed
  membrane.uds                membraned's private socket (spec 001 §4, §4.1)
  ledger.ndjson               the cell's journal (spec 008), written by the controller
  membraned.log               membraned's stderr
  runtime/                    created by membraned, mode 0700
    root.img                  writable_root
    control.uds               control_path, the 4090 listener
    data.uds                  data_path, the 4091 listener
<root>/retired/<cell>/        a retired cell, same layout, kept
```

- `supervisor.lock` and `controller.lock` use `flock(2)`, never `fcntl` record locks: a record
  lock does not exclude two opens in one process and is dropped by closing any descriptor on the
  file. Every lock descriptor is opened close-on-exec, except the one handed to `membraned`.
- `launched` is one line of strict JSON, `{"cell": "<id>", "boot_session": "<uuid>"}`, created
  exclusively and synced with its directory. On Darwin `boot_session` is `kern.bootsessionuuid`.
  A Linux host would use `/proc/sys/kernel/random/boot_id`.
- `created` is empty. The controller creates it exclusively and syncs it when `cell.new`
  completes (section 4, step 9).
- Spec 008's discovery reads `cells/` only. A retired directory is never queried or recovered,
  whatever its journal holds.
- The instance root must be on a local filesystem. NFS, SMB and FUSE-backed roots are out of
  scope; `flock` does not keep these semantics there.

### 4. How the controller starts a cell

1. Refuse before anything else when the host is not Darwin arm64, with 105
   `{from: "unsupported_host", to: "germinating"}`, or when `cell_runtime` is not configured, with
   105 `{from: "no_cell_runtime", to: "germinating"}`.
2. Verify the `membraned` artifact (section 1).
3. Resolve the genome, if one is named, through D2b and spec 022. Every refusal that resolution
   can find is returned here.
4. Under the allocation mutex, allocate the ID and create the directory. Create
   `supervisor.lock` exclusively, mode 0600, close-on-exec, and take `flock(LOCK_EX | LOCK_NB)`.
5. Start `membraned` (below). Then close the controller's copies of the lock descriptor and the
   pipe's read end, and write the configuration record to the pipe and close it.
6. Wait for `membrane.status` on `membrane.uds` to answer ready, within the remaining start
   budget. While waiting, the controller also tries the lock on a fresh descriptor. If it gets
   it, `membraned` has exited: the controller stops waiting at once (section 6).
7. With a genome, attach its members in one transaction under spec 008.
8. Mark the cell ready in the controller (section 6).
9. Create `created`, then reply `{cell, state: "ready", plasmids}`.

**The start.** `membraned` is executed directly, never through a shell. Its argv is exactly
`[membraned.path, "--config-fd=3", "--lock-fd=4", "--cell=<id>"]`; the cell ID is there so an
operator can find a hung supervisor with `ps`. Its environment is an explicit empty array.
Descriptors 0 and 1 are `/dev/null`, 2 is `membraned.log` opened for append, 3 is the read end of
the config pipe, and 4 is the locked `supervisor.lock`. Every other descriptor is closed in the
new process before exec: either by `posix_spawn` with `POSIX_SPAWN_CLOEXEC_DEFAULT`, or by an
explicit close of every other descriptor using only async-signal-safe calls (spec 018). A
close-on-exec flag set after the descriptor was made is not enough, because Darwin cannot make a
pipe or an accepted socket close-on-exec atomically, and a thread may fork in between. The
controller's instance lock, control socket, journal handles and other cells' pipes never reach
`membraned`. Every other process the controller starts follows the same rule.

`membraned` runs in its own session, as the leader of its own process group, and is not the
controller's child. The controller forks a short-lived intermediate process. The intermediate
starts `membraned` with a new session (`POSIX_SPAWN_SETSID`, or `setsid` before exec) and exits;
the controller reaps only the intermediate, so the operating system's init process (launchd on
macOS) is `membraned`'s parent. The controller keeps no PID and never signals `membraned`; every
later exchange goes through `membrane.uds`.

**The config record** is one line of strict JSON, at most 1,048,576 bytes, ending in LF and then
EOF. It has exactly these keys: `cell`, `instance_root`, `runtime_recipe` (the template plus the
three per-cell paths) and `boot_deadline_ms` (the remaining start budget at spawn). It carries no
`control_socket` and no `brokers`. The controller writes it after the spawn, so a record larger
than the pipe's buffer cannot block the spawn. No config file is written to disk. A
cell-configured `membraned` serves every method of spec 001 §4, `membrane.status` included, on
`membrane.uds`, and binds no second socket.

### 5. What membraned does, in order

`membraned` first reads descriptor 3 to EOF, parses the record strictly and closes the
descriptor. A record that is malformed, larger than the bound, missing its LF, or followed by
anything, makes it exit nonzero before step 1. It has then created nothing, and its exit frees
the lock.

1. Take `flock(4, LOCK_EX | LOCK_NB)`. With the handed-over descriptor this succeeds at once,
   because that open file already holds the lock; if it fails, exit. Set close-on-exec on
   descriptor 4, so the helper and brokers never inherit it.
2. Open `<instance_root>/cells/<cell>` again by name, through spec 008's no-follow opens, and
   open `supervisor.lock` inside it. Refuse and exit unless that file is the same file (device and
   inode) as descriptor 4. This stops a `membraned` from running in a directory that has been
   retired, or with a lock that belongs to another directory.
3. Bind `membrane.uds` under spec 001 §4.1's private socket rules.
4. Create `launched` (section 3), and sync it and the directory.
5. Create `runtime/`, copy the root image, bind the two bridge listeners and launch the helper,
   as spec 001 §4.2 says.
6. `boot_deadline_ms` is `membraned`'s own budget, counted from its exec to the guest's hello. If
   it passes first, `membraned` tears down as a launch failure (below) and exits, whether or not a
   controller is still waiting.
7. Report the cell `germinating` until the controller's `membrane.cell.desired` for the genome is
   applied, then `ready` (section 6).
8. Serve until `membrane.cell.kill`, SIGTERM or SIGINT. Then tear down: stop the guest (section
   7), reap the helper, stop and reap every broker, and remove both bridge sockets and
   `root.img`. Remove `launched` only after a clean teardown, which means all of those finished.
9. Remove its own `membrane.uds` by identity, then exit. Exit releases the lock. The exception
   is a `membrane.cell.kill` whose teardown was not clean: then `membraned` keeps serving, with its
   lock and marker, so the cell stays visible and a later kill can finish (section 7).

A launch failure follows the same order: the attempt releases what it created, `launched` is
removed only if that release was clean, and `membraned` exits nonzero. A SIGKILL runs none of
steps 8 and 9: the lock frees, `launched` stays, and the helper and guest may keep running. That
is owner decision O-10. This spec makes such a cell visible and does not clean it.

### 6. Readiness

**From the controller.** A cell whose `cell.new` has not replied is `germinating`, whatever its
supervisor says. `cell.list` and `cell.status` show it with that state. `cell.exec` and every
plasmid verb on it refuse with 105 `{from: "germinating", to: "ready"}`. Requests for other cells
are not blocked by a `cell.new` in progress. Mutations of one cell (`cell.new`, plasmid verbs,
`cell.kill` and the check of section 8) run one at a time, in arrival order. Reads never wait for
them; `cell.exec` waits for them only as spec 024 says.

**From the supervisor.** `membrane.status` covers the cell. A cell-configured membrane answers
`{ready: true, state: "serving"}` exactly when its cell is `ready` and every broker it supervises
answers ready. When its cell is not ready, it answers `ready: false`, `state: "cell_not_ready"`
and `cell_state` with the lifecycle state (`germinating`, `draining` or `dead`). When only a
broker is not ready, it answers as spec 001 §4 already says. `cell.status` relays this answer in
its `supervisor` field as before.

`dead` is observable only while `membraned` still runs after its guest stopped: a guest that
crashed, or a teardown that was not clean (section 7). A cleanly ended cell is retired, and is
101.

**A failed start.** If the start budget runs out, or the genome attach fails, the controller
ends the cell as `cell.kill` does (section 7), with a fresh budget of `stop_deadline_ms` and no
workload to stop. If `membraned` has already exited (step 6 of section 4 found the lock free),
the controller instead applies the supervisor check (section 8) at once. Then it replies:

- a start timeout or an early exit: 105 `{from: <last observed state>, to: "ready", cell}`, where
  the last observed state is `germinating` if the supervisor never answered;
- a failed attach: the attach's own refusal.

The cell is retired when the cleanup finished. When it did not, the cell stays in `cells/`,
`cell.list` shows it, and the 105 names it in `cell` so the caller can retry `cell.kill`. The
reason for an early exit is in `membraned.log`.

### 7. Ending a cell

`cell.kill` gains an optional `deadline_ms`, a positive integer. Its default is
`stop_deadline_ms`, or 30,000 when `cell_runtime` is not configured. Call it D. Every step below
is bounded by D, counted from the request's arrival:

1. **Stop the workload** with spec 024's `membrane.workload.stop`, `deadline_ms` D/4. With `now`,
   the deadline is 0, so every workload process is killed at once.
2. **Withdraw** every attachment, if there is any, in one removal transaction under spec 008:
   safe removal, or Force when `now` is set, with the operator assertion spec 001 §3.7 records.
   The drain deadline is what remains of D/2. A drain that times out replies 106
   `{handle, deadline_ms}` and the cell stays.
3. **Stop the guest** with `membrane.cell.kill` and `DrainSpec {deadline, policy}`: policy
   `Safe`, or `Force` with `now`; deadline R minus a reserve, where R is what remains of D and
   the reserve is the smaller of 5,000 ms and R/2. `membraned` sends the 4090 `shutdown` with
   `deadline_ms` equal to that deadline minus the smaller of 5,000 ms and half of it; with `Force`
   it sends none. When the shutdown deadline passes and the helper still runs, it kills the
   helper's process group and reaps the helper. Then it tears down (section 5, step 8). Its result
   is `{state, clean, residue, residue_items?}`, with the residue observed by `membraned` itself
   after the teardown, as spec 001 §3.4 requires of residue.
4. **Wait for the lock** to free, within what remains of D, by trying `flock` on a fresh
   descriptor, and keep it.
5. **Retire.** Holding the lock, check that `launched` is absent and the journal has no settled
   attachment and no pending transaction. Rename `cells/<cell>` to `retired/<cell>` and sync both
   parent directories. Release the lock.
6. **Reply** `{cell, state: "dead", drained, residue}` as spec 001 §3.7 shows, with the residue
   from step 3.

Outcomes that do not retire the cell:

- `membraned` stopped the guest but the teardown was not clean (for example, a bridge socket it
  could not remove): it answers `{state: "dead", clean: false, residue: "items", ...}` and keeps
  running with its lock and marker. `cell.kill` replies with the same `residue` and the cell stays
  in `cells/` as `dead`. A later `cell.kill` repeats the steps; `membraned` retries what did not
  finish.
- The helper is not reaped by the deadline: 105 `{from: "draining", to: "dead", cell}`.
- The lock does not free within D: 105 `{from: "dead", to: "retired", cell}`.

A cell that is already retired, or never existed, is 101 `cell <id>`.

### 8. The supervisor check

Spec 008 tells startup to query each cell's `membrane.uds`, and forbids deriving state from a PID
file or socket existence. The lock and the marker are neither: a held lock proves a live holder,
an absent marker proves no runtime was launched or it was cleanly torn down, and a marker's boot
session says whether it was written since this host booted. The controller runs this check:

- at startup, for every validated cell directory, through spec 008's no-follow opens;
- while serving, whenever a query to a cell's membrane fails, except for a cell whose `cell.new`
  is still in progress.

It never touches an entry spec 008 does not validate. To test the lock, the controller tries
`flock(LOCK_EX | LOCK_NB)` on a fresh descriptor. "Free" means it got the lock; it then holds it
until the row's action is done.

| Lock | Marker | Other | Outcome |
| --- | --- | --- | --- |
| held | any | answers within `recovery_deadline_ms` | Query under spec 008 as before. A cell that answers `germinating` is waited for within the same budget. |
| held | any | no answer, or still `germinating`, when the budget ends | Unaccounted: `unresponsive` or `germinating`. |
| free | from an earlier boot | any journal | Retire. Every settled holding and pending operation in the journal is written to the session log as residue of a host restart. |
| free | from this boot, or unreadable | any journal | Unaccounted: `supervisor_lost` (O-10). |
| free | absent | valid journal, nothing settled, nothing pending | Retire, with a session log line. |
| free | absent | valid journal with a settled attachment or a pending operation | Unaccounted: `holdings_without_supervisor`. |
| free | any | corrupt journal | Quarantine under spec 008, with the lock state and marker in its report. |

A missing journal reads as empty, as spec 008 says.

**Unaccounted cells** are handled as spec 008 handles quarantine. The cell is absent from the
ordinary registry, so `cell.list` omits it and every verb naming it is 101. `plasmosome.recovery`
lists it with its reason, its lock state and its marker. It is never mutated, and readiness is
false while any cell is unaccounted. It is never a reason to refuse startup or to stop serving
other cells. A later startup checks it again. Nothing else clears it; what should is an owner
question.

**Order.** At startup the controller classifies every validated cell first, holding each free
lock. It acts only after that, and only if startup is going to serve. A startup that refuses for
another reason (spec 008's partial discovery, unfinished cleanup or publication) retires nothing
and releases the locks.

**After a successful startup**, an adopted cell with no `created` file is one whose `cell.new`
never replied, so no client knows it exists. The controller ends it as `cell.kill` does, with a
budget of `stop_deadline_ms`, and writes a session log line naming it. If that kill does not
finish, the cell stays listed with its state.

**While serving**, the outcome applies to that one cell, and the request that found it replies
from the result: 101 when the cell was retired or set aside.

### 9. Isolation of the private socket

Spec 001 §4.1 requires that a cell workload cannot reach `membrane.uds`. Here the workload runs in
a hardware guest with no virtiofs export and no network device. The helper that runs it is
confined by its host policy (spec 001 §4.2). For this spec to hold, that policy must deny the
helper every Unix socket except this cell's two bridge sockets, and every file write except this
cell's `runtime/root.img`. In particular it must deny this cell's `membrane.uds`, every other
cell's directory, and the controller's control socket.

That depends on O-2. The qualified bundle's policy today allows outbound connections to any Unix
socket, and writes anywhere under `/usr/local/var/plasmosome/work`. One policy file pinned by one
hash cannot name one cell's paths unless the policy takes per-cell parameters, which spec 001
§4.2 does not define. Binding the policy to one cell is part of O-2. Until it is answered, this
section and acceptance item 16 are gated, and the instance root must sit under the policy's
writable directory for the qualified bundle to run at all.

## Changes proposed to accepted specs

These are proposed text, not edits made in this PR. `docs/specs/README.md` does not say that the
accepting PR applies them, and spec 001 says its text changes in a pull request with the
reasoning written down. So the PR that accepts this spec must carry these edits, or this spec
stays draft. Spec 024 changes the same 105 row of spec 001 §1, so the row below is the merged
text both specs propose.

**Spec 001 §1, the 105 row.** Replace the structured fields with:

> `from`, `to`; `detail` on a guest refusal, exactly one of spec 024's detail records; `cell`
> when a `cell.new` or `cell.kill` refusal leaves a cell in place (spec 023); private recovery
> methods additionally carry the typed `recovery` refusal in §4.1

**Spec 001 §3.3 (`001:243-247`).** After "Quarantine alone can coexist with a serving controller
when all live observations are complete; readiness remains false.", add:

> A cell that spec 023's supervisor check leaves unaccounted is handled the same way: it is not an
> incomplete observation, it coexists with a serving controller, and readiness remains false.

**Spec 001 §3.4.** After "then stops the controller.", add:

> Stop does not end cells: each cell's supervisor and guest keep running, and the next controller
> adopts them. Only `cell.kill` ends a cell (spec 023).

**Spec 001 §3.5.** Change the example result's `"state": "germinating"` to `"state": "ready"`,
and add this bullet after the `genome` bullet:

> - The controller allocates the cell ID and starts the cell's supervisor as spec 023 says. It
>   replies only when the cell is ready and the genome is attached; until then the cell is listed
>   as `germinating` and refuses `cell.exec`. Refusals that resolving the genome can find are
>   returned before any cell directory exists. A cell that misses the start deadline, whose
>   supervisor exits early, or whose genome attach fails, is ended and retired before the reply,
>   which is 105 `{from, to: "ready", cell}` or the attach's own refusal. On a host other than
>   Darwin arm64, or without configured `cell_runtime`, cell.new is 105
>   `{from: "unsupported_host" | "no_cell_runtime", to: "germinating"}`.

**Spec 001 §3.7.** After the paragraph beginning "With `"now": true`", add:

> Optional `deadline_ms` bounds the whole kill; its default is `cell_runtime.stop_deadline_ms`, or
> 30,000. The controller stops the workload, withdraws every attachment in one removal
> transaction, then sends `membrane.cell.kill`, waits for the supervisor to exit, and moves the
> cell directory to `<instance>/retired/<cell>` (spec 023) before replying. A drain timeout is 106.
> A teardown that was not clean replies `"state": "dead"` with its residue and leaves the cell
> listed as `dead` for a retry. A guest not stopped, or a supervisor not gone, within the deadline
> is 105 naming the cell. A retired cell is 101.

**Spec 001 §4, `membrane.status`.** Replace "Every call asks every broker again; no answer is
kept." with:

> Every call asks every broker again, and a membrane configured for a cell also reports its cell;
> no answer is kept. Such a membrane is ready exactly when its cell is `ready` and every broker
> answers ready.

Replace the `empty` row with these two rows:

> | `cell_not_ready` | `cell_state` — the cell's lifecycle state: `germinating`, `draining` or `dead` |
> | `empty` | none — a membrane with neither a cell nor brokers, which is never ready |

**Spec 001 §4, `membrane.cell.kill`.** Append:

> The DrainSpec policy is `Safe`, or `Force` for `cell.kill --now`. The membrane sends the guest
> `shutdown` within the deadline, kills and reaps the helper when it passes, and answers
> `{state, clean, residue}` with residue it observed itself after teardown. After a clean
> teardown it removes its launch marker, removes its own socket and exits. Otherwise it keeps
> running with its lock and marker. A membrane configured for a cell serves every method of this
> section on `membrane.uds` and binds no second socket.

**Spec 001 §4.1.** After the paragraph introducing `registry_root`, add:

> The same configuration admits optional `cell_runtime: {membraned: Artifact, template,
> start_deadline_ms, stop_deadline_ms?}`, spec 023's deployment input for new cells. Recovery
> never reads it.

**Spec 001 §4.2.** After "Recovery reconnects, not relaunches.", add:

> The controller starts each cell's supervisor once, in its own session with init as its parent,
> with argv `[membraned.path,"--config-fd=3","--lock-fd=4","--cell=<id>"]`, an explicit empty
> environment, and only descriptors 0 to 4; descriptor 4 is the cell's `supervisor.lock`, already
> locked with `flock` by the controller. The supervisor holds that lock for its whole life, binds
> its socket, and creates the `launched` marker, which records the host boot session, before any
> runtime resource. It removes the marker only after a clean teardown (spec 023).

**Spec 008, writer lock (`008:66-67`).** Replace "takes a nonblocking exclusive OS file lock" with:

> takes a nonblocking exclusive `flock(2)` lock on a close-on-exec descriptor

**Spec 008, startup query (`008:477-483`).** After "the controller must not derive that state
from a PID file, socket existence or inability to connect.", add:

> Before that query, startup applies spec 023's supervisor check to each validated cell, and
> queries only cells whose `supervisor.lock` is held. The lock and the `launched` marker are the
> supervisor's own lifecycle evidence, not a PID file or socket existence. A missing, failed,
> timed-out or malformed response, or a free lock with a marker from this boot, makes that one
> cell unaccounted: it is handled as quarantine is, and never refuses startup. A free lock with no
> marker and nothing held, or with a marker from an earlier boot, retires the directory to
> `<root>/retired/<cell>`. Retired directories are not discovered. The same check runs while
> serving, whenever a query to a cell's supervisor fails.

**Spec 008, refusing startup (`008:537-540`).** After "exits nonzero rather than serving a partial
instance.", add:

> An unaccounted cell under spec 023 is not incomplete observation for this rule.

**Spec 008, unlinking (`008:543-546`).** After "as the existing daemon does.", add:

> Retiring a cell under spec 023 renames its whole directory, journal and any stale
> `membrane.uds` included, to `<root>/retired/<cell>`; nothing is unlinked. The rule for the
> instance's control socket is unchanged.

**Spec 008, R8.** Replace "Unknown observation prevents startup success." with:

> Unknown observation keeps readiness false and leaves that cell out of the ordinary registry; it
> does not prevent startup (spec 023).

**Spec 008, R9.** Replace "An unavailable supervisor, partial discovery, unfinished cleanup or
unresolved publication prevents serving" with:

> Partial discovery, unfinished cleanup or unresolved publication prevents serving; an unavailable
> supervisor makes only its cell unaccounted under spec 023, with readiness false,

**Spec 003, the seam table.** Add two rows:

> | Process spawning (the cell supervisor) | a launcher passed to the controller, with spec 023's spawn contract | a launcher that records argv, environment and descriptors, plus the fixture test helper |
> | Cell supervisor lock and marker | directory path injected as argument | a test-owned cell directory in a `TempDir` |

## Open questions

These need the owner. Where this spec needs an answer to work, it sets a conservative default.

1. **O-10, an orphaned guest.** A SIGKILLed `membraned` leaves its helper and guest running. This
   spec makes that cell unaccounted and does not clean it. Accept the documented limit, or amend
   spec 001 §4.2 to give the helper a lifeline? (Handing the helper an inherited copy of
   `supervisor.lock` would be one: a free lock would then also prove the helper gone.)
2. **Relaunch after a crash.** After a host restart or a lost supervisor, the cell is retired or
   set aside, never relaunched (the default, matching spec 008's out-of-scope line). Should a
   cell come back after a host crash, as intent 003's title suggests?
3. **Clearing an unaccounted cell.** Nothing clears one except a later startup's check (the
   default). Should there be an operator verb, for example a Force retire that records an
   operator assertion, and what may it assume about a hung supervisor or a lost guest?
4. **Retired directories and ID reuse.** They are kept, so IDs are never reused (the default).
   Pruning them would let IDs repeat unless a high-water record is kept. Who prunes, and when?
5. **Does `plasmosome.stop` end cells?** The default is no: cells outlive a stopped controller.
6. **Double fork or a launchd job.** Double fork keeps the controller free of a launchd
   dependency. A launchd job adds the operating system's own record of the process, but its
   restart policy must be off. On a Linux host, a double fork does not leave a systemd unit's
   cgroup, so the unit's kill mode would end every cell.
7. **Per-request sizing.** Every cell gets the template's `vcpus` and `memory_mib` (the default).
   Should `cell.new` accept smaller values?

O-2 is not listed here as a question this spec asks, but section 9 and item 16 are gated on it.

## Acceptance

Each item names the broken implementation it catches. Unless an item says otherwise, it runs at
the Darwin OS mechanism level with the fixture test helper and a controller whose recovery path
exists, and needs no owner gate. Items that attach plasmids also need the RealCellBackend task
and an accepted spec 022; the helper must pass spec 001 §4.2's checks as a real, verified test
artifact, never a production fake.

1. A test helper that holds its hello until the test releases it. While held, `cell.new` has not
   replied, `cell.status` shows `germinating`, `cell.exec` refuses with 105 `{from:
   "germinating"}`, and `membrane.status` is `cell_not_ready` with `cell_state: "germinating"`.
   After release, `cell.new` replies `ready` and `created` exists. Catches: replying on a fork or
   a bound socket, and admitting work before the genome is attached.
2. A helper that never sends hello. At the moment `cell.new` replies 105 `{from: "germinating",
   to: "ready", cell}` (after `start_deadline_ms`, within `stop_deadline_ms` more), the directory
   is under `retired/`, has no `launched`, and no helper or `membraned` process remains. Catches:
   leaving cleanup to a later startup, and charging cleanup to the spent start budget.
3. A config that makes `membraned` exit before ready (a wrong helper digest): `cell.new` replies
   105 naming the cell well before `start_deadline_ms`, and the reason is in `membraned.log`.
   Catches: waiting out the whole start budget for a supervisor that is gone.
4. With a ready cell, kill the controller's whole process group with SIGKILL and hang up its
   session. `membraned` and the helper keep running; `membraned`'s `getsid` and `getpgid` differ
   from the controller's and its parent is PID 1. A new controller takes `controller.lock`,
   adopts the cell, and no second `membraned` starts. Catches: a shared session or process group,
   an inherited instance lock, and a recovery that relaunches.
5. Plant a descriptor without close-on-exec in the controller before `cell.new`. In the started
   `membraned`: the environment is empty; argv carries the cell ID; descriptors 0 to 2 are as
   stated; 3 is closed after reading; 4 is the cell's `supervisor.lock`; the planted descriptor
   and every controller descriptor are absent; no config file exists. Catches: a leaked
   descriptor, and configuration through the environment or a file.
6. `membraned` started by the test with descriptor 4 set to each of: another cell's locked
   `supervisor.lock`; an unrelated file; this cell's lock file opened without the lock while the
   test holds it through another open. Each exits nonzero without binding a socket, creating
   `launched` or creating `runtime/`. With this cell's lock held by the test through another open
   and the directory renamed to `retired/` before start, it also exits. Catches: trusting the
   descriptor without checking it, and launching in a retired directory.
7. A config record cut short (no LF, then EOF), one over 1,048,576 bytes, and one followed by a
   second line. Each `membraned` exits without binding a socket or creating `launched`, and the
   controller then retires the directory. Catches: acting on a partial record.
8. Two `cell.new` requests at once get two distinct IDs and two directories. While one waits on a
   held hello, `cell.status` of another cell answers at once. A `cell.kill` and a serving-time
   check of one cell, started together, run one after the other: the kill replies with its own
   result and the directory is retired once. Catches: unserialized allocation, a controller
   blocked by one boot, and two threads that both believe they hold one cell's lock.
9. Startup over these validated cells, each built by hand:
   - A: lock free, `launched` from this boot, empty journal, no `membrane.uds`;
   - B: lock free, no `launched`, a stale `membrane.uds` file, empty journal;
   - C: lock free, `launched` with another boot session, a journal with one settled attachment;
   - D: lock free, no `launched`, a journal with a settled attachment;
   - E: lock held by a test process that never answers.

   Startup serves. A, D and E are absent from `cell.list`, listed by `plasmosome.recovery` as
   `supervisor_lost`, `holdings_without_supervisor` and `unresponsive`, and readiness is false.
   B and C are under `retired/`, and the session log names C's holding as residue. A directory
   under `retired/` with a corrupt journal is ignored. Catches: deciding from socket existence,
   refusing the whole instance for one cell, and leaving a cell from before a host restart
   blocking forever.
10. The same set, plus an invalid controller configuration that makes startup refuse after
    discovery: nothing is retired, and B and C are still under `cells/`. Catches: acting before
    every cell is classified.
11. While serving: send SIGTERM to a cell's `membraned` with no attachments, then query it. The
    reply is 101 and the directory is under `retired/`. SIGKILL another cell's `membraned`, then
    query it: 101, the cell is listed by `plasmosome.recovery` as `supervisor_lost`, readiness is
    false, and a third cell still answers. Catches: serving-time outcomes that are undefined, or
    that take down the instance.
12. Stop the controller after it spawned `membraned` and before ready, with a helper that holds
    hello. `membraned` gives up at `boot_deadline_ms`, tears down and exits; a new controller
    retires the directory. Then repeat with a helper that releases hello after the controller died:
    the new controller adopts the cell, finds no `created`, ends it and logs it. Catches: a boot
    that outlives its deadline, and an unrequested cell left running.
13. `cell.kill` on a cell with two empty attachments, through a membrane double that records the
    order of requests: `membrane.workload.stop`, then the withdrawal, then `membrane.cell.kill`
    with a deadline at most D minus 5,000 ms, and a 4090 `shutdown` deadline below that. Then
    `membraned` has exited, the directory is `retired/<cell>`, `cell.status` is 101, the reply's
    residue came from `membraned`, and the next `cell.new` gets a new ID. Catches: ending the
    guest before withdrawing, a shutdown with no budget, residue from controller memory, and ID
    reuse.
14. A guest that ignores `shutdown`: with `deadline_ms` 10,000, `cell.kill` replies within 10
    seconds and the helper is gone. A bridge socket made unremovable: `cell.kill` replies
    `state: "dead"` with residue items, `membraned` still runs, and the cell is listed as `dead`.
    Make the socket removable and repeat `cell.kill`: the cell is retired. Catches: an unbounded
    kill, retiring a cell whose teardown failed, and a teardown that cannot be retried.
15. A `membraned` file under a group-writable parent (built under the canonical temp root), a
    group-writable `membraned` file, and a wrong digest: each `cell.new` replies 108 with the
    artifact path, and no cell directory exists. On a non-Darwin host (Linux host level,
    deferred) and without `cell_runtime`, `cell.new` replies the stated 105 and no directory
    exists; a controller restarted without `cell_runtime` over live cells recovers them.
    Catches: a hash-only check, allocating before a refusal, and recovery that needs deployment
    input.
16. **Gated on O-2; Darwin pinned runtime.** Under the real host policy, the helper's attempts to
    connect to its own `membrane.uds`, to another cell's `control.uds` and to the controller's
    control socket each fail with a permission error, and so does a write outside its
    `runtime/root.img`. A policy mutant that widens the Unix-socket outbound rules to the cell
    directory lets one connect, and the test fails. Catches: a host policy wider than this cell's
    `runtime/`.
17. A genome whose resolution refuses (spec 022's 103) creates no cell directory. A genome whose
    apply fails after boot leaves a retired directory and no running `membraned`. The first half
    needs a registry-imported genome (spec 020). Catches: creating a cell before resolution, and
    leaving an empty running cell after a failed attach.

## Out of scope

- What runs inside the guest, and how commands reach it. That is spec 024.
- How plasmids become capabilities. That is spec 022.
- Cleaning an orphaned guest after a SIGKILLed supervisor (O-10).
- Relaunching a cell after a host or supervisor crash.
- Instance roots on NFS, SMB or FUSE filesystems.
- `cell.clone`, `cell.save`, `cell.load` and `freeze`.
