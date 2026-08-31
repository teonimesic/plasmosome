---
id: 014
title: The controller answers its control socket
status: in_progress
priority: 1
specs: [001]
intents: []
refs:
  [
    docs/specs/001-control-protocol.md,
    AGENTS.md,
    crates/plasmosome-core/AGENTS.md,
    crates/plasmosome-core/src/lib.rs,
    crates/plasmosome-core/src/state.rs,
    crates/plasmosome-freeze-checks/tests/freeze_rules.rs,
    crates/plasmosome-membrane/src/readiness.rs,
  ]
done_when: >-
  plasmosome-core serves an ndjson connection: a line that is not JSON gets
  -32700 with a null id; a JSON line that is not the spec 001 §1 envelope gets
  -32600; an unserved method gets -32601; plasmosome.status params that do not
  parse get -32602; every reply echoes the request id verbatim, in request
  order. plasmosome.status answers the §3.3 shape built from a ControllerState,
  proven over a real Unix socket in a test; a status request naming an instance
  this controller is not gets code 101 with a target field. Every application
  error code 100-110 has a typed constructor whose serialization carries
  exactly the structured fields in §1's table, and an unknown code does not
  deserialize. The freeze checks cover the new wire module and its types.
pr:
evidence:
---

## Why

The control protocol is frozen on paper and exists nowhere in code. There is no envelope type,
no error table, and no way for any client to ask a controller anything. Spec 001 §6 item 1 —
every verb round-trips against the real controller — is open precisely because of this gap, and
the next P1 step (four-plasmid end-to-end) needs a socket that answers before it can exist.

Without this, every later verb re-decides the envelope, the error shapes, and the dispatch
rules, and they drift. Freezing them in code once, behind a passing test, is the smallest piece
of the controller daemon that is worth shipping on its own.

## Plan

**Deliverable:** two new modules in `plasmosome-core` — `protocol.rs`, the spec 001 §1 envelope,
the closed error table, and the `plasmosome.status` shapes as serde wire types; and `control.rs`,
a per-connection ndjson loop that maps protocol failures to the JSON-RPC reserve codes and
dispatches to a handler, with a production handler serving `plasmosome.status` from a
`ControllerState`.

**Out of scope, deliberately.** The `plasmosomed` binary, the
`~/.plasmosome/instances/<name>/` directory layout, socket binding and the accept loop, and
daemon lifecycle — they need directory-layout decisions and add nothing to the wire contract.
Every other verb: each one drags in D2b genome resolution, drains and the ledger, or the
membrane, and none fits the same afternoon. `cell.status` in particular must relay the
membrane's own answer verbatim (§3.6), so it waits for the controller-side membrane client;
that client (§4) is its own unit. Concurrency: the loop serves one connection; who accepts and
how many at once is the daemon's decision, not this one's. An unimplemented frozen verb answers
`-32601` — honest until its unit lands.

**Where the code lives, and why.** `plasmosome-core` is already a controller crate under the
freeze checks, so 86 §4 rules 1-3 apply to this code with no new wiring. No new crate, no
workspace or CI change. The loop uses only `std` — no new dependency of any kind. Read
`crates/plasmosome-membrane/src/readiness.rs` for the house pattern: it is the client half of
this same conversation, and its tests show how a socket conversation is tested.

**The parsing ladder.** For each line read:

1. Not parseable as JSON → `-32700`, id `null`.
2. JSON, but not the envelope — not an object, no `id`, `method` not a string, or `params`
   absent or not an object → `-32600`, echoing `id` when the object had one, else `null`.
   `params` is never optional (§1); a missing `params` is an invalid request, not an empty one.
3. Envelope, but a method the handler does not serve → `-32601`, id echoed.
4. Served method, but params that do not deserialize → `-32602`, id echoed.

The loop replies and keeps reading after every failure — framing is per line, so one bad line
poisons nothing. EOF ends the loop. A write error returns it.

**The wire types (`protocol.rs`).** All serde, no locks, no `Arc`, no shared memory of any
kind. Warning: the freeze check scans this file as raw text for the shared-memory patterns
listed in `freeze_rules.rs` — none of those words may appear anywhere in the file, tests and
doc text included.

```rust
pub struct Request {
    pub id: Value,
    pub method: String,
    pub params: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    Success { id: Value, result: Value },
    Failure { id: Value, error: WireError },
}
```

`ErrorCode` is a C-like enum over the full closed set — the four reserve codes and 100-110 —
with hand-written `Serialize`/`Deserialize` as its integer. Deserializing an integer outside
the set fails: the table is closed, and a client that invents a code must be caught, not
absorbed.

