---
id: 016
title: The control loop needs a daemon to own its edges
status: todo
priority: 2
specs: [001]
intents: []
refs:
  [
    crates/plasmosome-core/src/control.rs,
    docs/specs/001-control-protocol.md,
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

Five edges, each with the evidence a reviewer found in the code as it stands.

**A non-UTF-8 byte kills the connection with no reply.** `serve_connection` iterates
`reader.lines()`, which yields `Err(InvalidData)` for a line that is not UTF-8. The `line?` at
the top of the loop propagates it and returns. The connection dies, the client is still waiting,
and nothing was written. The function's own documentation says "a line that fails to parse is
answered and the conversation continues" — a non-UTF-8 line is a line that fails to parse, and
it is neither answered nor continued. Decide whether such a line gets `-32700` and the loop goes
on (`read_until` plus a lossy decode), or whether the daemon drops the connection on purpose.

**There is no line-length cap.** `lines()` grows its buffer until it finds a newline. A client
that opens the socket and writes without one makes the controller allocate until it cannot. Any
cap is a daemon-level number, and refusing a line past it is a wire-visible answer the spec does
not yet name.

**A panicking handler unwinds through the loop.** Nothing catches. The panic leaves
`serve_connection`, and in the socket test's shape it surfaces to the caller as a join error, not
as a protocol error — so a client sees a closed socket where it asked a question. The `Handler`
documentation says what to return for a method it does not serve and for params that do not
parse; it never says a panic is fatal to the connection. Decide: catch and answer, or state that
a handler must not panic and let the daemon restart.

**Nothing stops a handler returning the loop's own reserve codes.** `WireError::parse_error`,
`invalid_request`, `method_not_found` and `invalid_params` are public constructors, and
`Handler::handle` may return any `WireError`. The loop owns `-32700` and `-32600`, and the
handler owns `-32601` and `-32602` by convention only. A handler answering `-32700` produces a
reply that says the line was not JSON about a line that parsed. The division of labour wants
either a type that cannot express it or a test that holds it.

**`"genome": null` versus omitting the key is undecided.** `CellStatusEntry::genome` is an
`Option<GenomeName>` with no `skip_serializing_if`, so a cell with no genome serializes as
`"genome": null`. Spec 001 §3.3 shows only cells that have one. A client written against the
example sees a key the example does not have. Either encoding is fine; which one it is belongs in
the spec, because a client branches on it.

## Notes
