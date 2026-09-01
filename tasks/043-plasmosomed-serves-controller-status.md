---
id: 043
title: plasmosomed answers plasmosome.status from its cell registry
status: in_review
priority: 2
specs: [001]
intents: [003, 004, 009, 012]
refs: [crates/plasmosome-core/src/lib.rs, crates/plasmosome-core/src/control.rs,
  crates/plasmosome-core/src/protocol.rs, crates/plasmosome-core/src/state.rs,
  crates/plasmosome-core/AGENTS.md, crates/plasmosome-membrane/src/main.rs,
  crates/plasmosome-membrane/src/daemon.rs, crates/plasmosome-membrane/tests/membraned.rs,
  docs/specs/001-control-protocol.md, docs/specs/013-what-earns-a-guard.md,
  tasks/041-membraned-serves-broker-readiness.md, AGENTS.md]
done_when:
  - A plasmosome.status request on plasmosomed's control socket is answered with the
    configured instance name, state running, ledger_generation 0 and `cells: []`; a status
    naming any other instance is answered code 101, and mistyped params code -32602.
  - SIGTERM ends the process with exit 0 and the socket path removed; a start on an occupied
    socket path exits 1 naming the path and does not unlink it; no args, an unreadable
    config, and a malformed config each exit 2 naming the offender.
  - The section 1 envelope edges hold on the wire - -32700 continues (non-JSON and
    non-UTF-8 both), a line at the cap is served with its id echoed verbatim, one byte over
    is -32600 under a null id and the connection closes, and an unknown method is -32601.
  - The binary lives in crates/plasmosome-core as `plasmosomed`; crates/plasmosome, the
    crates.io name-hold, still has no binary target, no package ships a binary named after
    another package, and every plasmosome-guards test passes.
  - All five gate commands exit 0, reported as bare exit codes.
  - The chain walks - this task names spec 001 and intents 003, 004, 009, 012, reaches
    in_review with pr: set, and the pull request is a draft whose body ends with `task: 043`.
pr: 76
evidence:
---

## Why

Spec 001 calls the protocol the controller's (`plasmosomed`) only control surface, and section 6
item 1 records that no daemon answers it; `plasmosome-core` already implements the envelope and
the status verb with nothing serving them on a socket. This delivers the serving side, the way
task 041 delivered `membraned`.

## Plan

### Deliverable, in one sentence

A `plasmosomed` binary that reads a JSON config naming its control socket and instance name,
serves the spec 001 section 1 ndjson envelope on that socket through the existing
`plasmosome_core::control` machinery, answers `plasmosome.status` from an empty
`ControllerState`, and on SIGTERM/SIGINT tears down — socket path removed, exit 0.

### Out of scope

The reconciler, cell lifecycle, two-phase transactions, the ledger (`ledger_generation` is
hardcoded `0` at the composition point), any backend transport, attaching plasmids, the
`~/.plasmosome/instances/<name>/` path convention (it arrives with `plasmosome.start` and the
CLI; this daemon is config-path-driven exactly as `membraned` is, which also keeps tests
hermetic), and every other verb in spec 001 section 3.

### Files to read, and nothing else

Root `AGENTS.md`; `.agents/skills/tasks/SKILL.md` and `.agents/skills/pr-review/SKILL.md`;
`docs/specs/001-control-protocol.md` (sections 1 and 3.3 bind this work, section 6 item 1 is
edited by it); `crates/plasmosome-core/src/{lib,control,protocol,state}.rs`,
`crates/plasmosome-core/AGENTS.md`, `crates/plasmosome-core/Cargo.toml`;
`crates/plasmosome-membrane/src/{main,daemon,control}.rs`,
`crates/plasmosome-membrane/tests/membraned.rs`, `crates/plasmosome-membrane/Cargo.toml` — the
template being mirrored; `tasks/041-membraned-serves-broker-readiness.md`,
`docs/templates/task.md`; `docs/specs/013-what-earns-a-guard.md`. Do not explore beyond these.

### Where the binary lives

