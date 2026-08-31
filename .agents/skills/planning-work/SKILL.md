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

**One worktree per agent, under `.worktrees/`.** Agents never share a checkout — substitute your
own names into `git worktree add .worktrees/task-004-ledger-replay -b task-004-ledger-replay`.
`.worktrees/` is gitignored, so the checkouts stay inside the repo and one `rm -rf` cleans them
all up. Two agents in one clone will switch branches under each other and commit to the wrong
one. Never run `git reset --hard`, force-push, or branch-switch a checkout you did not create —
if you find your commit on the wrong branch, cherry-pick it where it belongs and leave the other
branch alone.

What a `## Plan` must contain is written once, in `.agents/skills/tasks` under "Writing a
`## Plan` for a stranger". Read it there rather than from a second copy here.

Before the executor calls the work finished, it appends to the task's `## Notes` whatever the
next agent would otherwise have to work out again: what was tried and abandoned, where a surprise
was hiding, which reference turned out to be wrong.

Nested checkouts are invisible to `git grep` and to `rg`, which is what the gate and the guard
use, and to `cargo`, whose workspace members are explicit paths. They are **not** invisible to a
plain `grep -r` or `find` from the repo root: those walk every worktree and report the same file
many times. Search with `git grep` or `rg`.

Honesty outranks finishing: never report a green you did not run; time-box a blocker, then
document it and move on rather than weakening a test; if a test passes both before and after the
fix, say so.
