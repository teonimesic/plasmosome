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

**Parallel agents share one scratchpad. Name your files uniquely.** Three executors running at
once were all given the same scratch directory, and two wrote a PR body to `pr-body.md`. One
overwrote the other, and a `gh pr edit` then pushed the wrong PR's text onto a PR. Put your task
number in the filename, or write to a directory you made.

**Say who else is running.** An agent that finds its PR edited underneath it will reasonably
conclude something is wrong. Tell each one which other agents are live and what they own, so a
collision reads as a collision rather than an attack.

**Never commit in a worktree someone else is working in — not even a one-line docs change.**
`git add -A` takes whatever is in the tree, including the half-finished file the agent working
there has not committed yet. Creating the worktree does not make it yours while an agent is in it.
If a change is urgent and the worktree is busy, branch from `main` somewhere else and let the two
land separately.

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

**Brief the executor on the problem, not only the plan.** A plan that arrives as a list of files
and assertions produces a change that works and a description nobody can read, because the person
who wrote it was never told what it was for. Give them the intent in a sentence, and say the PR
description has to lead with it — `.agents/skills/pr-review` says how.

Honesty outranks finishing: never report a green you did not run; time-box a blocker, then
document it and move on rather than weakening a test; if a test passes both before and after the
fix, say so.