`WireError` is one flat struct: `code: ErrorCode`, `message: String`, and one
`Option` per structured field named in §1's table (`candidates`, `target`, `capability`,
`plasmid`, `node`, `modes: Option<Vec<MockMode>>`, `plasmids`, `resolutions`, `from`, `to`,
`handle`, `deadline_ms: Option<u64>`, `detail`, `path`, `verb`), each with
`skip_serializing_if`. Fields are private; a constructor per code is the only way to build one,
so a code cannot ship without its fields:

```rust
impl WireError {
    pub fn ambiguous_target(candidates: Vec<String>) -> WireError
    pub fn unknown_target(target: String) -> WireError
    pub fn already_exists(target: String) -> WireError
    pub fn unresolved_requirement(capability: String, plasmid: String) -> WireError
    pub fn mock_mode_conflict(node: String, modes: Vec<MockMode>, plasmids: Vec<String>, resolutions: Vec<String>) -> WireError
    pub fn illegal_state(from: String, to: String) -> WireError
    pub fn drain_timeout(handle: String, deadline_ms: u64) -> WireError
    pub fn not_running(target: String) -> WireError
    pub fn manifest_invalid(detail: String, path: String) -> WireError
    pub fn widening_forbidden(plasmid: String) -> WireError
    pub fn attestation_required(verb: String) -> WireError
    pub fn parse_error() -> WireError
    pub fn invalid_request(message: String) -> WireError
    pub fn method_not_found(method: &str) -> WireError
    pub fn invalid_params(message: String) -> WireError
}
```

Each constructor writes its own human `message` from its fields; tests assert code and fields,
never message text — the message is not the contract (§1).

The status shapes, mirroring §3.3 exactly:

```rust
#[serde(rename_all = "lowercase")]
pub enum InstanceState { Running, Stopped, Unreachable }

pub struct StatusResult {
    pub name: String,
    pub state: InstanceState,
    pub ready: bool,
    pub controller: ControllerInfo,
    pub cells: Vec<CellStatusEntry>,
}

pub struct ControllerInfo {
    pub uptime_ms: u64,
    pub ledger_generation: u64,
}

pub struct CellStatusEntry {
    pub id: CellId,
    pub genome: Option<GenomeName>,
    pub state: CellStatus,
    pub plasmids: Vec<String>,
}

#[serde(deny_unknown_fields)]
pub struct StatusParams {
    pub name: Option<String>,
}
```

`plasmids` entries are the D2 labels — build them with the existing
`PlasmidRecord::list_label()`, never a second format string.

**The seam (`control.rs`).** The loop must be testable without a socket and the handler without
the loop:

```rust
pub trait Handler {
    fn handle(&mut self, method: &str, params: &Map<String, Value>) -> Result<Value, WireError>;
}

pub fn serve_connection<R: BufRead, W: Write, H: Handler>(
    reader: R,
    writer: W,
    handler: &mut H,
) -> std::io::Result<()>
```

Two adapters exist the moment the tests land — the production `Controller` and the test fakes —
so this is a real seam. The loop owns steps 1-3 of the ladder; the handler owns step 4 and the
verbs. Flush after every line.

The production handler:

```rust
pub struct Controller {
    name: InstanceName,
    state: ControllerState,
    started: Instant,
    ledger_generation: u64,
}
```

It serves exactly one method, `plasmosome.status`. Params `name` absent or equal to its own name
→ a `StatusResult`: `state: Running`, `ready: true` (this process answered; that is what running
means here), `uptime_ms` from `started`, cells and labels from the matching `InstanceRecord` in
`state` (no matching record means no cells). Params naming any other instance →
`WireError::unknown_target` with target `plasmosome <name>` — this controller resolves, it never
guesses (§2). Every other method → `method_not_found`. `ledger_generation` is a constructor
argument: nothing in the crate exposes a live generation yet, and inventing one here would be a
lie; the daemon unit wires it to the real ledger.

**Freeze-check extension.** In `crates/plasmosome-freeze-checks/tests/freeze_rules.rs`: add
`crates/plasmosome-core/src/protocol.rs` to the `wire_sources` list, and add `Request`,
`Response`, `WireError`, `ErrorCode`, `InstanceState`, `StatusResult`, `ControllerInfo`,
`CellStatusEntry`, and `StatusParams` to `every_seam_wire_type_is_serde_in_both_directions`.
`control.rs` stays off the wire list: it is host-local machinery, not state that crosses the
seam.

