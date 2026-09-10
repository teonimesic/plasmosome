---
name: heartbeat
description: Advance approved project goals by reconciling current work, resolving prerequisites and dispatching useful deliverables. Use at session start and when choosing work.
---

# The heartbeat

The main driver advances the project toward working capabilities. Reconciliation supplies evidence
for that work; a successful sweep is not itself a delivery. Select across the whole approved
intent/spec/task graph, not a remembered set of task IDs or only the currently ready queue.

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

Use code and delivery evidence to find useful unfinished behavior under approved intents,
including intents that already have specs and accepted specs that already have tasks. A link is
not evidence that the wanted behavior exists. For missing specs, draft specs and incomplete
plans, identify the next finishable prerequisite and dispatch its author when ownership and
capacity permit. New task filing follows the tasks skill; do not fabricate a goal or mapping.

Distinguish an implementation blocker from a blocker on that prerequisite. A missing accepted
spec blocks code, not writing and reviewing the spec. Technical design choices belong to the
planner; spec acceptance follows `docs/specs/README.md`, not an invented owner approval gate.
After an investigation finishes, advance its result into the needed spec, executable plan or
implementation. Do not repeat the investigation, or treat its assignment's limited write scope
as a permanent prohibition on the next assignment.

For a genuine external or owner-only blocker, name the exact missing decision or evidence and
who supplies it, then consider unrelated approved goals. Preserve unresolved ownership; it
blocks taking that work, not investigating every other non-overlapping opportunity. Proposals
needing intent approval follow the intent README's draft-PR gate. A recorded refusal is not an
approval. If the owner has asked not to be questioned, record the decision needed without
prompting them and continue work within existing authority.

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

## 6. Explore the product when planned work runs out

When no eligible planned task can be dispatched, and existing authors and reviews are accounted
for, try a bounded real development workflow before concluding there is no useful work. Build
Plasmosome, run its actual commands and services, and compare the result with its documentation
and approved goals. Explore performance, security, documentation and usability through concrete
scenarios, not just source inspection or another queue scan. These are investigation directions,
not a requirement to repeat an exhaustive checklist on every sweep.

Use common software-development needs to choose scenarios and candidate plasmids: workspace and
local tools, compilation and tests, Git, and API/MCP integrations. Progressively try using
Plasmosome to develop itself, including OMP, Claude Code, Codex or another supported harness
inside a cell. First establish that the required cell runtime, workspace, toolchain, connectivity
and credential boundaries exist. A host process is not a cell, a declaration is not a working
adapter, and launching a harness does not prove isolation or revocation. If a prerequisite is
missing, capture the earliest actual refusal and the next needed capability; do not simulate
success or bypass the boundary to claim dogfooding.

Keep experiments bounded and reversible: owned temporary workspaces and processes, explicit
time/resource limits, synthetic credentials where sufficient, and verified cleanup. Do not
touch another author's work, production endpoints or real credentials without the necessary
authorization. Record the revision, configuration, commands, expected and observed behavior,
measurements and limits. Distinguish a defect, an unsupported capability and an untested idea;
a quick timing sample is not a performance guarantee.

Check native records before filing findings. Add new evidence to an existing issue rather than
duplicate it; file concrete new work through the task skill's spec/intent admission rules.
Propose missing authority instead of inventing approval. Rank development-plasmid and dogfooding
gaps by the workflow they unblock. Do not repeat a known experiment without a changed input,
implementation or question, or create speculative tasks merely to keep the queue populated.

Before reporting no safe action, account for agent-resolvable prerequisites as well as ready
implementation work across approved goals. An empty ready list or unchanged blocked labels is
not sufficient. Repeated idle sweeps while such work remains are a work-selection failure to
investigate, not proof the driver is healthy. Do not manufacture documentation or tasks to
appear busy; report actual behavior proved, reviewed contracts and merged delivery separately.

If fewer candidates can run, record the concrete ownership, admission, dependency or review
constraint. Persist changed decisions, blockers and newly admitted work in Beads; do not append
the same idle note each sweep. If no safe action remains, stop this sweep. A user summary is not
a substitute for native records. Explicitly publish when intended: an ordinary task mutation
never claims to have backed up or synchronized the store.
