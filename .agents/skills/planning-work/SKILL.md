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

## Who does the work

The agent that dispatches other agents does not write the change. It dispatches, decides, and
watches. It does not write product code, tests, PR descriptions, specs, or plans. Planner and
orchestrator are separate roles: where a spec or a plan is needed, the orchestrator dispatches a
planner agent, and that agent writes it.

**Three roles, and the orchestrator is only one of them.** The **author** owns its PR from opening
through merging — writes it, answers the findings, resolves the threads, and merges it. Not "hands
it back once it is clean": merges it. **Reviewers** exist to contradict it and find what is wrong
with it. That is the whole job; they do not fix and they do not merge. The **orchestrator**
dispatches,
watches for work that has stalled, and puts in front of a person anything needing a decision or an
action only a person can take.

**Review findings go back to the agent that wrote the change.** That agent still holds the
context — why the code is shaped the way it is, what it already tried and dropped. Resume it
rather than spawning a fresh one to work all of that out again, and rather than fixing the finding
yourself. A reviewer that patches what it is reviewing has stopped being an independent reader
of it.

**Do not transcribe review findings.** An agent can read the pull request. Dispatch it to the PR —
"you own PR #23, read the open threads, resolve them, push" — and let it read the reviewer's words
rather than the orchestrator's summary of them. Brief only what is genuinely not in the PR: a
decision that is yours to make, a constraint the reviewer could not know, or which of two
directions to take.

The orchestrator's own work is the judgement a dispatched agent cannot make: which piece of work
comes next, what is in scope and what is not, which of two designs to take, when something needs a
decision record rather than a spec, and what has to go to a person because no agent may settle it.
That is the whole job, and it is enough of one.

**The orchestrator writes nothing that ships.** Every file a change touches belongs to a dispatched
agent — under `crates/`, under `docs/`, under `.agents/`, anywhere — and so do the PR that
carries it, its threads, and its merge. The decision record included, the spec included.

The one thing it edits itself is a claim in `tasks/` whose own author is gone: the `status:`, the
`evidence:`, and the `## Notes` line recording what established the truth. An author still closes
its own task, the way `.agents/skills/pr-review` describes; this is for the abandoned ones nobody
is left to close. That edit reaches `main` on a branch through a PR like everything else, and the
orchestrator authors that one — it is the single exception, and stretching it is how the drift
below starts.

**An orchestrator that merges is the single gate the whole queue waits at.** Every finished PR sits
there until one agent gets round to it, and nothing a second party brings to the merge was missing:
the author has the context and the PR already open.

Doing it yourself feels faster, and that is why the drift happens: it starts with a review finding
too small to seem worth handing back, and grows from there. You can see the one change moving and
you cannot see the four that never started. Measured end to end, work reaches `main` more slowly,
not faster.

## Handing work over

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
