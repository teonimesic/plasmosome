---
name: heartbeat
description: Reconcile PRs and shared Beads ownership against evidence, then dispatch eligible planned tasks. Use at session start and when choosing work.
---

# The heartbeat

Finish work already in flight before starting more. The task queue is shared native Beads,
accessed only through `./tools/work-state`; the contents of a chat, branch or old task snapshot
cannot replace it. `.agents/skills/tasks` describes the records and operations.

Local reads do not synchronize. If this session needs an updated remote replica, explicitly
coordinate `./tools/work-state dolt pull` before relying on it. A successful local read is not
proof of remote freshness, and an unreachable remote is not an empty queue. Do not dispatch from
an independent writer clone as though it shared the local claim lock; see spec016.

## 1. Resume PRs and reviews

List open GitHub PRs, including drafts. Resolve each work PR's Beads ID using `show ID` and follow
all its spec and intent links. Spec012 defines the only taskless shapes. A missing task or an
invalid chain is work to return to the author, not permission to merge.

Apply `.agents/skills/pr-review`: check actual current-head review evidence, CI and every thread,
including findings in review bodies and issue comments. Read all pages; failed or partial queries
are unknown, not a clear queue. Resume the original author to answer and resolve findings.
A draft waiting on owner intent approval is correctly waiting on a person, not a stalled author
to prod into marking it ready.

## 2. Reconcile existing claims

```shell
./tools/work-state list --status in_progress --limit 0
./tools/work-state list --status blocked --limit 0
./tools/work-state list --label in-review --limit 0
```

Inspect each relevant record's assignee, notes, dependencies and `external_ref` with `show ID`.
Ask GitHub about its PR **before** interpreting a missing branch: squash merging deletes a
branch without making its old tip an ancestor of main. `gh pr view NUMBER --json state,mergeCommit,mergedAt,url`
is the forge observation; only `MERGED` with a merge commit is delivery evidence.

The author records merge evidence and closes in Beads. If it is gone, the orchestrator performs
that same reconciliation, recording what established the result. Do not make a task-status
commit or closure PR. A closed-but-unmerged PR is not done. Release a claim only through the
confirmed abandonment procedure in the tasks skill; uncertainty about the owner is not release.

## 3. Inspect planning gaps and the governing documents

```shell
./tools/work-state list --label needs-plan --limit 0
./tools/work-state blocked
./tools/work-state list --all --limit 0 --json
```

Use native metadata to find missing spec mappings and specs with no task; never search task
Markdown. Enumerate all records rather than mistaking a default-limited list for the whole queue.
Compare with the numeric documents in `docs/specs/` and `docs/intents/`. Missing mappings in
legacy records are planning work, not reason to fabricate wanted goals. A missing copied
`intent_ids` union is repairable metadata, not a second approval gate.

Read document frontmatter as records before selecting on status. Spec012 requires uniquely
resolving IDs and well-formed state declarations for every numeric intent and spec; spec and
intent READMEs define their legal sets. A missing/empty document set, malformed or duplicate
status, missing link target, or accepted spec reaching no approved intent is a fault, not an
empty planning queue. Report each fault rather than hiding it with a status selector. Task states
and contents come from native rows, never a Markdown state-line sweep.

For a draft spec with no task, or an approved intent with no spec, decide whether it needs a
planner or why not. An unanswered draft intent goes to the owner as a draft PR. Drafts with a
recorded refusal/outcome are not forgotten approvals. New task filing follows the mapping rule
in the tasks skill; noticing something during review is not enough to admit it.

## 4. Establish actual capacity

Inspect `git worktree list --porcelain` and communicate with live authors. Worktree existence and
PR state alone do not establish liveness. Classify known work as active, finished, confirmed
abandoned, or unresolved; persist relevant ownership/recovery observations in Beads notes.
Unknown, detached, closed-PR and unreachable cases stay unresolved until evidence settles them.
Do not count unresolved work as spare capacity.

Only remove a finished clean worktree after its owner is done; preserve old or uncertain trees
and databases. Remove by actual path, not an inferred branch name. Reconcile any native owner
with no active author, and any active author with no claim, before dispatching over them.

Three independent authors is the standing target when the queue, file ownership and review
throughput permit it. Reviewers do not own implementation slots. Compare planned write sets,
not only `metadata.refs` (references are reads). Do not invent overlapping work to fill slots.
CodeRabbit throughput is repo-wide, historically about ten rounds an hour; account for actual
review usage rather than treating three authors as a promise of three review slots.

## 5. Pick and dispatch

```shell
./tools/work-state ready --label planned
```

Take eligible planned work before planning new work, lowest priority number first. For every
candidate, read `show ID` and recheck the full plan, acceptance, accepted specs and approved intent
chain. Native ready is dependency eligibility, not a spec validator. Send each executor the full
Beads ID and its non-overlapping ownership; it uses its unique actor and atomically claims before
creating a code worktree. A losing claimant stops, not a second implementation.

If fewer candidates can run, record the reason: empty queue, blockers, missing plans, conflicting
files or review capacity. Persist task decisions, notes and any newly admitted work in Beads
before the session ends; a summary to the user communicates those records and is not their
replacement. Explicitly publish when intended: an ordinary task mutation never claims to have
backed up or synchronized the store.
