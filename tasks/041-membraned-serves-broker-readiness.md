---
id: 041
title: membraned answers membrane.status for its broker set
status: done
priority: 2
specs: [001]
intents: [003, 004, 009, 012]
refs: [crates/plasmosome-membrane/src/lib.rs, crates/plasmosome-membrane/src/main.rs, crates/plasmosome-membrane/src/brokers.rs, crates/plasmosome-membrane/src/readiness.rs, crates/plasmosome-membrane/src/vmm.rs, crates/plasmosome-membrane/AGENTS.md, AGENTS.md, docs/specs/001-control-protocol.md, docs/specs/013-what-earns-a-guard.md, .agents/skills/tasks/SKILL.md, .agents/skills/pr-review/SKILL.md, docs/templates/task.md]
done_when:
  - A membrane.status request on membraned's control socket is answered with the broker set's readiness, re-probed on every call.
  - SIGTERM ends the process with exit 0, the socket path removed, and no broker left running.
  - A start that cannot bind or cannot spawn leaves no child and no socket file.
  - The section 1 envelope edges hold - -32700 continues, over-cap -32600 closes, at-cap is served, -32601 for an unknown method.
  - All five gate commands exit 0, reported as bare exit codes.
  - The chain walks - this task names spec 001 and intents 003, 004, 009, 012, reaches in_review with pr: set, and the pull request is a draft whose body ends with `task: 041`.
pr: 70
evidence: squash commit b0d24e4 on main; membraned answers membrane.status with the broker set's readiness re-probed per call, SIGTERM leaves no socket and no broker running, and the section 1 envelope edges hold
---

## Why

Spec 001 section 4's `membrane.status` has a probe and no daemon answering it; this delivers the
answering side.

## Plan

### Deliverable, in one sentence

`membraned` reads a JSON config naming its control socket and its brokers, spawns the broker set
through the existing `BrokerSet`, serves the 001 section 1 ndjson envelope on the control socket,
answers `membrane.status` with the set's readiness, and on SIGTERM/SIGINT tears down — brokers
killed and reaped, socket path removed, exit 0.

### Out of scope

- `brokers.rs` and `vmm.rs`: zero edits. `readiness.rs`: only the appended `ControlSocketProbe`
  and one test.
- No shim, no vsock bridge, no broker-management wire verbs (`membrane.cell.desired/observe/kill`,
  `membrane.residue.snapshot` — all stay unimplemented), per `lib.rs`'s own deferral and 001
  section 4 RESERVED.
- No new error codes; no change to 001 beyond the section 4 `membrane.status` bullet.
- No new dependencies, dev-dependencies included; `Cargo.toml` and `Cargo.lock` unchanged.
- No build-failing guard of any kind — nothing here names a permanent or public consequence, so
  spec 013 forbids one.
- No other crate, no CI workflow, no skill file, no `.githooks` change.

### Files to read, and nothing else

`crates/plasmosome-membrane/src/{lib,main,brokers,readiness,vmm}.rs`,
`crates/plasmosome-membrane/AGENTS.md`, root `AGENTS.md`,
`docs/specs/001-control-protocol.md` (sections 1 and 4), `docs/specs/013-what-earns-a-guard.md`,
`.agents/skills/tasks/SKILL.md`, `.agents/skills/pr-review/SKILL.md`, `docs/templates/task.md`.
Do not explore beyond these.

### Config

JSON, parsed with `serde_json`, already a dependency:

```json
{"control_socket": "/…/c.uds",
 "status_deadline_ms": 500,
 "brokers": [{"name": "egressd", "control_socket": "/…/b0.uds", "command": ["sleep", "300"]}]}
```

`control_socket` required. `status_deadline_ms` optional, default 500, zero refused. `brokers`
optional, default empty. Refused, each with an error naming the offender: unknown keys at the top
level and per broker, duplicate broker `name`, empty `name`, empty `command`. Duplicate broker
sockets are not a parse error — `BrokerSet::spawn` already refuses them and the daemon surfaces
that.

### Wire behavior

Section 1 of spec 001, closed JSON-RPC reserve codes only; the application table is untouched.

- ndjson, one request per line, replies in order per connection, connections served sequentially.
- Line cap 1,048,576 bytes before the newline: over it the reply is `-32600` under a null id and
  the connection then closes.
- Non-JSON or non-UTF-8 line: `-32700`, id null, the conversation continues.
- Valid JSON that is not an object carrying a string `method` and an object `params`: `-32600`,
  id echoed if present else null, the conversation continues.