**Exports.** `lib.rs` gains `pub mod control;` and `pub mod protocol;` plus re-exports of
`Controller`, `Handler`, `serve_connection`, `Request`, `Response`, `WireError`, `ErrorCode`,
and `StatusResult`. Extend the crate's `//!` block by one sentence, no more.

**Order of work.** Write the types and stubs first — `serve_connection` reading nothing,
`Controller` refusing everything — then the tests, and run them: every `control.rs` test must be
seen failing against the stubs before the loop is written. The `protocol.rs` shape tests cannot
fail before their types exist; their honesty comes from the mutation table below instead.

**Test table.** Every test failure names the line, the code, or the field that was wrong.

| Test | Proves |
| --- | --- |
| `every_application_error_serializes_its_code_and_structured_fields` | all 11 constructors emit their §1 code and exactly their table fields |
| `an_unknown_error_code_does_not_deserialize` | the code set is closed |
| `a_response_carries_result_or_error_never_both` | both envelope halves serialize to the §1 shapes and round-trip |
| `the_status_result_serializes_the_frozen_shape` | §3.3 keys, nested `controller`, and D2 labels on the wire |
| `a_line_that_is_not_json_gets_parse_error_with_null_id` | ladder step 1 |
| `a_json_line_that_is_not_the_envelope_is_invalid_request` | ladder step 2, one case each: no id, no method, params missing, params not an object |
| `an_unknown_method_is_method_not_found` | ladder step 3 |
| `status_params_that_do_not_parse_are_invalid_params` | ladder step 4 is distinct from step 3 |
| `every_reply_echoes_the_request_id_verbatim` | a string id and an object id both come back untouched |
| `replies_come_back_in_request_order` | §1 ordering, over a mix of good and bad lines |
| `the_loop_survives_a_bad_line_and_keeps_serving` | one bad line does not end the conversation |
| `status_reports_the_instance_its_cells_and_their_mock_labels` | the production handler builds §3.3 from a `ControllerState` |
| `status_for_a_name_this_controller_is_not_is_unknown_target` | code 101 with `target`, never a guessed answer |
| `a_real_socket_conversation_answers_line_per_line_and_ends_at_eof` | the whole stack over a Unix socket in a `TempDir`; the serving thread joins after the client hangs up, so EOF provably ends the loop |

The loop tests drive `serve_connection` with in-memory readers and writers; only the last test
binds a socket.

**Mutations to watch.** Four tests in this repo have passed against the very bug they named. For
each guard below: apply the mutation, run the named test, see it fail, record the failing output
in `## Notes`, revert.

| Guarded test | Mutation that must make it fail |
| --- | --- |
| `every_application_error_serializes_its_code_and_structured_fields` | swap the integers for codes 100 and 101 in the `ErrorCode` serializer |
| `an_unknown_error_code_does_not_deserialize` | make the deserializer map unknown integers to `InvalidRequest` |
| `every_reply_echoes_the_request_id_verbatim` | hardcode id `0` in every reply |
| `replies_come_back_in_request_order` | collect all replies and write them reversed |
| `status_params_that_do_not_parse_are_invalid_params` | map a params failure to `method_not_found` |
| `a_json_line_that_is_not_the_envelope_is_invalid_request` | default a missing `params` to an empty object |
| `status_for_a_name_this_controller_is_not_is_unknown_target` | ignore the `name` param and always answer own status |
| the extended freeze check | add one line to `protocol.rs` aliasing a std lock type; the wire-source scan must fail; remove the line |

**Definition of done:** every line of `done_when`, the fourteen tests above green, the eight
mutations each observed failing and recorded, and the gate in root `AGENTS.md` (`cargo test
--workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --
--check`, `./.githooks/provenance-guard`). `plasmosome-membrane` is untouched.

Read only the files in `refs:`. Do not explore beyond them.

STOP when done. Do not start the daemon, another verb, or the membrane client.

## Notes

### Stubs first: every `control.rs` test seen failing before the loop existed

With `serve_connection` returning `Ok(())` without reading, and `Controller::handle`
refusing every method, `cargo test -p plasmosome-core` reported:

