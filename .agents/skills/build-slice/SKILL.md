---
name: build-slice
description: How work is planned and executed in this repository — the plan/execute split and what a slice is. Use when starting a new piece of work, writing a brief for another agent, or deciding how much to take on at once.
---

# Building a slice

Work here moves in **slices**: one narrow, finishable capability per branch, per PR, per agent.
The pattern exists because the alternative was tried and failed — a brief covering five concerns
at once produced an agent that wandered and delivered nothing.

## Plan and execute are different jobs

**Plan with the strongest model available. Execute with the next one down.**

The planner reads the references and makes *every* design decision: the API shape, the test
list, the seam that makes the thing testable, the platform caveats. It writes no code. Its
output is a document specific enough that an executor never has to invent anything.

The executor receives that plan **verbatim** with one instruction attached: execute exactly; if
the plan contradicts reality, stop and report the contradiction rather than improvising.

This split is where the leverage is. A planner given room to think catches what a busy executor
misses — in one case, that the reference implementation being copied never reaped its child
processes at all, so the "port it" instruction would have propagated the exact bug the slice
existed to fix.

## What a slice brief contains

- **One deliverable**, stated in a sentence, with an explicit list of what is *not* in scope.
- **The exact files to read** — and an instruction not to explore beyond them. An agent that
  reads the whole repository has spent its attention before it starts.
- **A test table**: each test's name and what it must prove. Names are constraint statements,
  not labels.
- **A definition of done** that includes the full gate (tests, clippy with warnings denied,
  formatting, the provenance guard).
- **"STOP when done — do not start the next slice."** Without it, agents continue into work
  nobody reviewed the plan for.

## Honesty rules that outrank finishing

- Never report a green you did not run.
- A blocker gets a time-box, then documentation, then you move on — it does not get a workaround
  that quietly weakens a test.
- If a test passes both before and after the fix, say so. A guard is useful; a guard mislabeled
  as proof is not.