`crates/plasmosome-core`, as `[[bin]] name = "plasmosomed"` alongside an explicit `[lib]`,
mirroring `crates/plasmosome-membrane/Cargo.toml`.

`crates/plasmosome` is refused: it is the crates.io name-hold at version `0.0.0` with
`publish = ["crates-io"]`, and task 031 pins it as a member with a library target and no binary
target. A binary there would ship to the public registry on the next republish. The same
reasoning refuses `crates/plasmid`.

A new crate is refused: no guard forces the split, and it would separate `plasmosomed` from the
`Controller`, `serve_connection` and protocol types it is a thin shell around.

`crates/plasmosome-core` is chosen: spec 001 names `plasmosomed` as the controller's binary, and
`lib.rs`'s crate doc already claims the crate answers the control protocol on an ndjson
connection. The guards pass — `plasmosomed` matches no package name and no other binary, core
stays `publish = false`, and the testkit rule is untouched.

Signal handling needs `libc = { workspace = true }` in core's `[dependencies]`, as `membraned`
does it. The freeze rule that once refused this was removed by spec 013; the surviving prose rule
in `crates/plasmosome-core/AGENTS.md` bans virtualization dependencies only. `Cargo.lock` gains a
`libc` edge under `plasmosome-core` and is committed — the guards run `cargo metadata --locked`.

### Reused, with zero edits

| Item | Role |
| --- | --- |
| `control::serve_connection(reader, writer, handler)` | the whole section 1 envelope per connection: `-32700` on non-JSON and non-UTF-8 (conversation continues), `-32600` on a non-envelope (continues) and on an over-cap line (then closes), `-32601`/`-32602` routing, `-32603` on a loop-owned code or handler panic, CRLF handling, verbatim id echo, reply-per-line in order |
| `control::Controller` | the `plasmosome.status` handler: name check (`unknown_target` 101), `StatusParams` validation (`-32602`), cells built from `ControllerState` |
| `control::MAX_LINE_BYTES`, `control::Handler` | the cap and the handler seam |
| `protocol::*` | wire types, closed error table, omit-empty serialization |
| `state::{InstanceName, ControllerState}` | validated instance name; `ControllerState::default()` is the empty cell registry |

### Written new, all inside `plasmosome-core`

`src/daemon.rs`, `src/main.rs`, `tests/plasmosomed.rs`:

- `DaemonConfig { control_socket: PathBuf, name: InstanceName }` — the validated name in the type
  makes an unvalidated one unrepresentable.
- `parse_config(text) -> Result<DaemonConfig, ConfigError>` — a serde-derive
  `#[serde(deny_unknown_fields)]` raw struct, then `InstanceName::parse`. `ConfigError` is
  `NotConfig(serde_json::Error)` or `NotAnInstanceName(InstanceNameError)`, each naming the
  offender. `membraned` hand-rolled its parse because the membrane crate has no serde-derive
  dependency; core has it, so this is the same contract in the crate's native tools.
- `DaemonError { Bind { path, source }, Listener(std::io::Error) }`.
- `run(config, shutdown: &AtomicBool) -> Result<(), DaemonError>` — bind first; a `BoundSocket`
  drop guard removes the path on every return and never unlinks a path it did not create; build
  `Controller::new(config.name, ControllerState::default(), 0)`; then a non-blocking accept loop,
  `WouldBlock` polling at 25 ms against the flag, `Interrupted` retried, anything else a
  `Listener` failure. Connections are served sequentially.
- `FlaggedReads` — a private `io::Read` over the stream and the flag, so core's blocking
  `serve_connection` can be reused verbatim under a shutdown flag.
- `src/main.rs` — `membraned`'s `main.rs` with the names swapped.
- `src/lib.rs` — `pub mod daemon;`, the re-exports, and one sentence in the `//!` naming the
  binary.

Handler panics are not caught at the daemon layer: `serve_connection` already answers `-32603`
before resuming the unwind, and the resumed panic unwinds `run`, whose `BoundSocket` still
removes the path. Documented in `run`'s contract alongside the SIGKILL caveat.

### Config and wire behavior

```json
{"control_socket": "/path/to/control.uds", "name": "work"}
```

