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
cell directory, and starts `membraned` detached from itself, with a configuration record on a
pipe and an empty environment. `membraned` boots the guest, and `cell.new` answers once the cell
is ready. `cell.kill` withdraws everything the cell holds, asks `membraned` to shut the guest
down, waits for it to exit, and moves the cell directory to `<root>/retired/<cell>`.

Two files in the cell directory make a dead supervisor safe to reason about. `supervisor.lock`
is held by `membraned` for its whole life, so a free lock proves no supervisor is running. The
`launched` marker is created just before `membraned` creates any runtime resource and removed
only after a clean teardown, so a missing marker proves no virtual machine can still be running
from that directory. A restarted controller uses both. A free lock with no marker and nothing
attached is a cell that never ran or ended cleanly: the controller retires it. A free lock with
the marker present may be a guest running without its supervisor: startup refuses and names the
cell, because nothing may decide that cell's state by guessing. This replaces the earlier idea of
treating a missing `membrane.uds` as proof that no guest ran, which is false: `membraned` removes
its socket on every exit, clean or not.

This serves intent 003, cells come back after a crash: a cell must survive the controller, and a
restarted controller must tell a live cell, a cleanly ended one and an unaccounted one apart
without a PID file. It serves intent 009, a command line an agent can drive: `cell.new` returns a
cell that is ready to use, or a named refusal, never a cell that may or may not be booting. The
controller never starts a second supervisor for an existing cell directory. Recovery reconnects;
it never relaunches.

**Platform.** The product is macOS first: Darwin arm64 is the only runtime host, and the cell is
a Linux guest run by libkrun. Linux/KVM host support is deferred, not dropped; on Linux,
`membraned` refuses to launch with a typed unsupported-host reason. A real guest reaches ready
only after owner gates O-1 and O-6 are answered and O-7 is settled, because the guest policy must
be loaded before its hello. Until then, the lock, marker, launch and retirement rules are proved
with a test helper at the Darwin OS mechanism level. Evidence for each acceptance item says which
level it reached: the model, the Darwin OS mechanism, the Darwin pinned runtime, or Linux, which
is deferred.

## Contract

### 1. Deployment input

The controller's strict JSON configuration gains an optional key, `cell_runtime`:

```json
"cell_runtime": {
  "membraned": {"path": "/usr/local/lib/plasmosome/membraned", "sha256": "<64 hex>"},
  "template": {"version": 1, "vcpus": 2, "memory_mib": 2048, "...": "..."},
  "start_deadline_ms": 60000
}
```

- `membraned` is an Artifact as spec 001 §4.2 defines it.
- `template` is a RuntimeRecipe without `writable_root`, `control_path` and `data_path`. Every
  other RuntimeRecipe field is required and follows spec 001 §4.2. The controller adds the three
  per-cell paths (section 3). A template that carries any of the three, or lacks any other field,
  is invalid configuration.
- `start_deadline_ms` is a positive integer: the whole budget for `cell.new` to reach ready.

Without `cell_runtime`, `cell.new` refuses with code 101, `target` `cell_runtime`. Startup,
recovery, `cell.kill` and every observation work without it. A running cell never needs its
deployment input again: `membraned` retains the recipe it was given.

The same immutability rule spec 001 §4.2 applies to the helper applies here. The controller
verifies the `membraned` file's SHA256 before exec. The file and its ancestors must not be
replaceable by another principal; a replaceable parent or mutable file is refused, not trusted
after hashing. A mismatch refuses `cell.new` with code 108, `path` the artifact path, before any
cell directory exists.

### 2. Cell IDs

The controller allocates the ID. It is `cell-<n>`, where `n` is one more than the largest such
number found under `<root>/cells/` or `<root>/retired/`, or 1 when there is none. An ID present in
either directory is never allocated again. Allocation and directory creation happen under the
instance writer lock, so two `cell.new` requests cannot race.

### 3. The cell directory

