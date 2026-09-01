---
id: 009
title: A command line granular enough for an agent to drive
status: approved
date: 2026-08-31
originator: Stefano
outcome:
---

I want to use Plasmosome from a command line, and I want that command line granular and
composable, so an agent can use it directly.

Today the kernel is only reachable from code. Everything it can do — bring a cell up, attach a
plasmid, take one away, ask a cell what it is holding right now — sits behind a library, so the
only things that exercise it are its own tests. I want to drive it myself from a terminal, and I
want an agent to drive it directly.

An agent is a first-class user of this thing, not an afterthought behind an interface built for a
person. I do not want a human command line now and something bolted on for agents later. One set
of operations, and both can use them.

Granular and composable is what makes that possible. Small operations that combine, rather than
one large command that assumes somebody is sitting there deciding what happens next. An agent
works by taking a step, reading what came back, and choosing the next one from that — a single
entry point that runs a whole session start to finish leaves it nothing to read and nothing to
recombine. The same smallness is what lets anyone assemble something the command line's author
never thought of.

What I get out of it: the kernel stops being something only its author can reach, and becomes
something an agent operates directly, one step at a time.

## Outcome

(filled in later)
