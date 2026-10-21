---
id: 023
title: Starting, owning and ending a cell's supervisor
status: accepted
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
environment. `cell.new` answers once the guest is up and the genome is attached. `cell.kill`
stops the workload, withdraws everything the cell holds, has `membraned` shut the guest down
within a deadline, waits for the lock to free, and moves the directory to `<root>/retired/<cell>`.

Two files make a missing supervisor safe to reason about. `supervisor.lock` is held without a
gap from its creation until `membraned` exits, so a free lock proves no supervisor holds the
directory. The `launched` marker is created before any runtime resource, records the host's boot
session, records a clean stop on SIGTERM, and is removed only by a clean `cell.kill` or a failed
launch. A free lock with no marker and nothing held is a cell that never ran or was ended: it is
retired. A free lock with a marker from an earlier boot, or one recording a clean stop, is a cell
whose guest is gone: it is retired unless it owes an external obligation, and its holdings are
reported as residue. A free lock with any other marker, or a held lock that never answers, **sets
aside** that one cell: it is reported, readiness is false, and the rest of the instance keeps
serving. The existence of `membrane.uds` decides nothing.

This serves intent 003: a cell survives the controller, and a restarted controller tells a live
cell, an ended one and a lost one apart without a PID file, so an unresponsive or lost supervisor
no longer takes the instance down. It serves intent 009: `cell.new` returns a cell that is ready
to use, or a refusal naming any cell it created. The controller never starts a second supervisor
for a cell directory, and only `cell.kill`, or a launch that fails before hello, deletes a guest
disk. Relaunch is an owner question.

**Platform.** macOS first: Darwin arm64 is the only runtime host, and the cell is a libkrun Linux
guest. A Linux host is deferred, not dropped; on any other host `cell.new` refuses before
allocating anything. A real guest reaches ready only after owner gates O-1 and O-6 are answered
and O-7 is settled; §9 also waits on O-2. Until then the lock, marker, launch and retirement rules
are proved with a test helper. Genomes need spec 022 accepted first.

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
  per-cell paths (§3). A template that carries any of the three, or lacks another field, is
  invalid configuration. Every cell gets the template's sizing; `cell.new` takes none.
- `start_deadline_ms` is a positive integer: the budget for `cell.new` to reach ready with its
  genome attached.
- `stop_deadline_ms` is optional, a positive integer, default 30,000: the default budget of
  `cell.kill`, and the cleanup budget after a failed start.
- A cell ID has at most ten digits. With such an ID, `<root>/cells/<cell>/membrane.uds` and the two
  bridge socket paths must each fit in 103 bytes, the Darwin `AF_UNIX` limit; otherwise
  `cell_runtime` is invalid configuration.

Invalid configuration refuses startup, as spec 001 §4.1 says. Without `cell_runtime`, startup,
recovery, `cell.kill` and every observation still work; only `cell.new` refuses (§4). A running
cell never needs its deployment input again: `membraned` keeps the recipe it was given.

Spec 001 §4.2's immutability rule for the helper applies to `membraned` and to every Artifact in
the template. Each file and every directory above it must be owned by root or the controller's
UID, not writable by group or others, and carry no ACL entry granting write to another principal.
A file that fails this is refused, not trusted after hashing. The controller verifies the rule and
each SHA256 before allocating anything; a failure refuses `cell.new` with 108, `path` the
artifact, and no cell directory exists. Spec 022 keeps every Mount source away from these files.

### 2. Cell IDs

The controller allocates the ID. It is `cell-<n>`, where `n` is one more than the largest such
number under `<root>/cells/` or `<root>/retired/`, or 1 when there is none. An ID present in
either directory is never allocated again. Allocation is serialized inside the controller by one
mutex, held from the scan until the directory exists. The directory is created with an exclusive
`mkdir`; on `EEXIST` the controller scans again and tries the next number. `controller.lock`
serializes controllers, not requests, so it does not provide this.

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
  exclusively and synced with its directory. On Darwin `boot_session` is `kern.bootsessionuuid`,
  assumed to change on every boot; the implementing task measures that. A Linux host would use
  `/proc/sys/kernel/random/boot_id`. After a clean stop (§5, step 9) the record is replaced
  atomically by one that adds `"stopped": true`.
- `created` is empty. The controller creates it exclusively and syncs it when `cell.new`
  completes (§4, step 8).
- Spec 008's discovery reads `cells/` only. A retired directory is never queried or recovered.
- The instance root must be on a local filesystem. NFS, SMB and FUSE-backed roots are out of
  scope; `flock` does not keep these semantics there.

### 4. How the controller starts a cell

1. Refuse before anything else when the host is not Darwin arm64, with 105
   `{from: "unsupported_host", to: "germinating"}`, or when `cell_runtime` is not configured, with
   105 `{from: "no_cell_runtime", to: "germinating"}`.
2. Verify `membraned` and the template's Artifacts (§1).
3. Resolve the genome, if one is named, through D2b and spec 022. Every refusal that resolution
   can find is returned here.
4. Under the allocation mutex, allocate the ID and create the directory. Create
   `supervisor.lock` exclusively, mode 0600, close-on-exec, and take `flock(LOCK_EX | LOCK_NB)`.
5. Start `membraned` (below). Close the controller's copies of the lock descriptor and the pipe's
   read end, then write the configuration record to the pipe and close it.
6. Wait, within the remaining start budget, until `membrane.status` on `membrane.uds` shows the
   guest up: any answer but `cell_not_ready` (§6). Meanwhile, try the lock on a fresh descriptor;
   if the controller gets it, `membraned` has exited and the controller stops waiting at once.
7. With a genome, attach its members in one transaction under spec 008.
8. Mark the cell ready, create `created`, and reply `{cell, state: "ready", plasmids}`.

**The start.** `membraned` is executed directly, never through a shell. Its argv is exactly
`[membraned.path, "--config-fd=3", "--lock-fd=4", "--cell=<id>"]`; the cell ID lets an operator
find a hung supervisor with `ps`. Its environment is an explicit empty array. Descriptors 0 and 1
are `/dev/null`, 2 is `membraned.log` opened for append, 3 is the read end of the config pipe,
and 4 is the locked `supervisor.lock`. Every other descriptor is closed in the new process before
exec, by `posix_spawn` with `POSIX_SPAWN_CLOEXEC_DEFAULT` or by an async-signal-safe close loop
(spec 018). A close-on-exec flag set after creation is not enough, since Darwin cannot set it
atomically on a pipe or an accepted socket. Every process the controller starts follows this.

