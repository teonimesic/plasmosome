---
name: planning-work
description: How work is planned before it is written — the plan/execute model split and what a plan must contain. Use when starting a piece of work or briefing another agent.
---

# Planning work before writing it

**Plan with the most powerful model available; execute with the next one down.** The planner
reads the references and makes every design decision (API shape, test list, testability seam,
platform caveats) and writes no code. The executor receives that plan verbatim, plus: *execute
exactly; if the plan contradicts reality, stop and report rather than improvise.*

**One unit of work per agent, per branch, per PR** — one finishable capability, not a theme.

**The planner's output is a file, not a message.** A plan is written into `tasks/NNN-slug.md`
with `status: planned` — plus, for work that crosses the spec threshold, an accepted spec in
`docs/specs/` that the task's `specs:` field names. A brief that exists only in a chat window
cannot be picked up tomorrow by a different agent. See `.agents/skills/tasks` for the formats
and the thresholds.

**Dispatch is one line: "work task NNN."** The executor opens that file and reads only it and
the files it names.

**One worktree per agent.** Agents never share a checkout — substitute your own names into
`git worktree add ../wt-ledger-replay -b task-004-ledger-replay`. Two agents in one clone will
switch branches under each other and commit to the wrong one. Never run `git reset --hard`,
force-push, or branch-switch a checkout you did not create —
if you find your commit on the wrong branch, cherry-pick it where it belongs and leave the other
branch alone.

The task's `## Plan` contains:
- One deliverable in a sentence, and an explicit list of what is out of scope.
- The exact files to read, and an instruction not to explore beyond them.
- A test table: each test's name and what it must prove.
- A definition of done including the gate (see root `AGENTS.md`).
- "STOP when done — do not start the next piece of work."

Before the executor calls the work finished, it appends to the task's `## Notes` whatever the
next agent would otherwise have to work out again: what was tried and abandoned, where a surprise
was hiding, which reference turned out to be wrong.

Honesty outranks finishing: never report a green you did not run; time-box a blocker, then
document it and move on rather than weakening a test; if a test passes both before and after the
fix, say so.