- Unknown method: `-32601`, id echoed verbatim, absent id echoed as null.
- `-32602` and `-32603` are not implemented: no verb here takes params to validate and no verb can
  fail internally, so either would be a branch no test reaches. A panic in the daemon unwinds
  `main`, which drops the `BrokerSet` and reaps.

### The `membrane.status` reply

Mapped one-to-one from `SetStatus`; empty fields omitted per section 1.

| `SetStatus` | result |
| --- | --- |
| `Ready` | `{"ready":true,"state":"serving"}` |
| `NotReady{broker,reason}` | `{"ready":false,"state":"not_serving","broker":"<name>","reason":"unreachable"\|"timed_out"\|"malformed"\|"reported"}` plus `"broker_state":"<s>"` only when the reason is `Reported` |
| `DeadlineSpent{unreached,asked}` | `{"ready":false,"state":"deadline_spent","unreached":"<name>","asked":[…]}` |
| `Empty` | `{"ready":false,"state":"empty"}` |

Every `status` request calls `set.status(deadline)` fresh, never cached.

### Lifecycle

In order inside `daemon::run(config, shutdown: &AtomicBool) -> Result<(), DaemonError>`:

1. Bind a `UnixListener` at `control_socket`, wrapped in a `BoundSocket` guard whose `Drop`
   best-effort-removes the path. Bind failure is `DaemonError::Bind`, raised before any fork, so a
   refused start leaves no children. The daemon never unlinks a pre-existing path.
2. Build one `ExecCommand` per broker, so every allocation happens in the parent. Failure is
   `DaemonError::BrokerCommand`; the guard removes the socket.
3. `BrokerSet::spawn(specs, launcher, ControlSocketProbe)`, the launcher taking each pre-built
   `ExecCommand` out of a map by `spec.name`. Spawn failure is `DaemonError::Spawn`; `BrokerSet`
   has already killed and reaped the part-spawned brokers, and the guard removes the socket.
4. `control::serve(listener, shutdown, || set.status(deadline))`: non-blocking accept polled
   against the flag, and a short read timeout per connection with the flag checked between
   timeouts, so an idle client never holds the daemon past shutdown.
5. Loop exit drops the set, which kills and reaps every broker, then drops the guard, which
   removes the socket.

`main.rs`: exactly one argv, the config path, else usage on stderr and exit 2; an unreadable or
invalid config is exit 2; SIGTERM and SIGINT `sigaction` handlers store `true` into a static
`AtomicBool`; a `DaemonError` is exit 1; a clean shutdown is exit 0. SIGKILL is the case the
drop guarantee cannot cover, documented in `run`'s contract rather than solved.

### `ExecCommand` (`src/exec.rs`)

`new(argv: Vec<String>) -> Result<ExecCommand, ExecError>`. In the parent, before any fork: refuse
empty argv and interior NUL; an argv[0] holding `/` is used as-is but must exist; otherwise it is
resolved by walking `PATH`; the environment is snapshotted; every `CString` and both
null-terminated pointer arrays are built into owned fields. `impl Launch`'s child body is exactly
`execve` then `_exit(127)` — no allocation and no panic path, satisfying the crate's post-fork
rule by construction.

### `ControlSocketProbe` (`readiness.rs`)

A unit struct implementing `brokers::Probe` by delegating to `readiness::probe`.

### Commits

1. `docs(tasks)` — this file.
2. `feat(membrane)` — the exec launcher resolved before the fork.
3. `feat(membrane)` — the production probe over a broker control socket.
4. `feat(membrane)` — the ndjson control loop answering `membrane.status`.
5. `feat(membrane)` — daemon lifecycle from config to reaped shutdown.
6. `feat(membrane)` — `membraned` driving the daemon from a config path.
7. `docs(spec)` — record the delivered not-ready reply of `membrane.status` in 001 section 4.
8. `docs(tasks)` — this task to `in_review`.

Each feature commit writes its tests and a named stub first, runs them, and pastes the verbatim
runtime failure into the commit body before the implementation is written.

### Test table