```
<root>/cells/<cell>/          created by the controller, exclusive, mode 0700, synced
  supervisor.lock             held by membraned for its whole life
  launched                    present from before launch until a clean teardown
  membrane.uds                membraned's private socket (spec 001 §4, §4.1)
  ledger.ndjson               the cell's journal (spec 008), written by the controller
  membraned.log               membraned's stderr
  runtime/                    created by membraned, mode 0700
    root.img                  writable_root
    control.uds               control_path, the 4090 listener
    data.uds                  data_path, the 4091 listener
<root>/retired/<cell>/        a retired cell, same layout, kept
```

The controller creates `cells/<cell>` and nothing else in it before starting `membraned`. It
creates `retired/` the same way as `cells/` the first time it needs it. Spec 008's discovery
reads `cells/` only; a retired directory is never queried or recovered.

### 4. How the controller starts membraned

1. Resolve the genome, if one is named, through D2b and spec 022, before allocating anything.
   Every refusal that resolution can find is returned before a cell directory exists.
2. Allocate the ID and create the directory.
3. Start `membraned` by direct exec, not a shell. argv is exactly
   `[membraned.path, "--config-fd=3"]`. The environment is an explicit empty array.
   Descriptor 0 and 1 are `/dev/null`, descriptor 2 is `membraned.log` opened for append, and
   descriptor 3 is the read end of a pipe holding exactly one config record and then EOF. No
   other descriptor survives exec. In particular the controller's instance lock, control socket
   and journal handles never reach `membraned`.
4. `membraned` runs in its own session and is not the controller's child. The controller forks
   twice so that the operating system's init process (launchd on macOS) reaps `membraned`. The
   controller reaps only the short-lived intermediate process. It keeps no PID and never signals
   `membraned`; every later exchange goes through `membrane.uds`.
5. Wait for `membrane.status` on `membrane.uds` to answer ready, polling within the remaining
   start budget.
6. With a genome, attach its members in one transaction under spec 008.
7. Reply.

The config record is one line of strict JSON with exactly these keys: `cell`, `instance_root`,
`runtime_recipe` (the template plus the three per-cell paths) and `boot_deadline_ms` (the
remaining start budget at spawn). It carries no `control_socket` and no `brokers`. A
cell-configured `membraned` serves every method of spec 001 §4, `membrane.status` included, on
`membrane.uds`, and binds no second socket. No config file is written to disk.

### 5. What membraned does, in order

`membraned` first reads descriptor 3 to EOF, parses the record strictly and closes the
descriptor. A malformed record, or anything after it, makes it exit nonzero before step 1.

1. Take `supervisor.lock` with a nonblocking exclusive lock, creating the file if it is missing.
   If the lock is busy, exit without touching anything else.
2. While holding the lock, open `<instance_root>/cells/<cell>` again by name and refuse unless it
   is the same directory (device and inode) as the one whose lock it holds. This stops a late
   `membraned` from launching inside a directory the controller has already retired.
3. Bind `membrane.uds` under spec 001 §4.1's private socket rules.
4. Create `launched` exclusively, and sync it and the directory.
5. Create `runtime/`, copy the root image, bind the two bridge listeners and launch the helper,
   as spec 001 §4.2 says.
6. Serve until `membrane.cell.kill`, SIGTERM or SIGINT. Then tear down. Remove `launched` only
   after a clean teardown: the helper reaped, both bridge sockets and `root.img` removed.
7. Remove its own `membrane.uds` by identity, then exit. The lock is released by exit. The one
   exception is an unclean teardown asked for by `membrane.cell.kill`: then `membraned` keeps
   serving, with its lock and marker, so the cell stays visible (section 7).

A launch failure follows the same order: the attempt releases what it created, `launched` is
removed only if that release was clean, and `membraned` exits nonzero. A SIGKILL runs none of
step 6 or 7: the lock is released, `launched` stays, and the helper and guest may keep running.
That is owner decision O-10; this spec makes such a cell visible and does not clean it.

### 6. Readiness