`membraned` runs in its own session, as the leader of its own process group, with the operating
system's init process (launchd on macOS) as its parent, not the controller. The controller keeps
no PID and never signals it; every later exchange goes through `membrane.uds`. How it gets there,
a short-lived intermediate or a launchd job with no restart policy, is the implementing task's
choice; anything run between `fork` and `exec` in the multithreaded controller is
async-signal-safe.

**The config record** is one line of strict JSON, at most 1,048,576 bytes before its LF, then
the LF and EOF. The LF does not count toward the bound, as spec 001 §1 counts a request line. It
has exactly these keys: `cell`, `instance_root`, `runtime_recipe` (the template plus the three
per-cell paths) and `boot_deadline_ms` (the remaining start budget at spawn). The controller
writes it after the spawn, so a record larger than the pipe's buffer cannot block the spawn. No
config file is written to disk. A cell-configured `membraned` serves every method of spec 001 §4
on `membrane.uds` and binds no second socket. Amended on 2026-10-20: this did not say whether the
LF counts toward the bound.

### 5. What membraned does, in order

`membraned` first reads descriptor 3 to EOF, parses the record strictly and closes the
descriptor. A record that is malformed, larger than the bound, missing its LF, or followed by
anything, makes it exit nonzero before step 1, having created nothing.

1. Take `flock(4, LOCK_EX | LOCK_NB)`. With the handed-over descriptor this succeeds at once,
   because that open file already holds the lock; if it fails, exit. Set close-on-exec on
   descriptor 4, so the helper and brokers never inherit it.
2. Open `<instance_root>/cells/<cell>` again by name, through spec 008's no-follow opens, and
   `supervisor.lock` inside it. Exit unless that is the same file (device and inode) as descriptor
   4. This stops a `membraned` from running in a retired directory, or with another cell's lock.
3. Bind `membrane.uds` under spec 001 §4.1's private socket rules.
4. Verify the runtime's Artifacts and their closure as spec 001 §4.2 requires.
5. Create `launched` (§3), and sync it and the directory.
6. Create `runtime/`, copy the root image, bind the two bridge listeners and launch the helper, as
   spec 001 §4.2 says. `boot_deadline_ms` is `membraned`'s own budget, from its exec to the
   guest's hello. If it passes first, or the helper exits before hello, `membraned` handles it
   as a launch failure (below).
7. Report the cell `germinating` until the guest's hello, then `ready` (§6). Serve.
8. On `membrane.cell.kill`, tear down: stop the guest (§7), reap the helper, stop and reap every
   broker, and remove both bridge sockets and `root.img`. Remove `launched` only after a clean
   teardown, meaning all of those finished. If the teardown was not clean, keep serving with the
   lock and marker, so the cell stays visible and a later kill can finish.
9. On SIGTERM or SIGINT, stop the guest: send it the 4090 `shutdown` with a positive deadline
   under 5 s, and if the helper still runs 5 s after the signal, kill its process group. Then
   reap the helper and every broker, and remove the bridge sockets, keeping `root.img`. If all of
   that finished, replace `launched` atomically with the record that adds `"stopped": true` and
   sync the directory; otherwise leave it. The marker then decides the cell (§8). The 5 s is
   fixed, chosen to end before a system shutdown's SIGKILL. It is revisited once task
   plasmosome-604b measures which signals a reparented `membraned` gets, and when.
   Amended on 2026-10-20: this step set no budget for the guest stop.
10. Remove its own `membrane.uds` by identity, then exit. Exit releases the lock.

A launch failure releases what the attempt created, removes `launched` only if that release was
clean, and exits nonzero. A SIGKILL runs none of steps 8 to 10: the lock frees, `launched` stays,
and the helper and guest may keep running. That is owner decision O-10; this spec makes such a
cell visible and does not clean it.

### 6. Readiness

