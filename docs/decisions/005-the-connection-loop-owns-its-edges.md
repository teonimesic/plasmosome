---
id: 005
title: The connection loop answers every edge it hits, and closes only when it cannot keep its word
date: 2026-08-31
status: accepted
---

## Context

`serve_connection` reads a line, answers it, and keeps going. That is the whole wire contract and
it was the right amount to freeze first. What it had no owner for was everything around the wire:
bytes that are not text, a line that never ends, a handler that panics mid-answer, a handler that
returns a code the loop reserves for itself, and whether an absent field travels as `null` or not
at all.

Five questions, all about one conversation, and none of them decidable by looking at a single
line. Left open, each would be settled by whichever connection hit it first, in whichever verb
happened to be under construction that week — which is how two connections end up disagreeing
about what a protocol says.

One constraint made a piece of this harder than it looks. Spec 001's error table is closed:
`ErrorCode` refuses to deserialize an integer outside it, deliberately, so that a client reading
an invented code is caught rather than absorbed. And the table had **no code meaning "the
controller failed"** — every code in it blames the request. There was nothing honest to say when
the fault was ours.

## Decision

**A line past 1 MiB is answered, then the connection closes.** `MAX_LINE_BYTES` is `1 << 20`. A
line that reaches it without a newline gets `-32600` under a `null` id, with a message naming the
cap, and then `serve_connection` returns `Ok(())`. Every frozen v1 verb's params are names, small
maps and short argv lists, orders of magnitude under the cap, so no honest client comes near it;
a hostile one costs one megabyte and one reply per connection.

**A handler panic is answered `-32603`, and then the panic resumes.** Only the `handler.handle`
call is wrapped, in `catch_unwind(AssertUnwindSafe(...))`. The client gets a protocol error rather
than a socket that dies mid-sentence; the connection then ends; the process above still sees the
panic and owns what happens next. The panic payload never reaches the wire — the message is
generic and the payload travels only through `resume_unwind`, where a host process can log it.
`AssertUnwindSafe` is honest here for exactly one reason: the handler is never touched again after
the catch.

**A handler that returns `-32700` or `-32600` is answered `-32603` in its place, and the loop
keeps serving.** Those two codes say something about framing, and only the loop saw the frame. A
handler that returns one is wrong about the protocol, not broken — it returned normally, so its
state is coherent and the conversation continues. `-32601` and `-32602` stay handler-owned: the
loop cannot know which methods a handler serves.

**A line that is not UTF-8 is answered `-32700` and the conversation continues.** JSON is UTF-8 by
definition, so "the line is not JSON" is the truthful answer with no new code. The id is `null`:
an id read out of bytes the loop refused to trust is not an id. The bytes are never decoded
lossily first — replacement characters can produce a line that parses, and answering a request the
client did not send is worse than refusing one it did.

**An absent optional reply field is omitted, never sent as `null`.** `WireError` already omits
every absent field; a cell with no genome now has no `genome` key. One reply convention, not two,
and a client checking for the key gets one shape.

## Rejected

**Resynchronizing after an over-cap line by discarding to the next newline.** It reads a hostile
client's bytes without bound, which is the thing the cap exists to stop. Worse, the loop would
then be pairing later replies to later requests on an assumption it cannot check — it never saw
where the oversized line actually ended.

**Catching a panic and continuing to serve.** A handler that panicked part-way through `&mut self`
may hold broken invariants, so every answer after it is a guess dressed as a reply.

**Documenting "handlers must not panic" and catching nothing.** The client sees a silent close it
cannot tell from a crashed controller. This protocol's whole stance is machine-legible refusal;
a dead socket is the one answer that cannot be read.

**Dropping the connection on a non-UTF-8 byte.** It punishes a one-byte corruption exactly as hard
as an attack, and it withholds the answer the client is owed for a line it framed.

**A type split so a handler cannot express the loop-owned codes.** Two error types and a changed
`Handler` signature, with a `Box::new` at fifteen construction sites — and it would not close the
hole. The freeze checks hold `WireError` to serde in both directions, so a relayed error (§3.6
relays the membrane's answers verbatim) can always carry any code. A runtime guard at the single
point where a handler's answer enters the wire is the whole fix.

**`"genome": null`.** A second convention sitting next to `WireError`'s omit-when-absent, for no
gain a client can use.

**Deferring all five to the `plasmosomed` binary.** Whichever connection hit an edge first would
have decided it, which is the complaint that started this.

## Consequences

**`-32603` is a contract change to a table spec 001 calls closed**, and it is made in spec 001 in
the same change that uses it, not left implicit in code. A test that asserted `-32603` must not
deserialize now asserts it reads back as `ErrorCode::Internal`. Any client holding the old closed
set will refuse a reply carrying the new code — which is the closed table working as designed, and
the reason the addition is written into the spec rather than smuggled in.

**An unterminated line under the cap still parks its connection.** A client that sends half a line
and then nothing holds that connection until it hangs up; the cap cannot end it, because nothing
has exceeded anything. Only a daemon-level read or idle timeout can, and there is none. That, and
whether connections are served one at a time or concurrently, stay open for the daemon unit on
purpose — they are properties of a process, and no process exists yet to own them.

The five answers above are now properties of `serve_connection` itself, so every future daemon
connection inherits them without deciding anything.
