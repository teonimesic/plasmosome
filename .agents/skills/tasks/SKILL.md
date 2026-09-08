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
A spec needed for implementation lands accepted before that work is claimed or its code branch
opens. Filing a task against a draft spec does not authorize starting it.

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
changing links: use key-level `--set-metadata` rather than replacing the whole metadata object.
For commands that address existing tasks by ID (such as `show`, `update`, and `close`), pass each
complete Beads ID rather than relying on the native last-touched default. Creation, collection,
automatic-selection, and dependency commands follow their native operand and arity rules.

## Planning and lifecycle

The lifecycle and label meanings are defined once in spec016. In particular, `planned` is an
**admitted-plan label**, not a custom status. Beads 1.1.2 accepts a custom planned status but
its native ready and claim paths do not support it. Use the native lifecycle, not old `todo`,
`in_review` or `done` status values.

A plan in `design` must let a stranger execute without a conversation: the deliverable and why,
non-goals, exact relevant references, design decisions and API shape, verification cases and
what each proves, platform constraints, and the definition of done including the root gate.
Keep acceptance in `acceptance_criteria`, rather than relying on a checklist in a message.
End the assignment at this deliverable, not the next task.

Before making a task eligible, read every `metadata.spec_ids` entry at accepted `main` and all
intents they name. Each ID must resolve uniquely; accepted specs must name approved intents.
`metadata.intent_ids` is the first-seen ordered union of those spec intent lists. Repair a stale
copied list; it is not a separate approval gate. Record the checked revisions in notes. Then:

```shell
./tools/work-state update ID --design 'Complete execution plan' \
  --acceptance 'Observable completion conditions' \
  --remove-label needs-plan --add-label planned
./tools/work-state ready --label planned
```

Native ready does not read repository specs or decide whether a plan is adequate. The planner
checks admission, and the executor rechecks it before claiming. A listed row is a candidate, not
permission to bypass those checks.

`planned` records an admitted plan, not an exclusive lifecycle state. Keep it while claimed;
native status keeps in-progress and closed tasks out of `ready`. Remove it only if the plan or
admission ceases to hold.

## Ownership, dependencies and recovery

Set a unique actor for this executor session, not a shared Git username. Even when dispatched
directly by ID, read `show ID`, check its `planned` label and governing documents, and inspect
its native prerequisites with `show ID` and `blocked` before claiming. Native ready filters
dependency blockers; native claim checks open status and assignee, not dependency edges.
A successful claim therefore does not prove dependency eligibility. Claim only eligible work,
before creating its code worktree or doing work:

```shell
export BEADS_ACTOR='agent-name-unique-session-id'
./tools/work-state update ID --claim
```

`--actor` is the explicit per-command alternative. The native claim atomically sets assignee and
`in_progress`; another actor loses and does no work or dispatch for that task. A claim already
held by the same actor is idempotent, so sharing an actor would defeat competing-claim protection.
The executor claims for itself; an orchestrator dispatches a candidate ID, not an already-claimed
task under the orchestrator's identity.

Use `./tools/work-state dep add CHILD PREREQUISITE` for actual blocking dependencies: CHILD waits
for PREREQUISITE. Use `blocked` and `show ID` to inspect them. Record external blockers in notes
and set `--status blocked`; when they clear, the existing owner resumes with `in_progress`, or
an explicitly released task returns to `open`. Do not erase dependencies merely to make ready
list a task.

Claims persist across sessions and do not expire automatically. Before recovering another
actor's claim, establish that its owner is no longer working, inspect its PR on GitHub first,
and preserve the reason and observations in notes. A missing branch, silent agent or failed
network query alone does not establish abandonment. The author owns this work; the orchestrator
reconciles only when the author is gone. After confirming release, clear assignee with
`update ID --assignee '' --status open --remove-label in-review`; retain `planned` only if its
plan and admission still hold, otherwise replace it with `needs-plan`. Do not clear a historical
PR reference without first preserving it in notes/metadata.refs.

## Review, closure and durable evidence

The author records the PR with `update ID --external-ref PR_URL --add-label in-review` and keeps
ownership throughout review. Task notes contain verification results, abandoned approaches,
review evidence references and anything the next agent would otherwise rediscover. PR review
threads remain on GitHub; link them instead of transcribing or treating chat as evidence.

`.agents/skills/pr-review` owns review and the merge gate. After GitHub reports `MERGED` with a
squash commit, record that PR URL, commit and observed merge time in notes, then run:

```shell
./tools/work-state update ID --remove-label in-review
./tools/work-state close ID --reason 'Merged PR_URL at SQUASH_SHA'
```

A successful merge command, deleted branch or closed-but-unmerged PR is not proof.
Cancellation may close a task with an explicit cancellation reason, never as delivered work.

Task creation, planning, status changes and closure never need commits, branches or status-only
PRs. Beads is the task record; GitHub is the evidence for forge facts. An ordinary local mutation
is not a remote backup: explicit publication and its limitations are in spec016.

Priority retains the project's meaning: 1 unblocks other work, 2 a defect or next capability,
3 nothing depends on it. Choose the lowest number among eligible work, not the newest task.
