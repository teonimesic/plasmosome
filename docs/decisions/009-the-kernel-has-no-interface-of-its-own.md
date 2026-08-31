---
id: 009
title: The kernel has no interface of its own
date: 2026-08-31
status: accepted
---

## Context

Somebody is about to write the first line of Plasmosome's command-line tool, and the repository
does not say which crate it goes in. Intent 009 asks for that command line — granular enough that
an agent drives it a step at a time — and rightly says nothing about where it lives. There is no
`plasmosome` binary in the workspace; the only binary targets are `membraned` and `plasmid`. Yet
`docs/specs/001-control-protocol.md` is accepted and already assumes one, starting a kernel
instance with `plasmosome start --name work` in its §2.

That spec also settles the shape the crates have to fit. The control socket is the controller's
only control surface, and "the CLI (`plasmosome` / `plasmid` binaries), the future MCP server
(D1: a later transposition of the same verbs), and any test harness are all clients of this one
socket". So the open question is not whether the kernel gets a command line. It is which crate
holds it, and whether a library sits between that crate and the kernel.

`plasmosome-core` already answers half of it, and that half needs stating precisely. Core does
expose a surface: `serve_connection`, `Handler`, `MAX_LINE_BYTES` and the whole `protocol` module
are public, and core's crate doc says it "answers the frozen control protocol on an ndjson
connection". That surface is not core's invention — it is the contract spec 001 freezes and every
client speaks. What core has none of is a way of being driven that it owns.
`serve_connection<R: BufRead, W: Write, H: Handler>` is generic over its reader and writer, so
core binds no socket outside its own tests; its dependencies are `plasmosome-backend`, `serde`,
`serde_json` and `toml`, so it parses no arguments and speaks no transport. No rule says it must.

## Decision

**`plasmosome` is the CLI**: a binary crate, what `cargo install plasmosome` gets you, kept
deliberately thin. **`plasmosome-core` is kernel logic with no interface of its own** — it
exposes the frozen control protocol, transport-agnostic, and nothing else. "Of its own" is the
whole of it: no argument layer, no MCP server, no transport binding, nothing assuming who is on
the other end of the reader and writer it is handed. Answering a protocol somebody else froze is
not owning one. This ratifies what the crate is rather than asking it to change.

**A `plasmosome-cli` library between the two is deliberately not created now.** There is no
`plasmosome` command-line code at all yet, so a third crate would have one consumer and nothing
to test. It earns its place at a trigger concrete enough to recognise: when the CLI grows state a
test wants to drive without spawning a process — argument parsing with real error cases, a TUI's
model, an output formatter with more than one format — or when a second consumer wants the same
client code, which spec 001 already anticipates in the form of an MCP server. Whoever hits one of
those revisits this file rather than re-deriving the answer.

## Rejected

**Three crates from the start.** `plasmosome-cli` today would wrap nothing and be exercised by
one caller. `AGENTS.md` states it outright: two adapters means a real seam, one means a
hypothetical one — do not abstract until something second exists. What such a library would wrap
is the protocol surface core already exposes, so the CLI is a client of an existing contract.

**Put the CLI in `plasmosome-core` as a binary target.** Cargo gives a package one
`[dependencies]` table, shared by its library and binary targets, so core would acquire an
argument parser and a socket client. That is escapable — optional dependencies with
`required-features` on the `[[bin]]` is the standard arrangement, and library consumers then
never build them — but the escape is what costs. `cargo install plasmosome` would need a
`--features` incantation to deliver a tool, and "core has no interface of its own" would stop
being answerable from the manifest, since an optional dependency is in the build or not depending
on which features are on. This repository already asks that question mechanically, walking
`cargo tree` and reading `[dependencies]` in `plasmosome-freeze-checks/tests/freeze_rules.rs`.

**Leave the `plasmid` binary in `plasmid-sdk`.** The same shared table, and there it widens a
promise. The README calls `plasmid-sdk` "The stability boundary for plasmid authors — build
against this, not against the kernel", so its dependency table is what a plasmid author reads.
Nothing has been paid yet: the crate has no `[dependencies]` section at all, so
`cargo tree -p plasmid-sdk` prints it alone, and its binary is a 31-line stub over the standard
library. The first dependency that binary needs lands in the published library's table, and the
escape above costs the same here — an author asking what the boundary drags in gets a
feature-dependent answer instead of an empty one. Spec 001 also lists `plasmid` beside
`plasmosome` as a client of the control socket: a socket client and an authoring contract share a
table only because they sit in one directory. Moving it is a separate change already in flight.

## Consequences

The `plasmosome` name is claimed ahead of the CLI, by a separate change: a lib-only 0.0.0
placeholder holding the package name before there is anything to install. Whoever writes the
command line converts that package, or claims the name themselves if it has not landed by then.
Lib-only is deliberate: at 0.0.0 a bin target would let `cargo install plasmosome` succeed and
hand back no tool, where lib-only refuses cleanly, and a registry reserves a name, not a target
kind. Publishing it at all is a deliberate exception to the workspace-wide `publish = false` a
freeze check now holds every member to. Nothing runs `plasmosome start` until the CLI is written.

**This settles the CLI's crate, not the daemon's.** Spec 001 names a `plasmosomed` controller,
and decision 005 leaves connection policy to it — a read or idle timeout, and whether connections
are served one at a time or concurrently. Whether it is a second binary in the `plasmosome`
package or a package of its own re-opens the shared-table argument above.

Kernel logic worth testing lives in core, never in the binary, which stays thin enough to need no
tests of its own. That is a constraint on whoever writes the CLI, not a freebie: the day the
binary accumulates state a test wants — the trigger above — that state goes to `plasmosome-cli`,
not into core. Standing that crate up is a real refactor with a PR of its own, deferred knowingly.

The naming note that describes the CLI is superseded by this decision. Its two verb groups need
no revisiting on their content — spec 001 §2 states the same split — but their authority does:
the verbs come from §2, which is accepted and in this repository, not from the note.
