---
id: 016
title: The control loop needs a daemon to own its edges
status: in_progress
priority: 2
specs: [001]
intents: []
refs:
  [
    crates/plasmosome-core/src/control.rs,
    crates/plasmosome-core/src/protocol.rs,
    crates/plasmosome-core/src/lib.rs,
    crates/plasmosome-freeze-checks/tests/freeze_rules.rs,
    docs/specs/001-control-protocol.md,
    docs/templates/decision.md,
    tasks/014-control-socket-answers.md,
  ]
done_when: >-
  each edge below is decided by the daemon unit rather than left to whichever
  connection hits it first: a non-UTF-8 byte, an unterminated line, a panicking
  handler, a handler returning a reserve code, and the absent-genome encoding.
  Each decision is tested, and the ones a client can observe are written into
  spec 001.
pr:
evidence:
---

## Why

`serve_connection` reads a line, answers it, and keeps going. That is the whole loop, and it is
the right amount for the wire contract. What it does not have is an owner for the edges around
the wire: who decides what happens when the bytes are not text, when a line never ends, when a
handler panics, when a handler answers with a code the loop reserves for itself. Those are all
questions about a process — how long it lives, how much it accepts, what it survives — and the
`plasmosomed` daemon unit is where a process gets decided. Deciding them here, one at a time and
per connection, is how they end up inconsistent.

None of these is a bug in what task 014 shipped. They are the questions its scope deliberately
left open, written down so the daemon unit inherits them as a list instead of rediscovering them.

## Plan

**Deliverable, in one sentence.** The connection loop in `crates/plasmosome-core/src/control.rs`
decides all five edges this task names — a non-UTF-8 byte, an unterminated line, a panicking
handler, a handler returning a loop-owned code, and the absent-genome encoding — with every
client-observable answer written into spec 001 §1 and the contested choices recorded in
`docs/decisions/`.

**The unit chosen, and why.** All five edges are properties of one conversation: what the reply
is when the bytes, the handler, or the encoding go wrong. None of them needs a bound socket, an
instance directory, or an accept loop to be decided. `serve_connection` is the one function
every future daemon connection will pass through, so deciding them there decides them once, and
the existing in-memory test seam covers them.

**Out of scope, deliberately.** The `plasmosomed` binary. Creating
`~/.plasmosome/instances/<name>/` and binding `control.uds`. The accept loop, and whether
connections are served one at a time or concurrently. Shutdown, signals, and what a daemon does
after a handler panic ends a connection (restart, log, refuse — its call). Idle and read
timeouts: a client that sends part of a line under the cap and then nothing still parks its
connection until it hangs up; only a daemon-level timeout can end that, and this task does not
add one. The daemon unit inherits that shorter list.

### The five decisions

Every design decision is made here. Execute exactly; if the plan contradicts reality, stop and
report rather than improvise.

**1. A non-UTF-8 line: the code changes to match the doc.** It is answered `-32700` under a
`null` id and the conversation continues. Three reasons. First, task 014's framing principle:
framing is per line, one bad line poisons nothing, and the client is owed an answer for every
line it framed — a silent close is indistinguishable from a crashed controller, and this
protocol's whole stance is machine-legible refusal. Second, JSON is UTF-8 by definition
(RFC 8259), so "the line is not JSON" — `parse_error()`'s exact message — is the truthful
answer with no new code. Third, the alternative (drop the connection on purpose) punishes a
one-byte corruption as hard as an attack. Never decode the bytes lossily and then try to parse:
replacement characters can produce a line that parses, and answering a request the client did
not send is worse than refusing one it did. The id is `null` — an id read out of bytes the loop
refused to trust is not an id.

**2. A line-length cap: 1 MiB, answered, then the connection closes.**

```rust
pub const MAX_LINE_BYTES: usize = 1 << 20;
```