Both keys required, unknown keys refused. `plasmosome.status` against an empty registry answers
`{"name": "work", "state": "running", "ready": true, "controller": {"uptime_ms": …,
"ledger_generation": 0}, "cells": []}`.

### Test table

Unit tests in `src/daemon.rs` (T1–T8); end-to-end in `tests/plasmosomed.rs` via
`env!("CARGO_BIN_EXE_plasmosomed")` (T9–T11).

| # | Test | Pins | A wrong implementation it catches |
| --- | --- | --- | --- |
| T1 | `a_full_config_parses_and_each_malformed_config_is_refused_by_name` | full config parses; not JSON, missing or mistyped `control_socket`/`name`, unknown key, and the names `""`, `"a/b"`, `".."` each refused naming their offender | serde without `deny_unknown_fields`; a raw `String` name reaching the socket layer |
| T2 | `the_daemon_answers_status_with_an_empty_cell_registry` | the status reply's name, `running`, `ready`, `ledger_generation: 0`, `cells: []`, no `error`; then the flag stops `run` with `Ok` and the socket path is gone | fabricated cells; the config name not wired through; a shutdown reported as an error; a socket left behind |
| T3 | `a_status_for_a_name_this_daemon_is_not_is_refused_on_the_wire` | `{"name":"elsewhere"}` gives code 101 with target `plasmosome elsewhere`; `{"name": 7}` gives -32602 | a canned handler answering a hardcoded status to anything |
| T4 | `an_existing_control_socket_path_is_refused_and_the_path_is_not_unlinked` | `Err(Bind)` naming the path, the file still there | stale-socket cleanup, which steals a live daemon's socket |
| T5 | `shutdown_stops_serve_even_with_an_idle_connection_open` | one answered request, connection idle, flag set, `run` returns `Ok` | a blocking read with no timeout and no flag poll |
| T6 | `shutdown_stops_serve_even_with_a_client_that_never_reads_its_replies` | 20 000 unread requests, flag set, `run` returns `Ok` | a missing write timeout wedging the reply |
| T7 | `a_connection_survives_a_parse_error_and_a_second_connection_is_served` | bad line answered -32700, a status then served on the same connection, a second connection served after it closes | a parse error treated as connection-fatal; an accept loop serving one connection |
| T8 | `an_accept_error_is_classified_by_what_it_means_for_the_loop` | `WouldBlock` polls, `Interrupted` retries, anything else fails | `Interrupted` killing the daemon on any signal |
| T9 | `the_envelope_edges_hold_on_the_wire` | non-JSON then a served status on one connection; a `0xFF` byte answered -32700 and continuing; a line at the cap served with its id echoed; one byte over answered -32600 under a null id and the connection closed; `plasmosome.nope` answered -32601 | an off-by-one on the cap; a hand-rolled envelope that drifts |
| T10 | `plasmosomed_serves_status_and_dies_cleanly_on_sigterm` | the wire answer, exit 0 on SIGTERM, socket path removed | no `sigaction` installed; teardown that only works in-process |
| T11 | `plasmosomed_exits_nonzero_naming_the_failure` | no args gives 2 with usage; absent config 2 naming the file; malformed config 2; occupied socket 1 naming the path | refusals swallowed into exit 0; a bind failure conflated with a config failure |

### Commits

1. `docs(tasks)` — this file.
2. `feat(core)` — the `plasmosomed` config, refused by name.
3. `feat(core)` — the control socket serve loop from bind to reaped shutdown; `Cargo.toml` and
   `Cargo.lock` here.
4. `feat(core)` — `plasmosomed` driving the daemon from a config path.
5. `docs(spec)` — record in 001 section 6 that `plasmosomed` serves the envelope and
   `plasmosome.status`.
6. `docs(tasks)` — this task to `in_review`.

Each feature commit writes its tests and a named stub first, runs them, and pastes the verbatim
runtime failure into the commit body before the implementation is written.

No new guard: nothing here has a permanent or public consequence, so spec 013 forbids one. No
hardcoded paths in tests — every socket and config lives in a `tempfile::tempdir()`.