**From the controller.** A cell whose `cell.new` has not replied is `germinating`, whatever its
supervisor says. A cell whose `cell.kill` has started is `draining` until it is retired. A kill
that gives up leaves it `draining` after a 106, and otherwise in the state its supervisor
reports. `cell.list` and `cell.status` show these states. `cell.exec` and every plasmid
verb on a `germinating` or `draining` cell refuse at once with 105 `{from: <state>, to:
"ready"}`, before any wait. Mutations of one cell (`cell.new`, `plasmid.add`, `plasmid.remove`,
`plasmid.reload`, `plasmid.renew`, `cell.kill`, a `plasmosome.stop`'s drain of the cell
(spec 001 §3.4), the check of §8 and spec 028's expiry) run one
at a time, in arrival order. Requests for other cells are not blocked. Reads never wait for
mutations, except a read whose own query failed, which waits for the check it queued.
`plasmid.list` is a read. Corrected on 2026-10-20: this listed "plasmid verbs" among the
mutations, which read as including `plasmid.list`, and left out spec 028's expiry.
Amended on 2026-10-21: this list did not name a `plasmosome.stop`'s drain of the cell.

**From the supervisor.** A cell-configured membrane's `membrane.status` answers `{ready: true,
state: "serving"}` exactly when its guest has said hello, no `membrane.workload.stop` or
`membrane.cell.kill` has arrived, and every broker answers ready. When the cell is the reason, it
answers `ready: false`, `state: "cell_not_ready"` and `cell_state`: `germinating` before hello,
`draining` once a stop or kill has arrived, `dead` after the guest stopped (a crash, or an unclean
teardown). Otherwise it answers as spec 001 §4 says. `cell.status` relays it as before.

**A failed start.** If the start budget runs out, or the genome attach fails, the controller ends
the cell as `cell.kill` does (§7) with a fresh budget of `stop_deadline_ms`. If `membraned` has
already exited, the controller applies the supervisor check (§8) at once instead. Then it
replies:

- a start timeout: 105 `{from: <last observed state>, to: "ready", cell}`, where the state is
  `germinating` if the supervisor never answered;
- an early exit: 105 `{from: "supervisor_exited", to: "ready", cell}`, with the reason in
  `membraned.log`;
- a failed attach: the attach's own refusal, with `cell` added.

The cell is retired when the cleanup finished. Otherwise it stays in `cells/`, and the refusal
names it in `cell`.

### 7. Ending a cell

`cell.kill` gains three optional parameters. `deadline_ms` is a positive integer, default
`stop_deadline_ms`, or 30,000 without `cell_runtime`; call it D. `operator` and `reason` are
nonblank strings, required when `now` is true: they are spec 008's Force assertion, which spec 001
§3.7 says the reply carries as `force: {operator, reason}`, spec 008's record shape. The reply
carries `force` exactly when `now` is true. Amended on 2026-10-20: this did not name the reply's
field for the assertion, or say when the reply carries it.

The controller marks the cell `draining`, then:

1. **Stop the workload** with spec 024's `membrane.workload.stop`, only when the supervisor's
   `membrane.status` answer shows the guest up or draining: `serving`, `not_serving` or
   `deadline_spent` (the guest is up; only the brokers' answers differ), or `cell_not_ready` with
   `cell_state: "draining"`. Never send it with `now`, whose Force guest stop kills the helper's
   process group anyway. If the status query gets no answer, the stop is skipped and listed as
   `workload_stop` in `incomplete`, since a workload may still run. A stop that fails, is refused
   (`workload_remains`) or times out never blocks the kill: the kill continues and the reply
   reports it. Corrected on 2026-10-20: this said the supervisor reports the cell `ready` or
   `draining`, which are not `membrane.status` states, and gave no rule for a status query with
   no answer.
2. **Withdraw** every attachment, if there is any, in one removal transaction under spec 008:
   safe removal, or Force when `now` is set. Before it, the kill resumes any committed but
   unfinished transaction of the cell, such as a spec 028 expiry, since spec 008 starts no later
   transaction until that one finishes. A drain that times out, or a withdrawal whose
   publication does not settle within D, replies 106 `{handle, deadline_ms}`, with `handle` the
   cell ID and `deadline_ms` D, since a kill withdraws the whole cell in one transaction. Every
   other 106 of a kill carries the same, such as the one below when the `now` session log line
   cannot be appended. The cell stays `draining` for a later `cell.kill`, which publishes first
   (spec 008). Amended on 2026-10-20: this did not say what a kill's 106 carries in `handle` and
   `deadline_ms`.

   With `now`, a resumed transaction that has not finished within step 2's share of D does not
   stop the kill. This is the one case in which the guest ends before withdrawal finishes. The
   kill's own withdrawal is not prepared, since spec 008 starts it only after the resumed one
   finishes. The controller writes a session log line naming the cell, the resumed generation
   and the kill's operator and reason, then goes on to step 3 under that assertion; if the line
   cannot be appended, the reply is 106. When the teardown is clean, its residue `empty`, and
   step 4 holds the lock, so that `membraned` and every child it reaped are gone, that is the
   fresh observation spec 008 requires: every recorded effect of the cell is absent. Holding the
   lock, the controller finishes the resumed transaction on that observation, with no Force,
   then prepares the kill's own withdrawal with its Force assertion, appends its `force` event,
   and commits and finishes it on the same observation, before step 5. Neither is published,
   since no supervisor remains to hold a DesiredCell; spec 008 step 5 allows this one case. A
   teardown that is not clean finishes neither: the cell stays `dead` and listed, with
   `teardown` in `incomplete`, for a later kill, and the resumed generation stays unfinished in
   the journal. Without `now`, the reply stays 106.
3. **Stop the guest** with `membrane.cell.kill` and `DrainSpec {deadline, policy}`, policy
   `Graceful`, or `Force` with `now`. `membraned` sends the 4090 `shutdown` with a smaller
   positive deadline, or none with `Force`. When that passes and the helper still runs, it kills
   the helper's process group and reaps the helper, then tears down (§5, step 8). Its result is
   `{cell, state, clean, residue, helper_killed}`, with `residue_items` exactly when `residue` is
   `items`, and with residue `membraned` observed itself after the teardown. `helper_killed` is a
   required boolean: true when `membraned` killed the helper's process group because the helper
   still ran at the DrainSpec deadline, whether or not it sent `shutdown`. Without `now`,
   `helper_killed: true` makes the controller list `shutdown` (step 6). Corrected on 2026-10-20:
   this named the policy `Safe`, which is not one of the seam's `RevokePolicy` values, `Graceful`
   and `Force`. Amended on 2026-10-20: the result had no `cell` and no `helper_killed`, so the
   controller could not tell when to list `shutdown`, and it wrote `residue_items?` instead of
   saying when `residue_items` is present.
4. **Wait for the lock** to free, by trying `flock` on a fresh descriptor, and keep it.
5. **Retire.** Holding the lock, check that `launched` is absent and the journal has no settled
   attachment and no pending transaction. If the check passes, rename `cells/<cell>` to
   `retired/<cell>`, sync both parent directories, and release the lock; if the rename fails,
   release the lock and reply 105 `{from: "dead", to: "retired", cell}`, and the cell stays listed
   `dead` until a later check retires it. If the check fails, do not retire: decide the cell at
   once by §8's table, as §6 does when `membraned` exits during a start. The lock the kill holds
   counts as the table's `free` row, and the check does not test it again. Reply with `teardown`
   in `incomplete` unless the table retires the cell. If §8's retirement then fails, the cell is
   set aside as §8 says, as `retire_unlogged` or `retire_failed`, and the reply lists `teardown`.
   A cell the table sets aside or quarantines leaves `cell.list`, and `plasmosome.recovery` names
   it. Amended on 2026-10-20: this gave no outcome for a failed check or a failed rename.
6. **Reply** `{cell, state: "dead", drained, residue}` as spec 001 §3.7 shows, with the residue
   from step 3. When a step fell short, the reply adds `incomplete`, a list over a closed set:
   `workload_stop` (it failed, was refused, timed out or had no budget, or was skipped because the
   status query got no answer), `shutdown` (the guest got no `shutdown` for lack of budget, or
   ignored it, and its helper was killed; never listed with `now`) and `teardown` (the cell was
   not retired: its teardown was not clean, or step 5's check failed and §8's table did not retire
   it). A reply is a retirement exactly when `incomplete` lacks `teardown`. Amended on 2026-10-20:
   `workload_stop` did not cover a stop skipped for lack of a status answer, and `teardown` did
   not cover a failed check in step 5.

**Budgets.** D counts from the request's arrival, so a kill may queue behind earlier mutations.
The steps run in order, each with a positive budget, and end within D, the withdrawal's
publication exchange included. The guest gets a positive shutdown budget before its helper is
killed, and the lock wait keeps a reserve; the shares are the implementing task's. Every deadline
sent is at least 1 ms. With `now`, the guest stop's DrainSpec deadline is 1 ms, and the
controller waits for its answer until D, less the lock wait's reserve, has passed since arrival,
and for at least 1 ms. A step left with no budget takes its next escalation: the workload stop is
skipped, the guest stop sends no `shutdown` (each listed in `incomplete`), a withdrawal is a drain
timeout (106), and the lock wait makes one try. Amended on 2026-10-20: this said only that the
guest stop gets 1 ms with `now`, and not how long the controller waits for the answer.

A `dead` cell can always be killed: step 1 is skipped and step 3 finishes its teardown. Outcomes
that do not retire the cell:

- The teardown was not clean (for example, a bridge socket that could not be removed):
  `membraned` answers `{state: "dead", clean: false, residue: "items", ...}` and keeps running.
  `cell.kill` replies with that residue and `teardown` in `incomplete`, and the cell stays `dead`
  for a later kill.
- Step 5's check fails and §8's table does not retire the cell: the reply lists `teardown` in
  `incomplete`, and the cell is set aside or quarantined as the table says. Either way it leaves
  the ordinary registry, so a later `cell.kill` on it is 101: clearing a set-aside cell whose lock
  is free is owner question 3, and spec 008 has no cleanup for a quarantined one. Amended on
  2026-10-20: this outcome is new.
- Step 5's rename fails: 105 `{from: "dead", to: "retired", cell}`. Amended on 2026-10-20: this
  outcome is new.
- The helper is not reaped by the deadline: 105 `{from: "draining", to: "dead", cell}`.
- The lock does not free within D: 105 `{from: "dead", to: "retired", cell}`.

A cell that is retired, or never existed, is 101 `cell <id>`.

### 8. The supervisor check

Spec 008 forbids deriving a cell's state from a PID file or socket existence. The lock and the
marker are neither: a held lock proves a live holder, an absent marker proves no runtime was
launched or it was cleanly ended, and a marker's boot session says whether it predates this boot.
The controller runs this check:

- at startup, for every validated cell directory, through spec 008's no-follow opens;
- while serving, whenever a query to a cell's membrane fails, except during that cell's
  `cell.new`;
- when a `cell.new` finds the lock free while it waits, for that cell only;
- on the next request naming a set-aside cell;
- when `cell.kill`'s retire check fails (§7, step 5), for that cell only, with the lock the kill
  holds. Amended on 2026-10-20: this case is new.

It never touches an entry spec 008 does not validate. To test the lock, the controller tries
`flock(LOCK_EX | LOCK_NB)` on a fresh descriptor; "free" means it got the lock, which it holds
until the row's action is done. A missing `supervisor.lock` is created exclusively, locked, and
counts as free. Whatever the lock, it then reads the `launched` marker before it acts, so every
set-aside entry can name it. Amended on 2026-10-20: this did not say the marker is read when the
lock is held.

At startup the held-lock cells are queried concurrently, each with an answer cap
below `recovery_deadline_ms` that leaves the rest of spec 001 §4.1's single startup deadline for
cleanup and publication; while serving, the cap is `recovery_deadline_ms`. A malformed answer
counts as no answer. Rows are tried in order:

| Lock | Marker | Journal | Outcome |
| --- | --- | --- | --- |
| held | any | any | Query under spec 008 within the cap, waiting for a `germinating` cell. No answer, or still `germinating`, at the cap: set aside as `unresponsive` or `germinating`. |
| free | any | corrupt | Quarantine under spec 008, with the lock state and marker in its report. |
| free | from an earlier boot, or recording a clean stop | no outstanding external obligation | Retire. Every settled holding and pending operation is written to the session log as residue. |
| free | from an earlier boot, or recording a clean stop | an outstanding external obligation | Set aside as `external_obligations`. |
| free | from this boot with no clean stop, or unreadable | any | Set aside as `supervisor_lost` (O-10). |
| free | absent | nothing settled, nothing pending | Retire, with a session log line. |
| free | absent | a settled attachment or a pending operation | Set aside as `holdings_without_supervisor`. |

A missing journal reads as empty, as spec 008 says. An outstanding external obligation is spec
008's published delayed effect or external assertion. Only an operator's Force discharges it, and
retiring never does; which verb carries that Force is owner question 3. A retirement whose session
log line cannot be appended sets the cell aside instead, as `retire_unlogged`. The line is
appended before the rename. A retirement whose rename fails after its line was appended sets the
cell aside as `retire_failed`, and a second session log line says the cell was not retired. If
that second line cannot be appended either, the cell stays set aside as `retire_failed` and the
failure is written to stderr, so the log's last line for the cell is a retirement that did not
happen; `plasmosome.recovery` still lists the cell. These two reasons join the five the table
names. Amended on 2026-10-20: this named no reason for that cell. Amended on 2026-10-20: this did
not say what a failed rename after the line does.

**Set-aside cells** are handled as spec 008 handles quarantine. The cell is absent from the ordinary
registry, so `cell.list` omits it. `plasmosome.recovery` lists it in `set_aside`, with its reason,
its lock state, its marker, its settled holdings and its pending operations (the spec 001 §3.3a
amendment below).
Readiness is false while any cell is set aside, and it is never a reason to refuse startup or to
stop serving other cells. Nobody can run its pending cleanup, reconciliation or publication under
spec 008 without its supervisor, so they wait, and block nothing else. It is not permanent: the
next request naming it, and every startup, runs the check again. A check that adopts it first does
what startup adoption does: spec 008's pending cleanup, generation comparison, reconciliation and
publication, then the no-`created` rule below. `cell.kill` on a set-aside cell whose lock is held
runs §7's steps. Every other verb naming a set-aside cell is 101, and so is `cell.kill` on one
whose lock is free; clearing those is owner question 3. Amended on 2026-10-20: this named no key
for set-aside cells in `plasmosome.recovery`, whose keys spec 001 fixes, and did not list their
settled holdings, which spec 028 §7 requires there.

**Order.** At startup the controller classifies every validated cell first, holding each free lock.
It acts only after that, and only if startup is going to serve. A startup that refuses for another
reason (spec 008's partial discovery, or an adopted cell's unfinished cleanup or publication)
retires nothing and releases the locks.

**A cell adopted with no `created` file**, at a startup that serves or by a later check, is one
whose `cell.new` never replied, so no client knows it exists. The controller ends it as `cell.kill`
does, with a budget of `stop_deadline_ms`, and writes a session log line naming it. If that kill
does not finish, the cell stays listed with its state.

### 9. Isolation of the private socket

Spec 001 §4.1 requires that a cell workload cannot reach `membrane.uds`. The workload runs in a
hardware guest with no virtiofs export and no network device, and its helper is confined by its
host policy (spec 001 §4.2). That policy must deny the helper every Unix socket except this cell's
two bridge sockets, and every write except this cell's `runtime/root.img`. That depends on O-2:
the qualified bundle's policy allows any Unix socket and writes anywhere under
`/usr/local/var/plasmosome/work`, and one hash-pinned policy cannot name one cell's paths without
per-cell parameters, which spec 001 §4.2 does not define. Until O-2 is answered, this section and
item 16 are gated, and the instance root must sit under that writable directory.

## Changes proposed to accepted specs

These are proposed text. The PR that accepts this spec edits no other accepted document. Each
amendment is applied by a later reviewed change to the document it amends, before any task that
relies on it. Where spec 022 or 024 proposes text for the same lines, the text below is the same.

**Spec 001 §1, the 105 row.** Replace "`from`, `to`; private recovery methods additionally carry
the typed `recovery` refusal in §4.1" with:

> `from`, `to`; `detail`, a record of a kind spec 001 defines, including spec 024's; `cell`
> whenever the refusal concerns a created cell (spec 023); private recovery methods additionally
> carry the typed `recovery` refusal in §4.1

**Spec 001 §3.3 (`001:243-247`).** After "Quarantine alone can coexist with a serving controller
when all live observations are complete; readiness remains false.", add:

> A cell that spec 023's supervisor check sets aside is handled the same way: it is not an
> incomplete observation, it coexists with a serving controller, and readiness remains false.

**Spec 001 §3.3a, `plasmosome.recovery` (`001:254-255`).** Replace "`{name, ledger_generation,
desired, tombstones, observed_cells, expected, drift, unmatched, incomplete, pending,
quarantined}`" with "`{name, ledger_generation, desired, tombstones, observed_cells, expected,
drift, unmatched, incomplete, pending, quarantined, set_aside}`", and after that sentence add:

> `set_aside` is the required array of the cells spec 023's supervisor check set aside, in byte
> order of the cell ID string (so `cell-10` sorts before `cell-9`), and `[]` when there is none.
> Each entry is `{cell, reason, lock, marker, desired, pending}`. `reason` is one of
> `unresponsive`, `germinating`, `external_obligations`, `supervisor_lost`,
> `holdings_without_supervisor`, `retire_unlogged` and `retire_failed`. `lock` is `held` or
> `free`. `marker` is required whatever the lock, since the check reads `launched` for every cell
> before it acts: `absent`, `earlier_boot`, `this_boot_stopped`, `this_boot` or `unreadable`, for
> no marker, one from an earlier boot, one from this boot that records a clean stop, one from this
> boot that records none, or one that cannot be read. `desired` is the cell's settled DesiredCell
> from its journal, in this section's projection, so each attachment shows its spec 028
> `lifetime`; an empty journal gives the empty generation-0 record. `pending` is the cell's
> pending transaction in this section's projection, omitted when it has none. Both are omitted
> when the cell's journal is corrupt, which happens only when the lock is held, since a free lock
> with a corrupt journal quarantines; spec 008 then reports that journal under `quarantined`. A
> cell appears in `set_aside` once. A set-aside cell's records appear only in its entry, apart from
> a corrupt journal's report under `quarantined`, and never in the top-level `desired`,
> `observed_cells`, `pending`, `expected`, `drift`, `unmatched`, `incomplete` or `tombstones`.
> Its tombstones stay in its journal and are not reported until it is adopted. `set_aside`
> belongs to the outcome, like every other key: it names the cells set aside when that outcome
> completed. A supervisor check while serving that sets a cell aside, changes its entry,
> quarantines a cell, or adopts or retires a set-aside cell, completes a new outcome in which
> only that cell's records change and `ledger_generation` is recomputed over the adopted cells.
> So a new capture shows the current list, and a capture already pinned keeps its outcome.
> `set_aside` is not added to §4.1's stderr diagnostic, since a set-aside cell never refuses
> startup; the quarantine reports in that diagnostic carry `lock` and `marker` when spec 023's
> check quarantined their cell (spec 008's amendment below).

Amended on 2026-10-20: this item is new. Spec 023 §8 said `plasmosome.recovery` lists set-aside
cells, but no amendment gave them a key, and spec 001 fixes the result's keys.

**Spec 001 §3.5.** Replace the example request and result with this text, which spec 022 proposes
too (whichever lands second finds it applied):

```json
{"id": 5, "method": "cell.new",
 "params": {"kernel": "work", "genome": "researcher", "mock": "simulate",
  "artifact": {"registry_id": "6f1c2a9e-4b7d-4e3a-9f20-8d5b1c7e0a43",
   "release": {"kind": "genome", "population": "user", "publisher": "acme",
    "name": "researcher", "version": "1.0.0",
    "digest": "sha256:e2ddc2cb169322884a76554796ab8cc32f23231e90e50799ed8a25e13b2b9a48"}}}}