`membrane.status` covers the cell. A cell-configured membrane answers `{ready: true, state:
"serving"}` exactly when its cell's lifecycle state is `ready`. Otherwise it answers `ready:
false`, `state: "cell_not_ready"` and `cell_state` with the lifecycle state (`germinating`,
`draining` or `dead`). `empty` stays for a membrane with neither a cell nor brokers.
`cell.status` relays this answer in its `supervisor` field as before.

If `start_deadline_ms` runs out before ready, or the genome attach fails after boot, the
controller ends the cell as `cell.kill` does (section 7), then replies. If `membraned` has
already exited, the controller applies the supervisor check of section 8 instead. A timeout
replies 105 `{from: <last observed state>, to: "ready"}`. A failed attach replies with the
attach's own refusal. Either way the cell is retired when the kill succeeds; if the kill itself
cannot finish, the cell stays in `cells/` and the reply is 105.

### 7. Ending a cell

`cell.kill` does these steps in order:

1. Withdraw every attachment, if there is any, in one removal transaction under spec 008: safe
   removal, or Force when `now` is set, with the operator assertion spec 001 §3.7 records.
2. Send `membrane.cell.kill`. `membraned` sends the 4090 `shutdown`, waits for the helper to be
   reaped, and tears down the runtime. After a clean teardown it removes `launched`, answers
   `dead`, removes its socket and exits. After an unclean teardown it answers 105
   `{from: "draining", to: "dead"}` and keeps running, holding its lock and marker, so the cell
   stays visible. A later `membrane.cell.kill` retries the parts that did not finish.
3. Wait, within the request's deadline, for `supervisor.lock` to become free, and take it.
4. Holding the lock, check that `launched` is absent and the journal has no settled attachment
   and no pending transaction. Rename `cells/<cell>` to `retired/<cell>` and sync both parent
   directories. Then release the lock.
5. Reply `{cell, state: "dead", drained, residue}` as spec 001 §3.7 shows.

A timeout or a failed check at any step replies 105 and leaves the cell in `cells/` for a retry.
A cell that is already retired, or never existed, is 101 `cell <id>`.

### 8. The supervisor check, at startup and after

Spec 008 tells startup to query each cell's `membrane.uds` and forbids deriving state from a PID
file or socket existence. The lock and the marker are not those. A held lock proves a live
holder. An absent marker proves the runtime was never launched or was cleanly torn down. The
controller runs this check for each directory under `cells/` at startup, before querying, and
again whenever a query to a cell's membrane fails:

| Lock | Marker | Journal | Result |
| --- | --- | --- | --- |
| held | any | any | Query `membrane.uds` under spec 008. The query may repeat within the remaining deadline, so a supervisor still binding its socket is not taken for a dead one. |
| free | present | any | Observation failure naming the cell. Startup refuses. |
| free | absent | valid, no settled attachment, no pending transaction | Retire the directory while holding the lock, as in section 7 step 4. |
| free | absent | attachment or pending transaction | Observation failure naming the cell. |
| free | absent | corrupt | Quarantine, with observation unavailable, as spec 008 already does. |

"Free" means the controller took the lock itself, creating the file if missing. It holds the lock
until the row's action is done, so a late `membraned` cannot start in between.

### 9. Isolation of the private socket

Spec 001 §4.1 requires that a cell workload cannot reach `membrane.uds`. Here the workload runs in
a hardware guest with no virtiofs export and no network device, and the helper that runs it is
confined by its host policy (spec 001 §4.2). That policy admits the artifacts, `runtime/root.img`
and the two bridge sockets, and nothing else in the cell directory. A host policy that admits the
whole cell directory does not meet this.

## Changes proposed to accepted specs

These are proposed text, not edits made in this PR.

**Spec 001 §3.5.** Change the example result's `"state": "germinating"` to `"state": "ready"`,
and add this bullet after the `genome` bullet:

> - The controller allocates the cell ID and starts the cell's supervisor as spec023 says. It
>   replies only when the supervisor reports the cell ready. Refusals that resolving the genome
>   can find are returned before any cell directory exists. A cell that misses the configured
>   start deadline, or whose genome attach fails after boot, is killed and retired before the
>   reply, which is105 `{from, to:"ready"}` or the attach's own refusal. Without configured
>   `cell_runtime`, cell.new is101 with `target` `cell_runtime`.

**Spec 001 §3.7.** After the paragraph beginning "With `"now": true`", add:

> The controller first withdraws every attachment in one removal transaction, then sends
> `membrane.cell.kill`. After a clean teardown the supervisor exits, and the controller moves the
> cell directory to `<instance>/retired/<cell>` (spec023) before replying. A timeout or unclean
> teardown is105 and leaves the cell in place for a retry. A retired cell is101.

**Spec 001 §4, `membrane.status`.** Add a table row and a sentence:

> | `cell_not_ready` | `cell_state` — the cell's lifecycle state: `germinating`, `draining` or `dead` |
>
> A membrane configured for a cell is ready exactly when its cell is `ready`, and answers
> `cell_not_ready` otherwise. `empty` is a membrane with neither a cell nor brokers.

**Spec 001 §4, `membrane.cell.kill`.** Append:

> After a clean teardown the membrane removes its launch marker, answers `dead`, removes its own
> socket and exits. After an unclean teardown it answers105 and keeps running with its lock and
> marker. A membrane configured for a cell serves every method of this section on
> `membrane.uds` and binds no second socket.

**Spec 001 §4.1.** After the paragraph introducing `registry_root`, add:

> The same configuration admits optional `cell_runtime: {membraned: Artifact, template,
> start_deadline_ms}`, spec023's deployment input for new cells. Recovery never reads it.

**Spec 001 §4.2.** After "Recovery reconnects, not relaunches.", add:

> The controller starts each cell's supervisor once, detached in its own session, with argv
> `[membraned.path,"--config-fd=3"]`, an explicit empty environment, and only descriptors0–3.
> The supervisor holds the cell's `supervisor.lock` for its whole life, binds its socket, and
> creates the `launched` marker before any runtime resource; it removes the marker only after a
> clean teardown (spec023).

**Spec 008, "Quarantine and startup integration".** After the paragraph that ends "socket
existence or inability to connect.", add:

> Before that query, startup applies spec023's supervisor check to each validated cell. A held
> `supervisor.lock` leads to the query above, which may repeat within the startup deadline. A
> free lock with the `launched` marker present is an observation failure. A free lock, no marker,
> and a valid journal with no settled attachment or pending transaction retires the directory to
> `<root>/retired/<cell>` under that lock. A free lock and no marker with an attachment or pending
> transaction is an observation failure; a corrupt journal is quarantined with its observation
> unavailable. The lock and marker are the supervisor's own lifecycle evidence, not a PID file or
> socket existence. Retired directories are not discovered.

## Open questions

These need the owner. This spec does not decide them.

1. **O-10, an orphaned guest.** A SIGKILLed `membraned` leaves its helper and guest running. This
   spec makes that cell visible (startup refuses and names it) but does not clean it. Accept the
   documented limit, or amend spec 001 §4.2 to give the helper a lifeline?
2. **O-8, the distinct-UID witness.** Proving that another local user cannot reach `membrane.uds`
   needs a second user or a sudo fixture on this Mac. Without one, that witness is recorded as
   unproved on Darwin.
3. **Retired directories.** They are kept, and they hold only the journal, log, lock and an empty
   `runtime/` after a clean teardown. Should they be pruned, and by whom?
4. **Double fork or a launchd job.** Double fork keeps the controller free of a launchd
   dependency. A launchd job adds the operating system's own record of the process, but its
   restart policy must be off, because a cell must never be relaunched.
5. **Per-request sizing.** Every cell gets the template's `vcpus` and `memory_mib`. Should
   `cell.new` accept smaller values?
6. **A dead supervisor with holdings.** A free lock, no marker, and recorded attachments refuses
   startup. What operator action clears it: a Force removal with no supervisor, or manual
   repair with a named procedure?

## Acceptance

Each item names the broken implementation it catches.

1. `cell.new` with a test helper that completes the handshake replies `state: "ready"`, and
   `membrane.status` was `cell_not_ready` with `cell_state: "germinating"` before that. Catches:
   replying ready on a successful fork or a bound socket.
2. A test helper that never sends hello: `cell.new` replies 105 `{from: "germinating", to:
   "ready"}` after `start_deadline_ms`, the directory is under `retired/` with no `launched`, and
   no helper or `membraned` process remains. Catches: leaving a half-booted guest behind a
   refused request.
3. SIGKILL the controller while a cell is ready. `membraned` and the helper keep running. A new
   controller starts at once, takes `controller.lock` and recovers the cell, and nothing starts a
   second `membraned`. Catches: `membraned` inheriting the instance lock, sharing the controller's
   session or process group, or a recovery that relaunches.
4. Inspect a started `membraned`: its environment is empty, it has descriptors 0 to 2 as stated,
   descriptor 3 is closed after reading, it holds none of the controller's descriptors, and no
   config file exists in the cell directory. Catches:
   configuration through the environment or a file on disk, and a leaked descriptor.
5. Hold `supervisor.lock` from the test and start `membraned`: it exits without binding a socket,
   creating `launched` or creating `runtime/`. Catches: a missing or blocking lock.
6. Rename the cell directory to `retired/` after `membraned` opened it and before it takes the
   lock: it exits without launching. Catches: launching inside a retired directory.
7. Startup differential. Directory A: lock free, `launched` present, no `membrane.uds`.
   Directory B: lock free, no `launched`, a stale `membrane.uds` socket file, empty journal.
   Startup refuses and names A. Remove A's marker: startup retires B and serves. Catches: any
   decision made from socket existence, in either direction.
8. Lock held by a live `membraned` that has not yet bound its socket: startup waits within its
   deadline and adopts the cell, and does not retire it. Catches: retiring a cell that is still
   starting.
9. A lock-free directory with no marker but a settled attachment: startup refuses with an
   observation failure naming the cell. Catches: retiring a cell whose holdings were never
   withdrawn.
10. `cell.kill` on a cell with two attachments: the journal shows the removal transaction
    finished before `membrane.cell.kill` was sent, `membraned` has exited, the directory is
    `retired/<cell>`, `cell.status` is 101, and the next `cell.new` gets a new ID. Catches: ending
    the guest before withdrawing grants, deleting evidence, and reusing an ID.
11. A helper whose teardown cannot finish: `cell.kill` replies 105, `membraned` is still running
    with its lock and marker, and the cell stays in `cells/`. Catches: retiring a cell whose
    guest may still run.
12. A wrong `membraned` digest: `cell.new` replies 108 with the artifact path, and no cell
    directory is created. Catches: exec of an unverified supervisor.
13. Without `cell_runtime`, `cell.new` replies 101 `cell_runtime`, and a controller restarted
    over existing live cells still recovers them. Catches: recovery that depends on deployment
    input.
14. A genome whose resolution refuses (spec 022's 103) creates no cell directory. A genome whose
    apply fails after boot leaves a retired directory and no running `membraned`. Catches:
    creating a cell before resolution, and leaving an empty running cell after a failed attach.
15. Under the real host policy, the helper's attempt to connect to `membrane.uds` fails with a
    permission error. A policy mutant that admits the whole cell directory lets it connect, and
    the test fails. Catches: a host policy wider than `runtime/`.

## Out of scope

- What runs inside the guest, and how commands reach it. That is spec 024.
- How plasmids become capabilities. That is spec 022.
- `plasmosome.stop`. A cell outlives a stopped controller exactly as it outlives a crashed one;
  `cell.kill` is the only verb that ends a cell.
- Cleaning an orphaned guest after a SIGKILLed supervisor (O-10).
- `cell.clone`, `cell.save`, `cell.load` and `freeze`.