### The gate

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
./.githooks/provenance-guard
./.githooks/attribution-guard
```

STOP when done — do not start the next piece of work.

## Notes

### 2026-09-01

**The plan's fourth `done_when` was wrong about `crates/plasmid` and has been corrected.** It
asked that `crates/plasmosome` and `crates/plasmid` both still have no binary target.
`crates/plasmid` has carried `src/main.rs` — a binary target named `plasmid`, auto-discovered —
since task 031 (`6d6048c`), so that half of the line was already false on `main` and no
implementation could satisfy it. What is true and checkable is what the line now says:
`crates/plasmosome` alone is the name-hold with no binary, and the guard that matters here is
`no_binary_target_takes_a_name_another_package_owns`, which `plasmosomed` passes because it is
not the name of any package. The reasoning that refused siting the binary in a held crate is
unchanged.

**The manifest change moved one commit later than planned.** The plan put `[lib]`, `[[bin]]` and
`libc` in the serve-loop commit. `[[bin]] path = "src/main.rs"` does not build before that file
exists, and nothing used `libc` until the signal handlers, so both landed with `main.rs`.

**serde does not name a mistyped field, and the config test needs it to.** `{"name": 7}` yields
``invalid type: integer `7`, expected a string`` with no mention of `name`; missing and unknown
fields are named, mistyped ones are not. Each field is therefore read through a
`deserialize_with` that puts the field name back into the message, which keeps the refusal
naming its offender without hand-rolling the parse.

**Roughly forty lines are duplicated from `plasmosome-membrane` on purpose: do not extract them
yet.** The accept-loop skeleton, the `Next` classification, the 25 ms poll, the `BoundSocket`
guard, the three timeout constants and the `Running`/`addressable`/`ask` test scaffolding all
exist twice now. Extracting them needs either a new shared crate or a new dependency edge, and
the edge is the wrong one in both directions — `plasmosome-core` may never depend on
`plasmosome-membrane`, since a controller linking its supervisor inverts the crash-isolation
boundary both crates' notes state, and a membrane depending on core would serve nobody today.
The repo's own seam rule is not to abstract until something second exists as a real adapter, and
two daemons sharing forty stable lines is not that. What is **not** duplicated is the part that
would drift: the section 1 envelope has exactly one implementation (`control::serve_connection`)
and the status wire shape exactly one (`protocol::StatusResult`). **Extract when a third daemon
appears, or at the first divergence between the two accept loops** — whichever comes first.

**Three tests used to detect their mutation by hanging rather than by failing, and now do
not.** Independent review found it: with the bind wrongly succeeding on an occupied path, `run`
served forever and neither `an_existing_control_socket_path_is_refused_and_the_path_is_not_unlinked`
nor the occupied-socket arm of `plasmosomed_exits_nonzero_naming_the_failure` had any deadline to
fail against; and with the write timeout or the `FlaggedReads` shutdown check removed, the two
shutdown tests raised their assertion on time but `Running::drop` then joined a thread that could
never exit, turning a detected failure into a wedged process. Three fixes, all in the harness:
`refuses` runs a start expected to fail on its own thread and asserts the refusal arrives inside
`PATIENCE`; `Running::drop` waits for the daemon to signal before it joins, and leaks the thread
rather than blocking when it does not; and `output_within` gives the spawned binary a deadline,
killing it and failing rather than waiting on a process that should have exited. Re-measured
against the same three mutations: 10s, 10s and 20s to a reported failure, where all three
previously ran until killed. **The shape is inherited verbatim from `plasmosome-membrane`, whose
tests still have it** — fixing it there is not this unit of work, and it is the first thing the
extraction named above should carry.

**One idle client holds every other client out.** Connections are served sequentially, as
`membraned` serves them, so a client that connects and sends nothing occupies the daemon until
it disconnects or the daemon is asked to stop. It cannot hold the daemon past shutdown — that is
what `FlaggedReads` and the write timeout are tested for — but it can starve a second client.
Stated in `run`'s contract; concurrency is not part of this unit of work.
