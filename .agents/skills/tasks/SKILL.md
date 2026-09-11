---
name: tasks
description: Find, file, plan, claim and close tasks in shared native Beads. Use for every task operation; intents and specs remain versioned documents.
---

# Finding, filing and closing work

All task content and coordination live in shared native Beads: descriptions, plans, acceptance,
notes, evidence, status, dependencies and ownership. Use `./tools/work-state`, never task Markdown,
branch copies, a chat transcript or a tracked export. The storage and native API contract is
[`docs/specs/016-native-beads-task-authority.md`](../../../docs/specs/016-native-beads-task-authority.md).

Intents and specs remain versioned Markdown in `docs/intents/` and `docs/specs/`. The repository's
vision and architecture stay in the root and crate docs; decisions stay in `docs/decisions/`.
A decision can be a task reference but cannot substitute for its governing spec.

Start a session with `.agents/skills/heartbeat`. For a task you have been given, read it by its
complete Beads ID with `./tools/work-state show ID`; never infer its current contents from a PR
body or a worktree's old files.

## Admission and filing

Every work PR names a Beads task, whose `metadata.spec_ids` names existing specs. Each spec names
its intents. The only taskless PR shapes are those filing an intent (the top of the chain) or a
spec (which still names its parent intent). This closed list belongs to spec012; no size or kind
of change is exempt, and an area with no spec is not a third shape.

Mapping to an existing spec is normal. A new task must name a spec, even if it is still draft.
If no spec reaches the work, propose one under an existing intent; if no intent wants it, put a
draft intent to the owner or drop the work with the reason recorded where the question arose.
Do not populate the queue with unmapped new work. Imported unmapped tasks retain their history,
but cannot become planned or be claimed until mapped. Correcting their notes does not require
inventing a mapping.

The approval and acceptance gate is in `docs/intents/README.md` and `docs/specs/README.md`.
A spec needed for implementation lands accepted before the implementation claim or transition
and before its code branch opens. A planning claim against a draft spec authorizes design work
only, under spec016; it does not admit implementation.

File a persistent native issue, retaining the returned ID:

```shell
./tools/work-state create --type task --title 'Short deliverable' --priority 2 \
  --labels needs-plan --description 'Why this is needed' \
  --acceptance 'Observable completion condition' \
  --metadata '{"spec_ids":["012"],"intent_ids":["008"],"refs":[]}'
```

Replace the example with the actual links and contents. Native IDs are hash-based; do not number
new tasks from Git branches or request a legacy ID. `plasmosome-NNN` identifies an imported task.
Spec and intent IDs remain three-digit strings in their separate Git namespaces. Their templates
remain in `docs/templates/`; there is no task template file.

Use native `update ID --description`, `--design`, `--acceptance` and `--append-notes` for task
content. `--body-file`, `--design-file` and `--metadata @file.json` can consume temporary input;
such inputs are not another authority and are not committed. Preserve migration metadata when
changing links: pass only changed keys with `--metadata '{"intent_ids":["002"]}'`. Native update
merges those keys, preserving unrelated metadata and JSON value types. Do not use `--set-metadata`
for link arrays: Beads 1.1.2 stores that input as a string rather than an array.
For commands that address existing tasks by ID (such as `show`, `update`, and `close`), pass each
complete Beads ID rather than relying on the native last-touched default. Creation, collection,
automatic-selection, and dependency commands follow their native operand and arity rules.

## Planning and lifecycle

