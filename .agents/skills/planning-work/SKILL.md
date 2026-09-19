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

Before authoring a native task's plan, take its owned `planning` phase using the tasks skill.
Planning is visible work even when implementation awaits prerequisites; preserve those blockers
instead of hiding the active planner in `blocked`. The native phase follows the work actually
assigned, not a taskless prerequisite spec PR. Such PRs retain spec012's distinct shape.

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
Name whether this assignment is planning, implementation or review repair. The dispatched
actor follows spec016's claim or existing-owner transition for that phase before work. A losing
claim ends that dispatch; an orchestrator's message does not override Beads ownership.

Planner dispatches leave a record before any launch; [spec019](../../../docs/specs/019-dispatched-planners-leave-a-trace.md)
is the contract and these are its operations. The dispatch is written on its **carrier**, the
record of the work the planner will update. One carrier per subject: enumerate first and reuse
an existing non-closed carrier naming the subject.

- Design planner for a draft spec no task implements: enumerate non-closed tasks naming that
  spec (full `./tools/work-state list --all --limit 0 --json`); file the task when none exists
  (filing against a draft spec is valid) and it is the carrier.
- Spec author for a mapped task lacking its governing spec: the task is the carrier and the
  dispatch is a dated note on it; the planner takes no phase and no claim.
- Spec author for an approved intent with no spec: the carrier is a standalone chore record,
  shaped in the tasks skill; the planner may not invent a task.

Before launching, append a dated dispatch entry to the carrier naming the subject, the planner's
actor, the dispatching sweep's actor, the UTC time and a recovery contact; a replacement entry
names the earlier entry it succeeds. Launch only after reading the entry back. A refused or
failed write is re-read first, because a write can commit and still report failure: entry
present, the dispatch stands; absent, no launch, and a retry appends on the same carrier.

The planner's first act on the subject is a dated start receipt citing the dispatch entry, with
its actor and session; on stopping it appends a dated outcome citing that entry. A spec author
names the spec PR or the failure; a design planner names the published design and acceptance and
the phase it left, or the failure. A planner that cannot write after retrying stops and reports
through the recovery contact rather than working invisibly; whoever stops a planner records its
outcome.

A subject that is pending, started, unresolved, or completed with its spec PR not yet merged
accepted is in flight: dispatch nothing, and append a dated skip entry citing the carrier ID and
the deciding entry. A failed skip append is retried, then dropped; it bears no state.

Only entries on the carrier move a dispatch between four states: pending launch, started,
completed and positively dead; the owning dispatch is the earliest live entry in native history
order. Death evidence is only a planner final report, a dispatcher-observed launch failure, or a
supervisor-observed termination, each naming observer, observation and time. Silence, elapsed
time, a missing branch, worktree or PR, failed forge queries and timeouts prove nothing: a sweep
that finds only absence reports the subject unresolved to the recovery contact and records that
report on the carrier.

After a positively dead entry a sweep appends exactly one replacement citing it, then launches;
where a native claim exists it is recovered through spec016 before claiming.

The launcher's lock covers one command, so two sweeps can both record a dispatch. Of two live
dispatch entries the earliest owns: the later dispatcher retracts its own entry first —
retrying, then escalating, a failed retraction — and stops its planner only once the retraction
is recorded. Of two carriers naming one intent, or two tasks filed for one draft spec, the
earliest-created is the carrier: the moved dispatch and its entries reappear on the survivor as
dated restatements citing the closed record, the duplicate closes only after every restatement
is observed there, and classification reads the survivor alone; a partial transfer is in
flight, never half-free.

These guarantees hold for one clone's linked worktrees; nothing here prevents simultaneous
dispatch or fences independent clones.

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
has not proved that fix. Record blockers against the actual phase under spec016, not as a
weakened acceptance condition or a claim that the deliverable is finished.
