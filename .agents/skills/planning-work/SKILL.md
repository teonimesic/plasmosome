---
name: planning-work
description: Plan in Beads before execution, then dispatch by native task ID with clear ownership and isolated code worktrees.
---

# Planning work before writing it

Plan with the most powerful model available; execute with the next one down. The planner reads
the references, settles API shape, verification cases, testability seams and platform caveats,
and writes the plan into the Beads task's `design`. The executor reads that record and executes
it; if reality contradicts the plan, stop, record the discrepancy in its notes and return the
decision to the planner rather than silently inventing a different assignment.

One finishable capability belongs to one author, code worktree and PR. All task operations use
`./tools/work-state`; `.agents/skills/tasks` defines admission, planning contents and the native
claim procedure. A plan in chat or a worktree file is not a handoff and cannot be its authority.

## Roles and dispatch

The orchestrator dispatches and makes cross-task decisions: order, non-overlapping ownership,
what is in scope, and what must go to a person. It does not write the product change, spec, plan,
PR body or decision record; those belong to a dispatched author or planner. It may reconcile an
abandoned author's task in Beads from evidence, without opening a bookkeeping PR.

The author owns its PR through merge: implementation, evidence, responses to findings, thread
resolution and task closure. The independent reviewer contradicts the change, does not repair
it and does not merge it. The author spawns that reviewer where possible; the orchestrator is the
fallback. Return findings to the original author while it remains available, because it holds
the reasoning behind the change. A PR with unanswered threads and no author working it is the
concrete failure this division must prevent.

Dispatch by full native ID: **“Work Beads ID; start with `./tools/work-state show ID`.”** The
record carries the problem, plan, acceptance and references. Add only constraints or decisions
not yet available there, then persist those in Beads too. Name other live agents and their file
ownership, so overlapping edits can be coordinated before either party changes the file.
The executor uses its own unique actor and performs the atomic claim before code work. A losing
claim ends that dispatch; an orchestrator's message does not override Beads ownership.

For review repairs, dispatch by task ID and PR URL: let the author read the actual threads,
rather than replacing the reviewer's words with an orchestrator's transcription. Review findings
remain on the PR, with task notes linking the durable result.

## Code isolation, not task isolation

Use one worktree per author under `.worktrees/`, with a unique branch and directory, for example:

```shell
git worktree add .worktrees/beads-ID-agent -b beads-ID-agent main
```

The worktree isolates code only. Every invocation of its `./tools/work-state` discovers the same
absolute Git common directory and therefore the same native task database and command lock.
Never copy the database, tasks or plans into a worktree to make a private queue. Independent
clones are not a shared writer domain; spec016 defines that limitation.

Never commit in another agent's worktree or stage its unfinished files. Never reset, force-push,
or switch a checkout another agent owns. Preserve unmerged and uncertain worktrees; after a
verified merge remove only a clean worktree whose owner has finished, by its actual path rather
than a guessed branch-derived path. Worktree cleanup does not close or release a task.

Give temporary files unique task-and-actor names or a private scratch directory. Two authors
sharing `pr-body.md` have overwritten each other's PR descriptions; that collision is the reason,
not a new place to store plans. Search the intended checkout rather than recursively walking all
of `.worktrees/`.

Before handing off, append important attempts, surprises, references and verification results to
Beads notes. Report only evidence actually observed; a test that passes before and after a fix
has not proved that fix. A blocker remains recorded as blocked work, not a weakened acceptance
condition or a claim that the deliverable is finished.