| Test (module) | Pins |
| --- | --- |
| `an_exec_command_runs_the_program_and_its_exit_code_comes_back` (exec) | `["sh","-c","exit 7"]` gives `Exited{code:7}` |
| `argv_carries_every_argument_in_order` (exec) | `["sh","-c","exit $1","x","9"]` gives `Exited{9}` |
| `the_childs_environment_is_the_parents_snapshot` (exec) | `["sh","-c","test -n \"$PATH\""]` gives `Exited{0}` |
| `arguments_the_exec_cannot_carry_are_refused` (exec) | empty argv, interior NUL, absent program — each refused before any fork |
| `a_program_that_cannot_be_executed_exits_the_child_with_127` (exec) | a non-executable file reached by path: construction passes, the child `_exit(127)`s |
| `the_production_probe_asks_the_broker_socket_and_relays_its_answer` (readiness) | a serving socket gives `Ready`, a silent one `TimedOut` |
| `a_status_request_is_answered_with_the_sets_readiness` (control) | id echoed verbatim, result ready/serving, no `error` key |
| `each_set_status_maps_to_its_wire_shape` (control) | the whole table above, asserting omitted keys absent |
| `a_line_that_is_not_json_is_answered_32700` (control) | code -32700 under a null id |
| `an_envelope_that_is_not_a_request_is_answered_32600` (control) | non-object JSON, missing `method`, missing `params`, non-object `params` |
| `an_unknown_method_is_answered_32601` (control) | `membrane.nope` gives -32601 with the id echoed |
| `a_connection_carries_many_requests_in_order_and_survives_a_parse_error` (control) | a bad line then a good one, in order, then a second connection served |
| `a_request_line_at_the_cap_is_answered_and_one_byte_over_is_refused_and_closed` (control) | 1,048,576 bytes answered, 1,048,577 refused and closed |
| `shutdown_stops_serve_even_with_an_idle_connection_open` (control) | the flag ends `serve` within a bounded join |
| `a_full_config_parses_and_each_malformed_config_is_refused_by_name` (daemon) | every refusal above, each naming its offender |
| `the_daemon_answers_status_with_its_brokers_readiness` (daemon) | ready then not-ready across two calls, then reap and socket removal |
| `a_daemon_with_no_brokers_answers_empty_and_never_ready` (daemon) | `brokers: []` gives `state: "empty"` |
| `an_existing_control_socket_path_is_refused_before_any_fork` (daemon) | `Err(Bind)` naming the path, and no broker pidfile |
| `a_broker_command_that_cannot_resolve_refuses_startup_and_removes_the_socket` (daemon) | `Err(BrokerCommand)` naming broker and program, socket gone |
| `a_spawn_refusal_reaps_the_spawned_and_removes_the_socket` (daemon) | two brokers sharing one socket: `Err(Spawn)`, first pid `ECHILD`, socket gone |
| `membraned_serves_ready_and_dies_cleanly_on_sigterm` (E2E) | the wire answer, exit 0, socket removed, broker pid `ESRCH` |
| `membraned_reports_the_broker_that_is_not_serving` (E2E) | `reason: "reported"` and `broker_state: "starting"` |
| `membraned_exits_nonzero_naming_the_failure` (E2E) | no args gives 2, bad config gives 2, an occupied socket gives 1 |

STOP when done — do not start the next piece of work.

## Notes

### 2026-09-01

The plan asked `a_spawn_refusal_reaps_the_spawned_and_removes_the_socket` to read the first
broker's pid from a pidfile and assert `ECHILD`. That pid is never recorded: `BrokerSet::spawn`
does nothing between forking the first broker and refusing the second, so the SIGKILL lands
before the child finishes exec'ing `sh`. Measured at 0 of 5 checkpoints out to 1000ms, three runs
running. The test proves the same property differently — the pidfile must be **absent** after a
settle — and that is falsifiable because a broker left running records its pid within 50ms, which
was measured the same way. What it does not observe is reaped-versus-zombie; a zombie is not a
broker left running, and the reap itself is observed by `ECHILD` in
`the_daemon_answers_status_with_its_brokers_readiness`, where the broker lives long enough to
record its pid.

The end-to-end SIGTERM test checks the broker is gone with `kill(pid, 0)` returning `ESRCH`. The
broker is `membraned`'s child, not the test's, so `waitpid` is not available to it. That carries
a pid-reuse window of about a millisecond: if the pid were reused between the reap and the check,
the test would read a live unrelated process and fail rather than pass, so the risk is a flake
and not a false green. Five consecutive runs were clean.

`the_childs_environment_is_the_parents_snapshot` cannot be made red by a stub that exits the
child 0, because 0 is what it asserts. The red phase used a stub exiting 3 so that every exec
test fails on its assertion rather than one of them passing vacuously.

2026-09-01 — GitHub reports PR 70 merged at
`b0d24e47177c898cb38a218abaa28169cb827912`; the heartbeat reconciled the stale task record.