```
test result: FAILED. 63 passed; 10 failed; 0 ignored; 0 measured; 0 filtered out
failures:
    control::tests::a_json_line_that_is_not_the_envelope_is_invalid_request
    control::tests::a_line_that_is_not_json_gets_parse_error_with_null_id
    control::tests::a_real_socket_conversation_answers_line_per_line_and_ends_at_eof
    control::tests::an_unknown_method_is_method_not_found
    control::tests::every_reply_echoes_the_request_id_verbatim
    control::tests::replies_come_back_in_request_order
    control::tests::status_for_a_name_this_controller_is_not_is_unknown_target
    control::tests::status_params_that_do_not_parse_are_invalid_params
    control::tests::status_reports_the_instance_its_cells_and_their_mock_labels
    control::tests::the_loop_survives_a_bad_line_and_keeps_serving
```

The 63 that passed are the pre-existing suite plus the four `protocol.rs` shape tests,
which cannot fail before their types exist. All 73 pass after the loop and the handler
landed.

### The eight mutations, each applied, run, observed, and reverted

**1. Swap the integers for codes 100 and 101 in the `ErrorCode` serializer** —
`every_application_error_serializes_its_code_and_structured_fields`:

```
the code on the wire for {"candidates":["cell-1","cell-2"],"code":101,
"message":"the target is ambiguous: 2 candidates match"}
  left: Some(Number(101))
 right: Some(Number(100))
```

**2. Map unknown integers to `InvalidRequest` in the deserializer** —
`an_unknown_error_code_does_not_deserialize`:

```
code 111 is outside the closed table and must not deserialize, got Ok(InvalidRequest)
```

**3. Hardcode id `0` in every reply** — `every_reply_echoes_the_request_id_verbatim`:

```
the ids that came back: [Object {"id": Number(0), "result": Object {}},
                         Object {"id": Number(0), "result": Object {}}]
  left: [Number(0), Number(0)]
 right: [String("abc"), Object {"trace": String("x-9")}]
```

**4. Collect all replies and write them reversed** — `replies_come_back_in_request_order`:

```
replies arrive in request order: [ ... id 4, id 3, id Null, id 1 ... ]
  left: [Number(4), Number(3), Null, Number(1)]
 right: [Number(1), Null, Number(3), Number(4)]
```

**5. Map a params failure to `method_not_found`** —
`status_params_that_do_not_parse_are_invalid_params`:

```
a served method with params that do not parse is not a missing method:
{"error":{"code":-32601,"message":"`plasmosome.status` is not a method this controller
serves"},"id":1}
  left: -32601
 right: -32602
```

**6. Default a missing `params` to an empty object** (`#[serde(default)]` on
`Request::params`) — `a_json_line_that_is_not_the_envelope_is_invalid_request`:

```
reply {"id":2,"result":{}} carries no error code
```

**7. Ignore the `name` param and always answer own status** —
`status_for_a_name_this_controller_is_not_is_unknown_target`:

```
reply {"id":3,"result":{"cells":[...],"controller":{"ledger_generation":4,"uptime_ms":1},
"name":"work","ready":true,"state":"running"}} carries no error code
```

**8. Alias a std lock type at the top of `protocol.rs`**
(`type ControllerGuard = std::sync::Mutex<u8>;`) —
`controller_wire_state_shares_no_memory_across_the_seam`:

```
86 §4 rule 2 broken: `crates/plasmosome-core/src/protocol.rs` uses `Mutex`;
controller⇄supervisor state moves only as serde types, never as shared memory
```

### Two places the plan met the compiler

**Clippy rejects the plan's type shapes by default.** `WireError` is one flat struct with
an `Option` per field in the §1 table, which makes it about 384 bytes. That trips
`clippy::large_enum_variant` on `Response` (the `Failure` variant dwarfs `Success`) and
`clippy::result_large_err` on `Handler::handle`. Boxing would silence both but would change
two signatures the plan wrote out and force `Box::new` at every construction site, so both
lints carry a narrow `#[expect(..., reason = ...)]` instead and every signature is the one
the plan specified. Nothing about the wire changes: `code`, `message`, and the structured
fields serialize the same either way.

**Who owns ladder step 3.** The plan says the loop owns steps 1-3 and the handler owns step
4, but the `Handler` signature it gives has no way for the loop to know which methods a
handler serves. The concrete instruction two paragraphs later — "Every other method →
`method_not_found`" on `Controller` — is what got built: the loop owns steps 1 and 2, the
handler answers -32601 and -32602. The behaviour in `done_when` is unchanged.

**One deliberate hole worth naming for the next reader.** `WireError`'s private fields stop
anyone *constructing* a code without its fields. Its derived `Deserialize` — required by the
freeze check, which holds every wire type to serde in both directions — will still accept a
JSON object carrying a known code and no fields. That is the reading path, for a client
parsing whatever a server sent; the producing path has one constructor per code and no
other door.