The lifecycle and label meanings are defined once in
[spec016](../../../docs/specs/016-native-beads-task-authority.md#lifecycle-admission-and-ownership).
Use its actual native phases, not historical review labels. Store configuration and existing
record migration require its coordinated cutover; do not silently configure production during
an ordinary task assignment.

For a planning assignment, first read the whole record, resolve its mapped specs/intents and
identify which prerequisites constrain planning rather than implementation. Record the bounded
planning assignment, then claim an unowned open task with your unique session actor:

```shell
export BEADS_ACTOR='planner-name-unique-session-id'
./tools/work-state update ID --claim --status planning
./tools/work-state show ID --json
```

Check both your assignee and final `planning` status before writing the plan. Follow spec016's
partial-claim recovery if the command fails; do not assume failure released ownership. For a
task you already own, enter planning with `update ID --status planning`, retaining prerequisites.
Active design work is not blocked simply because implementation cannot yet start. If planning
itself cannot advance, record the cause and resume phase and use `blocked`.

A plan in `design` must let a stranger execute without a conversation: the deliverable and why,
non-goals, exact relevant references, design decisions and API shape, verification cases and
what each proves, platform constraints, and the definition of done including the root gate.
Keep acceptance in `acceptance_criteria`, rather than relying on a checklist in a message.
End the assignment at this deliverable, not the next task.

Before making a task eligible, read every `metadata.spec_ids` entry at accepted `main` and all
intents they name. Each ID must resolve uniquely; accepted specs must name approved intents.
`metadata.intent_ids` is the first-seen ordered union of those spec intent lists. Repair a stale
copied list; it is not a separate approval gate. Record the checked revisions in notes. For a
handoff, the current planner publishes the admitted plan and releases it for an executor:

```shell
./tools/work-state update ID --design 'Complete execution plan' \
  --acceptance 'Observable completion conditions' \
  --remove-label needs-plan --add-label planned --status open --assignee ''
./tools/work-state ready --label planned
```

If the same actor will implement, keep ownership instead of releasing it; after rechecking the
implementation gate and prerequisites, publish the plan with `planned` and set `in_progress`.
A drafted plan still awaiting spec acceptance is not `planned`: record the remaining admission
work, retain `needs-plan`, and use `blocked` when no planning can advance.

Native ready does not read repository specs or decide whether a plan is adequate. The planner
checks admission, and the executor rechecks it before implementation. A listed row is a candidate, not
permission to bypass those checks.

`planned` records an admitted plan, not an exclusive lifecycle state. Keep it while claimed;
native ready selects open candidates, not active phases or closed tasks. Remove it only if the plan or
admission ceases to hold.

## Ownership, dependencies and recovery

Set a unique actor for this executor session, not a shared Git username. For implementation,
even when dispatched directly by ID, read `show ID`, check its `planned` label and governing
main documents, and inspect its native prerequisites with `show ID` and `blocked`. Native ready
filters dependency blockers; native claim does not check dependency edges. Claim only eligible
implementation work before creating its code worktree or executing it:

```shell
export BEADS_ACTOR='agent-name-unique-session-id'
./tools/work-state update ID --claim
```

`--actor` is the explicit per-command alternative. The native claim atomically sets assignee and
`in_progress`; another actor loses and does no work or dispatch for that task. Read back both.
Owners resume or change phase with ordinary status updates, not a re-claim from a custom status.
The executor claims for itself; an orchestrator dispatches a candidate ID, not an already-claimed
task under the orchestrator's identity.

The launcher refuses native `close --continue` and `--suggest-next`, including `--no-auto`:
the pinned implementation can advance without ownership or skip history auto-commit.
Close normally, then inspect eligible next work. Before starting it, repeat the
admission/dependency checks above and explicitly claim its complete ID. Inspection does not
reserve the task. Ordinary JSON close remains available.

Use `./tools/work-state dep add CHILD PREREQUISITE` for actual blocking dependencies: CHILD waits
for PREREQUISITE. Use `blocked` and `show ID` to inspect them. Spec016 distinguishes active planning
despite implementation prerequisites from genuinely blocked work. Keep both real dependencies
and ownership; record the actual phase to resume before setting `blocked`. When it clears, the
existing owner resumes that phase, or an explicitly released task returns to `open`.

Claims persist across sessions and do not expire automatically. Before recovering another
actor's claim, establish that its owner is no longer working, inspect its PR on GitHub first,
and preserve the reason and observations in notes. A missing branch, silent agent or failed
network query alone does not establish abandonment. The author owns this work; the orchestrator
reconciles only when the author is gone. After confirming release, clear assignee with
`update ID --assignee '' --status open`; retain `planned` only if its
plan and admission still hold, otherwise replace it with `needs-plan`. Do not clear a historical
PR reference without first preserving it in notes/metadata.refs.

## Review, closure and durable evidence

The author enters `review` for a settled candidate under spec016 and records its actual PR
with `update ID --external-ref PR_URL --status review`, retaining ownership. Task notes contain
verification results, abandoned approaches,
review evidence references and anything the next agent would otherwise rediscover. PR review
threads remain on GitHub; link them instead of transcribing or treating chat as evidence.

`.agents/skills/pr-review` owns review and the merge gate. After GitHub reports `MERGED` with a
squash commit, record that PR URL, commit and observed merge time in notes, then run:

```shell
./tools/work-state close ID --reason 'Merged PR_URL at SQUASH_SHA'
```

A successful merge command, deleted branch or closed-but-unmerged PR is not proof.
Cancellation may close a task with an explicit cancellation reason, never as delivered work.

Measure time in native phases with
[spec016's current-status age procedure](../../../docs/specs/016-native-beads-task-authority.md#current-status-age).
Use the native transition history and report exact, lower-bound or unknown evidence honestly;
appending notes does not restart a status stay. Age never releases an owner or waives review.

Task creation, planning, status changes and closure never need commits, branches or status-only
PRs. Beads is the task record; GitHub is the evidence for forge facts. An ordinary local mutation
is not a remote backup: explicit publication and its limitations are in spec016.

Priority retains the project's meaning: 1 unblocks other work, 2 a defect or next capability,
3 nothing depends on it. Choose the lowest number among eligible work, not the newest task.