```

```json
{"id": 5, "result": {"cell": "cell-3", "state": "ready",
  "plasmids": ["github-pr [mock:simulate]", "workspace [mock:simulate]"]}}
```

After the `genome` bullet, add:

> - The controller allocates the cell ID and starts its supervisor as spec 023 says. It replies
>   only when the guest is up and the genome is attached; until then the cell is `germinating`
>   and refuses `cell.exec`. Resolution refusals come before any cell directory exists; every
>   later refusal carries `cell`, and its cell is ended first. On a host other than Darwin arm64,
>   or without `cell_runtime`, cell.new is 105 `{from: "unsupported_host" | "no_cell_runtime",
>   to: "germinating"}`.

**Spec 001 §3.7.** After "With `"now": true` the reply carries `"drained": false` and the recorded
operator assertion.", add:

> `now` requires `operator` and `reason`, nonblank strings: that assertion, which the reply
> carries as `force: {operator, reason}`, spec 008's record shape, exactly when `now` is true.
> Optional `deadline_ms` bounds the whole kill, default `cell_runtime.stop_deadline_ms` or 30,000.
> The cell is `draining` from the start. The controller stops the workload if the guest runs and
> `now` is not set, withdraws every attachment, sends `membrane.cell.kill`, waits for the
> supervisor to exit, and moves the directory to `<instance>/retired/<cell>` before replying (spec
> 023). A step that falls short is listed in `incomplete`, over the closed set `workload_stop`,
> `shutdown` and `teardown`; a failed workload stop never blocks. Every 106 a kill sends, for a
> drain or publication timeout, a resumed transaction that does not finish without `now`, or a
> `now` kill whose session log line cannot be appended, is `{handle, deadline_ms}` with `handle`
> the cell ID and `deadline_ms` the kill's deadline. An unclean teardown replies `"state": "dead"`
> with its residue and `teardown` in `incomplete`, and the cell stays listed for a retry. A cell
> that fails the check before its retirement (spec 023 §7, step 5) is decided at once by spec 023
> §8's table: unless the table retires it, the reply lists `teardown` in `incomplete`, and the
> cell is set aside or quarantined and leaves `cell.list`; a later `cell.kill` of it is 101, since
> clearing it is spec 023's owner question 3, or for a quarantined cell outside spec 008. A guest
> not stopped or a supervisor not gone within the deadline, or a rename that fails after the
> retire check passed, is 105 naming the cell.

Amended on 2026-10-20: this did not name the reply's field for the assertion or say when the
reply carries it, did not say what every 106 of a kill carries in `handle` and `deadline_ms`, and
gave no outcome for a failed check before retirement or a failed rename.

**Spec 001 §4, `membrane.status`.** Replace "Every call asks every broker again; no answer is
kept." with:

> Every call asks every broker again, and a membrane configured for a cell also reports its cell;
> no answer is kept. Such a membrane is ready exactly when its guest has said hello, no stop or
> kill has arrived, and every broker answers ready.

Replace the `empty` row with these two rows:

> | `cell_not_ready` | `cell_state` — the cell's lifecycle state: `germinating`, `draining` or `dead` |
> | `empty` | none — a membrane with neither a cell nor brokers, which is never ready |

**Spec 001 §4, `membrane.cell.kill`.** After "explicit parameter in the desired record.", add:

> The DrainSpec deadline is positive, and the policy is `Graceful`, or `Force` for `cell.kill
> --now`. The membrane sends the guest `shutdown` within the deadline, kills and reaps the helper
> when it passes, and answers `{cell, state, clean, residue, helper_killed}` with residue it
> observed itself after teardown, and `residue_items` exactly when `residue` is `items`.
> `helper_killed` is a required boolean, true when it killed the helper's process group because
> the helper still ran at the deadline. After a clean teardown it removes
> its launch marker, removes its own socket and exits; otherwise it keeps running with its lock
> and marker (spec 023).

Corrected on 2026-10-20: this named the policy `Safe`, which is not a `RevokePolicy` value, and
gave the result as `{state, clean, residue}`, without `cell`, `residue_items` or `helper_killed`.

**Spec 001 §4.1.** After the paragraph that introduces `registry_root`, add the paragraph below.
Spec 022 inserts after the same paragraph; whichever lands second goes after the other's.

> The same configuration admits optional `cell_runtime: {membraned: Artifact, template,
> start_deadline_ms, stop_deadline_ms}`, spec 023's deployment input for new cells, in which
> `stop_deadline_ms` is optional. Recovery never reads it.

Corrected on 2026-10-20: this said `stop_deadline_ms?`, a notation spec 001 never uses.

In the same section, after "its publication exchange gets one recovery_deadline_ms budget,
likewise not reset on retry.", add "Under `cell.kill`, that exchange also ends within the kill's
deadline (spec 023)."

**Spec 001 §4.2.** After "Recovery reconnects, not relaunches.", add:

> The controller starts each cell's supervisor once, as spec 023 §4 says. The supervisor holds
> the cell's `supervisor.lock` for its whole life, and creates the `launched` marker, which
> records the host boot session, before any runtime resource. Only `membrane.cell.kill`, or a
> launch that fails before hello, removes the marker and the guest's disk.

**Spec 008, writer lock (`008:66-67`).** Replace "takes a nonblocking exclusive OS file lock" with
"takes a nonblocking exclusive `flock(2)` lock on a close-on-exec descriptor".

**Spec 008, quarantine report (`008:475-476`).** After "fault, optional line, lines_parsed, found
objects, found incomplete effects and refusal claims.", add:

> A report for a cell that spec 023's supervisor check quarantined also carries `lock`, which is
> always `free`, since the check quarantines only under a free lock, and `marker`, with the values
> spec 001 §3.3a gives a set-aside cell. Every other report omits both.

Amended on 2026-10-20: this item is new. Spec 023 §8 said a quarantine report carries the lock
state and marker, but spec 008's report had no fields for them.

**Spec 008, startup query (`008:477-483`).** After "the controller must not derive that state
from a PID file, socket existence or inability to connect.", add:

> Before that query, startup applies spec 023's supervisor check to each validated cell. It queries
> only cells whose `supervisor.lock` is held, concurrently, each within a cap below the startup
> deadline. The lock and the `launched` marker are the supervisor's own lifecycle evidence, not a
> PID file or socket existence. A failed query, or a free lock with a marker from this boot that
> records no clean stop, sets that one cell aside: it is handled as quarantine is, and never refuses
> startup. Otherwise a free lock may retire the directory to `<root>/retired/<cell>`, which is not
> discovered again. The same check runs while serving, when a query to a supervisor fails.

**Spec 008, answering requests (`008:484-487`).** After "has been reconciled with its membrane.",
add:

> The pending cleanup here is an adopted cell's. A cell that spec 023's supervisor check sets
> aside is not adopted: its pending cleanup, reconciliation and publication wait, and the check
> that later adopts it runs them before the cell serves.

**Spec 008, refusing startup (`008:537-540`).** After "exits nonzero rather than serving a partial
instance.", add "A cell set aside under spec 023 is not incomplete observation, cleanup or
publication for this rule."

**Spec 008, unlinking (`008:543-546`).** After "as the existing daemon does.", add:

> Retiring a cell under spec 023 renames its whole directory, journal and any stale
> `membrane.uds` included, to `<root>/retired/<cell>`; nothing is unlinked. The rule for the
> instance's control socket is unchanged.

**Spec 008, R8.** Replace "Unknown observation prevents startup success." with "Unknown
observation keeps readiness false and sets that cell aside; it does not prevent startup (spec
023)."

**Spec 008, R9.** Replace "An unavailable supervisor, partial discovery, unfinished cleanup or
unresolved publication prevents serving" with:

> An unavailable supervisor sets only its cell aside under spec 023, with readiness false, and
> that cell's cleanup and publication wait until it is adopted. Partial discovery, or an adopted
> cell's unfinished cleanup or unresolved publication, prevents serving


## Open questions

These need the owner. Where this spec needs an answer to work, it sets a conservative default.

1. **O-10, an orphaned guest.** A SIGKILLed `membraned` leaves its helper and guest running. This
   spec sets that cell aside and does not clean it. Accept the documented limit, or amend spec
   001 §4.2 to give the helper a lifeline? (Handing the helper an inherited copy of
   `supervisor.lock` would be one: a free lock would then also prove the helper gone.)
2. **Relaunch after a crash.** After a host restart or a lost supervisor, the cell is retired or
   set aside, never relaunched, and its `root.img` is kept (the default, matching spec 008's
   out-of-scope line). Should a cell come back after a host restart, as intent 003's title
   suggests?
3. **Clearing a set-aside cell.** A cell whose lock is free stays set aside until a check finds
   it otherwise (the default). Until answered, an operator who has confirmed that no `membraned
   --cell=<id>` and no helper of that cell runs may move its directory into `retired/`, and must
   never delete it, or its ID can be allocated again. A move would drop an
   `external_obligations` cell's obligations, so that route is not for one. Should there be an
   operator verb, for example a Force retire that records an operator assertion, and what may it
   assume about a lost guest?
4. **Retired directories and ID reuse.** They are kept, guest disk included, so IDs are never
   reused (the default). Pruning them would let IDs repeat unless a high-water record is kept.
   Who prunes, and when?
5. **Does `plasmosome.stop` end cells?** Spec 001 §3.4 says a graceful stop drains every cell, then
   stops the controller, which would strip every plasmid from cells that keep running. The default
   is §3.4 as written: stop drains every cell, and the cells outlive the controller, empty. This
   spec does not implement stop. Should stop end cells, drain them, or leave them untouched?

O-2 is not a question this spec asks, but §9 and item 16 are gated on it.

## Acceptance

Each item names the broken implementation it catches. Unless it says otherwise, an item runs on
Darwin with the fixture test helper, a real, verified test artifact under spec 001 §4.2's checks.
Items that attach plasmids also need the RealCellBackend task and spec 022.

1. A helper that holds hello until released. While held, `cell.new` has not replied, `cell.status`
   shows `germinating`, `cell.exec` is 105 `{from: "germinating"}`, and `membrane.status` is
   `cell_not_ready` with `cell_state: "germinating"`. After release, `cell.new` replies `ready` and
   `created` exists. With a genome whose attach the test holds at prepare, `membrane.status` is not
   `cell_not_ready`, yet `cell.status` shows `germinating` and `cell.exec` refuses until `cell.new`
   replies `ready`. Catches: replying on a fork or a bound socket, a deadlock between ready and
   attach, and admitting work before the genome is attached.
2. A helper that never sends hello. When `cell.new` replies 105 `{from: "germinating", to:
   "ready", cell}`, after `start_deadline_ms` and within `stop_deadline_ms` more, the directory is
   under `retired/` with no `launched`, and no helper or `membraned` remains. Catches: leaving
   cleanup to a later startup, and charging cleanup to the spent start budget.
3. A helper that exits before hello: 105 `{from: "supervisor_exited", to: "ready", cell}` well
   before `start_deadline_ms`, with the reason in `membraned.log`. Catches: waiting out the start
   budget for a supervisor that is gone.
4. With a ready cell, SIGKILL the controller's whole process group and hang up its session.
   `membraned` and the helper keep running; `membraned`'s `getsid` and `getpgid` differ from the
   controller's, and its parent is PID 1. A new controller adopts the cell, and no second
   `membraned` starts. Catches: a shared session or process group, an inherited instance lock,
   and a recovery that relaunches.
5. Plant a descriptor without close-on-exec in the controller before `cell.new`. In `membraned`:
   the environment is empty, argv carries the cell ID, descriptors 0 to 4 are as stated (3 closed
   after reading), the planted and every controller descriptor are absent, and no config file
   exists. A probe trying `flock(LOCK_EX | LOCK_NB)` on a fresh descriptor, from directory creation
   until `membraned` exits, never succeeds. Catches: a leaked descriptor, configuration through the
   environment or a file, and handing over an unlocked descriptor.
6. `membraned` started by the test with descriptor 4 set to another cell's locked
   `supervisor.lock`, an unrelated file, or this cell's lock file opened without the lock while the
   test holds it: each exits nonzero without binding a socket or creating `launched` or
   `runtime/`. So does one whose directory was renamed to `retired/` before start. Catches:
   trusting the descriptor unchecked, and launching in a retired directory.
7. A config record with no LF before EOF, one over 1,048,576 bytes before its LF, and one followed
   by a second line: each `membraned` exits without binding a socket or creating `launched`, and
   the controller retires the directory. A record of exactly 1,048,576 bytes before its LF is
   accepted. Catches: acting on a partial record, and counting the LF toward the bound.
   Amended on 2026-10-20: this did not test a record of exactly 1,048,576 bytes.
8. Two `cell.new` requests at once get distinct IDs and directories. While one waits on a held
   hello, `cell.status` of another cell answers at once. A `cell.kill` and a serving-time check of
   one cell, started together, run one after the other, and the directory is retired once.
   Catches: unserialized allocation, a controller blocked by one boot, and two holders of one lock.
9. Startup over these validated cells, built by hand:
   - A: lock free, `launched` from this boot, empty journal, no `membrane.uds`;
   - B: lock free, no `launched`, a stale `membrane.uds`, empty journal;
   - C: lock free, `launched` from another boot, a journal with one settled attachment;
   - D: lock free, no `launched`, a journal with a settled attachment;
   - E: lock held by a test process that never answers, a journal with a prepared transaction;
   - F: a healthy supervisor whose settled generation still needs publication.

   Startup serves within `recovery_deadline_ms`, with F adopted and published. A, D and E are absent
   from `cell.list`, listed by `plasmosome.recovery` as `supervisor_lost`,
   `holdings_without_supervisor` and `unresponsive`, E with its transaction left uncleaned, and
   readiness is false. D's `set_aside` entry carries its settled attachment, with its `lifetime`,
   in `desired`, and E's carries its prepared transaction in `pending`; neither appears in the
   top-level `desired` or `pending`. B and C are under `retired/`, and the session log names C's
   holding as residue. A retired directory with a corrupt journal is ignored. Catches: deciding
   from socket existence, one hung supervisor spending the startup deadline, a cell from before a
   host restart blocking forever, and a set-aside entry with an empty `desired` or no `pending`.
   Amended on 2026-10-20: this did not check a set-aside entry's holdings.
10. The same set, plus an invalid configuration that makes startup refuse after discovery: nothing
    is retired. Catches: acting before every cell is classified.
11. While serving, SIGTERM the `membraned` of a cell with an attachment: it stops the guest,
    records a clean stop in `launched` and exits. `cell.status` on it is then 101, the session log
    names its holding, and `retired/<cell>/runtime/root.img` exists. SIGKILL another cell's
    `membraned`: `supervisor_lost`, and a third cell still answers; a startup with a different
    boot session in its marker retires it. Stall a fourth, with a prepared transaction and no
    `created`, past its query, then resume it: the next request naming it adopts it, cleans up the
    transaction, ends it and logs it. Catches: a SIGTERM that destroys the guest disk, a clean
    stop left set aside until reboot, one cell taking down the instance, a set-aside state that
    never clears, and an adoption that skips cleanup or keeps an unrequested cell.
12. Stop the controller after it spawned `membraned` and before hello. `membraned` gives up at
    `boot_deadline_ms` and exits; a new controller started after that exit retires the directory.
    Repeat with a helper that sends hello after the controller died, and start the new controller
    after hello: it adopts the cell, finds no `created`, ends it and logs it. Catches: a boot that
    outlives its deadline, and an unrequested cell left running.
13. `cell.kill` with D = 30,000 on a cell with two empty attachments, through a membrane double.
    Requests come in order, `membrane.workload.stop`, the withdrawal, `membrane.cell.kill`, each
    deadline positive and within the previous bound, with the 4090 `shutdown` deadline below the
    `membrane.cell.kill` one. While the double holds the withdrawal, `cell.status` shows `draining`
    and `cell.exec` is 105 `{from: "draining"}`. Then `membraned` has exited, the directory is
    `retired/<cell>`, the reply's residue came from `membraned`, and the next `cell.new` gets a new
    ID. With `deadline_ms` 1, and queued behind a held `cell.new` until D is spent, every deadline
    sent is positive, the reply is 106 and the cell stays `draining`. With `now`, no workload stop
    is sent, the cell is retired, and the reply has no `incomplete` and carries `force` with the
    request's `operator` and `reason`. A kill without `now` that passes `operator` and `reason`
    replies with no `force`. Catches: ending the guest before withdrawing, other than past a
    resumed transaction under `now` (§7 step 2), a zero deadline, residue from controller memory,
    ID reuse, a `now` reply without its assertion, and a `force` echoed without `now`. Amended on
    2026-10-20: this did not check that the reply carries `force`, or that it carries none without
    `now`.
14. A guest that ignores `shutdown`: with `deadline_ms` 10,000, `cell.kill` replies within 10
    seconds, the helper is gone and `incomplete` lists `shutdown`. A crashed guest: no workload stop
    is sent and the cell is retired. A workload stop refused with `workload_remains`, or never
    answered: the cell is retired and `incomplete` lists `workload_stop`. An `unresponsive` cell
    with no attachments whose lock is held: the kill runs instead of replying 101, and with the
    supervisor still silent replies 105 `{from: "draining", to: "dead", cell}`. An unremovable
    bridge socket: `state: "dead"` with residue items and `teardown` in `incomplete`, `membraned`
    still runs and the cell is listed `dead`; once removable, a repeat retires it. Catches: an
    unbounded kill, a cell that cannot be ended, and retiring after a failed teardown.
15. A `membraned` under a group-writable parent, a group-writable `membraned`, and a wrong digest
    for `membraned` or a template Artifact: each `cell.new` is 108 with the path, and no cell
    directory exists. Without `cell_runtime`, `cell.new` is the stated 105 with no directory, and
    a controller restarted without it over live cells recovers them. Catches: a hash-only check,
    allocating before a refusal, and recovery that needs deployment input.
16. **Gated on O-2; Darwin pinned runtime.** Under the real host policy, the helper cannot connect
    to its own `membrane.uds`, another cell's `control.uds` or the control socket, nor write
    outside its `runtime/root.img`. A policy mutant that widens the Unix-socket rules to the cell
    directory lets one connect, and the test fails. Catches: a policy wider than `runtime/`.
17. A genome whose resolution refuses (spec 022's 103) creates no cell directory; this half needs
    a registry-imported genome (spec 020). A genome whose apply fails after boot leaves a retired
    directory, no running `membraned`, and a refusal carrying `cell`. Catches: creating a cell
    before resolution, and an empty running cell after a failed attach.

## Out of scope

- What runs inside the guest, and how commands reach it. That is spec 024.
- How plasmids become capabilities. That is spec 022.
- Cleaning an orphaned guest after a SIGKILLed supervisor (O-10).
- Relaunching a cell after a host or supervisor crash.
- A client request ID on `cell.new`, which spec 025 asks for; that is task plasmosome-bnkf.
- Instance roots on NFS, SMB or FUSE filesystems.
- `cell.clone`, `cell.save`, `cell.load` and `freeze`.