in `control.rs`, with a `///` contract, re-exported from `lib.rs` next to `serve_connection`.
Every frozen v1 verb's params are names, small maps, and short argv lists — orders of magnitude
under 1 MiB — so no legitimate client comes near it, and a hostile one costs at most 1 MiB and
one reply per connection. A line that exceeds the cap without a newline is answered `-32600`
under a `null` id, with a message naming the cap, and then `serve_connection` returns `Ok(())`.
Closing rather than resynchronizing is the contested part; the decision record below carries the
rejected alternatives. Add a constructor so the message is built in one place:

```rust
impl WireError {
    pub fn line_too_long(cap: usize) -> WireError
}
```

Code `InvalidRequest`, message `the line exceeds {cap} bytes`, no structured fields.

**3. A panicking handler: the client is answered, then the panic resumes.** Wrap only the
`handler.handle(...)` call in `std::panic::catch_unwind(AssertUnwindSafe(...))`. On a caught
panic: reply `{"id": <the request's id>, "error": {"code": -32603, ...}}`, flush, then
`std::panic::resume_unwind(payload)`. The client gets a protocol error instead of a dead
socket; the connection then ends; the process above still sees the panic and owns what happens
next. Do not continue serving — a handler that panicked mid-`&mut self` may hold broken
invariants, and every later answer from it would be a guess. The `AssertUnwindSafe` is honest
for exactly that reason: the handler is never touched again after the catch. Never put the
panic payload's text on the wire — the message stays generic; the payload travels only through
`resume_unwind`, where the host process can log it.

This needs a code the table does not have. Add the fifth JSON-RPC reserve code:

```rust
ErrorCode::Internal
```

serialized as `-32603`, plus:

```rust
impl WireError {
    pub fn internal() -> WireError
}
```

message `the controller could not answer this request`, no structured fields. Spec §1 says the
error table is closed and additions are a contract change — this is that contract change, made
in spec 001 in the same PR (below). The existing test
`an_unknown_error_code_does_not_deserialize` lists `-32603` among the unknown codes; move it to
the known side (replace it with `-32604` in the unknown list, and assert `-32603` reads back as
`Internal`). Update that test list first, before touching `protocol.rs` — it passes both before
and after, since `-32604` is unknown either way; say so in `## Notes` rather than pretending it
failed.

