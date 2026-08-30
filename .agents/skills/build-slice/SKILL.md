---
name: build-slice
description: How work is planned and executed here — the plan/execute model split and what a slice brief contains. Use when starting a piece of work or briefing another agent.
---

# Building a slice

**Plan with the most powerful model available; execute with the next one down.** The planner
reads the references and makes every design decision (API shape, test list, testability seam,
platform caveats) and writes no code. The executor receives that plan verbatim, plus: *execute
exactly; if the plan contradicts reality, stop and report rather than improvise.*

**One slice per agent, per branch, per PR.** A slice is one finishable capability.

**One worktree per agent.** Agents never share a checkout: `git worktree add ../wt-<slice> -b
<branch>`. Two agents in one clone will switch branches under each other and commit to the wrong
one. Never run `git reset --hard`, force-push, or branch-switch a checkout you did not create —
if you find your commit on the wrong branch, cherry-pick it where it belongs and leave the other
branch alone.

A brief contains:
- One deliverable in a sentence, and an explicit list of what is out of scope.
- The exact files to read, and an instruction not to explore beyond them.
- A test table: each test's name and what it must prove.
- A definition of done including the gate (see root `AGENTS.md`).
- "STOP when done — do not start the next slice."

Honesty outranks finishing: never report a green you did not run; time-box a blocker, then
document it and move on rather than weakening a test; if a test passes both before and after the
fix, say so.