**4. A handler returning a loop-owned code is answered `-32603` in its place.** The loop owns
`-32700` and `-32600`: a reply carrying either says something about framing, and only the loop
saw the frame. When `handler.handle` returns `Ok(Err(error))` and `error.code()` is
`ParseError` or `InvalidRequest`, replace the whole error with `WireError::internal()` and keep
serving — the handler returned normally, so its state is coherent; it is wrong about the
protocol, not broken. `-32601` and `-32602` stay handler-owned: the loop cannot know which
methods a handler serves. Do not reach for a type split (two error types, a changed `Handler`
signature, fifteen construction sites) — and it would not close the hole anyway, because the
freeze checks require `WireError` to deserialize in both directions, so a relayed error
(§3.6 relays the membrane's answers verbatim) can always carry any code. A runtime guard at the
one point where a handler's answer enters the wire is the whole fix. Extend the `Handler` trait
doc with the division: the loop owns `-32700` and `-32600`; a handler must never return them;
one that does is answered `-32603` in its place; a panic is answered `-32603` and ends the
connection.

**5. An absent genome omits the key.** On `CellStatusEntry::genome` in `protocol.rs`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub genome: Option<GenomeName>,
```

`WireError` already omits every absent optional field; one reply convention, not two. Spec §3.3
shows no `null` anywhere, and a client checking for the key gets one shape. Derived
deserialization already treats a missing key as `None`, and the `default` makes that explicit.
State the rule once in spec §1 (below) so every later verb inherits it.

### Mechanism

Rewrite the read side of `serve_connection` (signature unchanged except `mut reader: R`):

```rust
let mut buffer: Vec<u8> = Vec::new();
loop {
    buffer.clear();
    let taken = (&mut reader)
        .take(MAX_LINE_BYTES as u64 + 1)
        .read_until(b'\n', &mut buffer)?;
    if taken == 0 {
        return Ok(());
    }
    if !buffer.ends_with(b"\n") && buffer.len() > MAX_LINE_BYTES {
        write_reply(&mut writer, over_cap_reply())?;
        return Ok(());
    }
    strip one trailing b'\n', then one trailing b'\r';
    let Ok(line) = str::from_utf8(&buffer) else {
        write_reply(&mut writer, parse_error_reply())?;
        continue;
    };
    let answered = answer(line, handler);
    write_reply(&mut writer, answered.response)?;
    if let Some(payload) = answered.panic {
        std::panic::resume_unwind(payload);
    }
}
```

The arithmetic that must hold: the take limit is `MAX_LINE_BYTES + 1`, so a line of exactly
`MAX_LINE_BYTES` content bytes plus its newline fits and is served, and content of
`MAX_LINE_BYTES + 1` bytes is refused. A final line at EOF with no newline and under the cap is
served, exactly as `lines()` served it. The `\r` strip preserves `lines()` behaviour too.

`answer` keeps its parsing ladder untouched and returns a private struct instead of a bare
`Response`:

```rust
struct Answered {
    response: Response,
    panic: Option<Box<dyn std::any::Any + Send>>,
}
```

with the dispatch arm becoming:

```rust
match std::panic::catch_unwind(AssertUnwindSafe(|| {
    handler.handle(&request.method, &request.params)
})) {
    Ok(Ok(result)) => success,
    Ok(Err(error)) => failure with the guard from decision 4 applied,
    Err(payload) => failure carrying WireError::internal(), panic: Some(payload),
}
```

Rewrite `serve_connection`'s `///` block to the contract that is now true:

> A line that fails to parse — including one that is not UTF-8 — is answered and the
> conversation continues. A line longer than `MAX_LINE_BYTES` is answered `-32600` under a
> `null` id and the connection then closes. A handler panic is answered `-32603`, and the panic
> then resumes on this thread. Returns at end of input, or with the first read or write failure.

Two compiler caveats known in advance. `clippy::result_large_err` already carries an `#[expect]`
on `Handler::handle`; if the closure inside `catch_unwind` trips it too, widen the existing
`#[expect]`'s placement rather than adding an `allow`. And `catch_unwind` lives in `control.rs`,
which the freeze checks do not scan — but `protocol.rs` is scanned as raw text for the patterns
listed in `freeze_rules.rs`, so none of those words may appear anywhere in it, doc text and
tests included.

### Spec 001 edits (same PR)

In §1, extend the reserve-code sentence to name five codes:

> Protocol-level failures reuse the JSON-RPC reserve: `-32700` parse error, `-32600` invalid
> request, `-32601` method not found, `-32602` invalid params, `-32603` internal error — the
> controller, not the request, failed.

After the framing paragraph, add a short block titled **Connection edges**:

> - A request line is at most 1,048,576 bytes before its terminating newline. A longer line is
>   answered `-32600` under a `null` id, and the connection then closes.
> - A line that is not UTF-8 is not JSON: it is answered `-32700` under a `null` id, and the
>   conversation continues.
> - A request the controller fails on internally — a crash while answering — is answered
>   `-32603`, and the connection then closes. A reply carrying `-32700` or `-32600` always
>   comes from the connection loop itself, never from a verb implementation.
> - A response field with nothing in it is omitted, never sent as `null`. A cell with no genome
>   has no `genome` key (§3.3, §3.6).

### The decision record

Three of these choices have rejected alternatives someone will argue for again: the cap's
reply-then-close, the panic's answer-then-resume, and `-32603` entering the closed table.
Create one decision file for the set. Take the number from the remote per
`.agents/skills/tasks` (the `git ls-remote` loop, run for `docs/decisions/`); as of planning,
002 is the highest on any remote branch, but a live agent holds a worktree named
`decision-003`, so expect to take 004. Copy `docs/templates/decision.md` and fill it with:

- **Context.** The five edges from this task; the loop had no owner for them; the error table
  was closed with no code meaning "the controller failed".
- **Decision.** The five outcomes above, in one paragraph each at most: cap of 1 MiB answered
  then closed; panic answered `-32603` then resumed; loop-owned codes replaced with `-32603`;
  invalid UTF-8 answered `-32700` and served past; absent optional reply fields omitted.
- **Rejected.** Resynchronizing after an over-cap line by discarding to the next newline — it
  reads a hostile client's bytes without bound, and the pairing of later replies to later
  requests becomes a claim the loop cannot keep. Catching a panic and continuing to serve —
  every later answer comes from a handler whose invariants just failed. Documenting "handlers
  must not panic" and catching nothing — the client sees a silent close it cannot tell from a
  crash. A type split so a handler cannot express the loop-owned codes — two error types and a
  changed trait signature, and deserialized (relayed) errors still carry any code, so the hole
  survives the surgery. `"genome": null` — a second convention next to `WireError`'s
  omit-when-absent. Deferring all of this to the daemon binary — whichever connection hit an
  edge first would have decided it, which is this task's complaint.
- **Consequences.** `-32603` is a contract change to a closed table, made in spec 001. An
  unterminated under-cap line still parks its connection; idle timeouts and the
  one-connection-at-a-time question stay open for the daemon unit, undecided here on purpose.

### Order of work

Stubs and tests first, in the task-014 shape. Write the new tests against the current code and
run them: every test in the table below except the two marked *(passes before)* must be seen
failing before the implementation lands. Record the failing output in `## Notes`. The two
marked *(passes before)* pass against `lines()` by construction; their honesty comes from the
mutation table, not from an initial red. Then implement, then run the mutations.

Sequence the `-32603` test edit as its own first step (decision 3 above), before `protocol.rs`
changes.

### Test table

In-memory means driving `serve_connection` with a byte reader and a `Vec<u8>` writer, as every
existing loop test does. Only the panic edge earns a socket: reply-before-close ordering as the
client observes it, and the join-error shape, are transport facts an in-memory writer cannot
witness. Every other edge is a pure function of bytes in, bytes out.

| Test | Where | Proves |
| --- | --- | --- |
| `a_line_that_is_not_utf8_is_answered_parse_error_and_the_loop_keeps_serving` | control.rs, in-memory (byte script, not `&str`) | a `\xFF` line gets `-32700` with `null` id; the valid request after it is served |
| `a_line_past_the_cap_is_refused_and_the_connection_ends` | control.rs, in-memory | `MAX_LINE_BYTES + 1` bytes of `a` then a valid request: exactly one reply, `-32600`, `null` id; the loop returned `Ok` |
| `a_line_exactly_at_the_cap_is_served` *(passes before)* | control.rs, in-memory | a valid request padded with trailing spaces to exactly `MAX_LINE_BYTES` bytes is answered with a success; the cap has no off-by-one |
| `a_final_line_without_a_newline_is_still_answered` *(passes before)* | control.rs, in-memory | the `read_until` rewrite keeps `lines()`'s EOF behaviour |
| `a_panicking_handler_is_answered_internal_before_the_panic_resumes` | control.rs, in-memory, `catch_unwind` around the whole call | the panic reaches the test; the writer holds exactly one reply, `-32603`, the request's id; the line after the panic was never served |
| `a_panicking_handler_still_answers_the_client_over_a_real_socket` | control.rs, real UDS in a `TempDir` | the client reads a full `-32603` reply line and then EOF; `server.join()` is `Err` — the daemon above still sees the panic |
| `a_handler_returning_a_loop_owned_code_is_answered_internal_and_the_loop_keeps_serving` | control.rs, in-memory | a handler answering `parse_error()` is replaced with `-32603` under the request's id; the next request is served |
| `a_cell_with_no_genome_omits_the_key_in_both_directions` | protocol.rs | serializing `genome: None` emits no `genome` key; a status entry without the key deserializes to `None` |
| `an_unknown_error_code_does_not_deserialize` (edited) | protocol.rs | `-32603` reads back as `Internal`; `-32604` replaces it on the unknown side |

The panicking test handler panics on one method and echoes on another; the lying handler
returns `WireError::parse_error()` on one method. Both are a few lines next to `Echo`. The
socket test's server thread will print a panic backtrace to test output; do not install a panic
hook to hide it — the hook is process-global and tests run in parallel.

### Mutations to watch

Five tests in this repo have passed against the very bug they named. For each row: apply the
mutation, run the named test, see it fail, record the failing output in `## Notes`, revert.

| Guarded test | Mutation that must make it fail |
| --- | --- |
| `a_line_that_is_not_utf8_is_answered_parse_error_and_the_loop_keeps_serving` | in the UTF-8 failure arm, return an `InvalidData` io error instead of writing the reply (the pre-task behaviour) |
| `a_line_past_the_cap_is_refused_and_the_connection_ends` | remove the `take` limit so `read_until` is unbounded |
| `a_line_exactly_at_the_cap_is_served` | lower the take limit to `MAX_LINE_BYTES` (the off-by-one) |
| `a_final_line_without_a_newline_is_still_answered` | serve only buffers that end in `b'\n'`, skipping the final one |
| `a_panicking_handler_is_answered_internal_before_the_panic_resumes` | delete the `catch_unwind`, call the handler directly |
| `a_panicking_handler_still_answers_the_client_over_a_real_socket` | `resume_unwind` before writing the reply |
| `a_handler_returning_a_loop_owned_code_is_answered_internal_and_the_loop_keeps_serving` | delete the guard, return the handler's error unchanged |
| `a_cell_with_no_genome_omits_the_key_in_both_directions` | remove the `skip_serializing_if` attribute |
| `an_unknown_error_code_does_not_deserialize` | remove `-32603` from `ErrorCode::from_i64` |

### Files to change, and nothing else

- `crates/plasmosome-core/src/control.rs` — the loop rewrite, `MAX_LINE_BYTES`, `Answered`,
  the guard, the doc rewrite, seven tests.
- `crates/plasmosome-core/src/protocol.rs` — `ErrorCode::Internal`, `WireError::internal()`,
  `WireError::line_too_long()`, the `genome` serde attributes, two tests touched.
- `crates/plasmosome-core/src/lib.rs` — add `MAX_LINE_BYTES` to the existing `control` re-export
  line.
- `docs/specs/001-control-protocol.md` — the §1 edits quoted above, verbatim.
- `docs/decisions/NNN-...` — the decision record, numbered from the remote.
- This file — `## Notes`, status transitions, `pr:`.

No change to `plasmosome-membrane`, no new dependency, no new crate, no freeze-check edits
(`protocol.rs` is already on the wire-source list and no type is added to the serde
round-trip test — `ErrorCode` and `WireError` are already there).

Read only the files in `refs:`. Do not explore beyond them. Task 014's `## Plan` and `## Notes`
are in `refs:` as the house pattern for this exact module — read them before writing anything.

### Definition of done

Every line of `done_when`; the nine tests above green with the seven required initial-red runs
recorded; the nine mutations each observed failing and recorded; the spec and decision files
merged in the same PR; and the gate in root `AGENTS.md`:

```shell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
./.githooks/provenance-guard
```

The PR description leads with the problem: the control loop had five edges nobody owned, and a
client could see a dead socket where it asked a question. Follow `.agents/skills/pr-review`.

Other agents are live in `.worktrees/task-012`, `.worktrees/task-017`,
`.worktrees/skills-orchestration`, `.worktrees/guard-fix`, `.worktrees/decision-003`, and
`.worktrees/spec-recovery` — work only in your own worktree, and put `016` in any scratch
filename.

STOP when done. Do not start the daemon binary, the accept loop, or any new verb.

## Notes


### The `-32603` test edit, sequenced first, passed before and after

Decision 3's first step — swapping `-32603` for `-32604` on the unknown side of
`an_unknown_error_code_does_not_deserialize` — passed against the pre-task code, exactly as the
plan predicted, since `-32604` is outside the table either way:

```text
test protocol::tests::an_unknown_error_code_does_not_deserialize ... ok
```

The other half of that edit — asserting `-32603` reads back as `ErrorCode::Internal` — cannot be
written before the variant exists, so it does not compile rather than failing. **Six of the nine
tests in the plan's table could be seen red, not seven.** Two are marked *(passes before)* by the
plan; this is the third that cannot start red, and mutation 9 below is what holds it honest.

### Six tests seen failing before the loop was rewritten

With `MAX_LINE_BYTES`, `ErrorCode::Internal`, `WireError::internal()` and
`WireError::line_too_long()` in place as stubs, and the loop still `reader.lines()`:

```text
test result: FAILED. 77 passed; 6 failed; 0 ignored; 0 measured; 0 filtered out

failures:
    control::tests::a_handler_returning_a_loop_owned_code_is_answered_internal_and_the_loop_keeps_serving
    control::tests::a_line_past_the_cap_is_refused_and_the_connection_ends
    control::tests::a_line_that_is_not_utf8_is_answered_parse_error_and_the_loop_keeps_serving
    control::tests::a_panicking_handler_is_answered_internal_before_the_panic_resumes
    control::tests::a_panicking_handler_still_answers_the_client_over_a_real_socket
    protocol::tests::a_cell_with_no_genome_omits_the_key_in_both_directions
```

The one that shows the pre-task behaviour most plainly is the non-UTF-8 line: `lines()` turned it
into a transport failure, so the loop returned an error instead of an answer.

```text
the loop serves the scripted lines to a writer that cannot fail:
Error { kind: InvalidData, message: "stream did not contain valid UTF-8" }
```

```text
only the loop saw the frame, so only the loop may answer about it:
{"error":{"code":-32700,"message":"the line is not JSON"},"id":1}
  left: -32700
 right: -32603
```

```text
the script asked for 1 replies and the loop wrote 2:
["{\"id\":null,\"error\":{\"code\":-32700,\"message\":\"the line is not JSON\"}}",
 "{\"id\":2,\"result\":{}}"]
```

```text
the panicking request is answered and nothing after it is served: []
  left: 0
 right: 1
```

```text
a reply field with nothing in it is omitted, never sent as null:
{"genome":null,"id":"cell-2","plasmids":[],"state":"draining"}
  left: Some(Null)
 right: None
```

`a_line_exactly_at_the_cap_is_served` and `a_final_line_without_a_newline_is_still_answered`
passed against `lines()` by construction, as the plan said they would. All 83 pass after the
rewrite.

### The nine mutations, each applied, run, observed, and reverted

**1. Return an `InvalidData` io error from the UTF-8 failure arm** —
`a_line_that_is_not_utf8_is_answered_parse_error_and_the_loop_keeps_serving`:

```text
the loop serves the scripted lines to a writer that cannot fail:
Custom { kind: InvalidData, error: "stream did not contain valid UTF-8" }
```

**2. Remove the `take` limit so `read_until` is unbounded** —
`a_line_past_the_cap_is_refused_and_the_connection_ends`:

```text
the script asked for 1 replies and the loop wrote 2:
["{\"id\":null,\"error\":{\"code\":-32700,\"message\":\"the line is not JSON\"}}",
 "{\"id\":2,\"result\":{}}"]
  left: 2
 right: 1
```

**3. Lower the take limit to `MAX_LINE_BYTES`** — `a_line_exactly_at_the_cap_is_served`. The
off-by-one splits the line in two: the cap-length content is served, then its orphaned newline is
answered as an empty line.

```text
the script asked for 1 replies and the loop wrote 2:
["{\"id\":1,\"result\":{}}",
 "{\"id\":null,\"error\":{\"code\":-32700,\"message\":\"the line is not JSON\"}}"]
  left: 2
 right: 1
```

**4. Serve only buffers that end in `b'\n'`** — `a_final_line_without_a_newline_is_still_answered`:

```text
the script asked for 2 replies and the loop wrote 1: ["{\"id\":1,\"result\":{}}"]
  left: 1
 right: 2
```

**5. Delete the `catch_unwind`, call the handler directly** —
`a_panicking_handler_is_answered_internal_before_the_panic_resumes`:

```text
the panicking request is answered and nothing after it is served: []
  left: 0
 right: 1
```

**6. `resume_unwind` before writing the reply** —
`a_panicking_handler_still_answers_the_client_over_a_real_socket`. This is the mutation the
in-memory test cannot catch: reply-before-close is only observable to a client on the other end
of a socket.

```text
the client reads a whole reply line and then end of input: []
  left: 0
 right: 1
```

**7. Delete the guard, return the handler's error unchanged** —
`a_handler_returning_a_loop_owned_code_is_answered_internal_and_the_loop_keeps_serving`:

```text
only the loop saw the frame, so only the loop may answer about it:
{"error":{"code":-32700,"message":"the line is not JSON"},"id":1}
  left: -32700
 right: -32603
```

**8. Remove the `skip_serializing_if` attribute** —
`a_cell_with_no_genome_omits_the_key_in_both_directions`:

```text
a reply field with nothing in it is omitted, never sent as null:
{"genome":null,"id":"cell-2","plasmids":[],"state":"draining"}
  left: Some(Null)
 right: None
```

**9. Remove `-32603` from `ErrorCode::from_i64`** — `an_unknown_error_code_does_not_deserialize`.
This is the guard that earns the test edit no initial red could:

```text
-32603 is in the table: Error("-32603 is not a control protocol error code", line: 0, column: 0)
```

### Where the plan met the compiler

**`result_large_err` fired on the `catch_unwind` closure, as the plan warned.** The closure returns
the handler's `Result<Value, WireError>`, and the 384-byte `WireError` trips the lint at a second
site:

```text
error: the `Err`-variant returned from this closure is very large
   --> crates/plasmosome-core/src/control.rs:149:53
    = note: `-D clippy::result-large-err` implied by `-D warnings`
```

Following the plan's instruction to widen the existing `#[expect]`'s placement rather than add an
`allow`: the expectation moved off `Handler::handle` to a module-level `#![expect(...)]` at the top
of `control.rs`, with its reason generalized from "every error path here" to "every error path in
this module". One expectation, one reason, both sites covered — rather than a second attribute
repeating the first. Nothing else in the module returns a `Result<_, WireError>`, so the wider
placement suppresses nothing a narrower pair would not have.

**A byte-script test helper.** `reply_lines` took `&str`, which cannot express a `\xFF` line.
It now delegates to `reply_lines_of(&[u8], …)`, and `converse` to `converse_of`; every existing
call site is unchanged. `parse_replies` was split out because the two panic tests need to read the
writer's bytes themselves — the helpers assert a reply count, and a panic test has to inspect the
buffer that survived the unwind.

**The in-memory panic test holds the writer across the unwind.** `serve_connection` takes the
writer by value, so a `Vec<u8>` moved into `catch_unwind` would be lost with the panic. The test
passes `&mut written` inside `AssertUnwindSafe` and reads the buffer after the catch, which is
what makes "exactly one reply was written before the panic resumed" observable at all.

### What the daemon unit still inherits

Unchanged from the plan's out-of-scope list, and restated in decision 005 so it is not rediscovered:
an unterminated line **under** the cap still parks its connection until the client hangs up. The
cap cannot end it — nothing has exceeded anything — and only a daemon-level read or idle timeout
can. That, and whether connections are served one at a time or concurrently, stay open.
