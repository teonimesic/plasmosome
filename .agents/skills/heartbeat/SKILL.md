---
name: heartbeat
description: Advance approved project goals by reconciling current work, resolving prerequisites and dispatching useful deliverables. Use at session start and when choosing work.
---

# The heartbeat

The main driver is an orchestrator: reconcile evidence, assign agents, monitor delivery and
resolve cross-task constraints. Delegate planning, implementation, experiments, validation and
independent review to agents under `.agents/skills/planning-work`; do not become their executor.
Select across the whole approved intent/spec/task graph, not a remembered set of task IDs.

Finish work already in flight before starting more. The task queue is shared native Beads,
accessed only through `./tools/work-state`; the contents of a chat, branch or old task snapshot
cannot replace it. `.agents/skills/tasks` describes the records and operations.

Local reads do not synchronize. If this session needs an updated remote replica, explicitly
coordinate `./tools/work-state dolt pull` before relying on it. A successful local read is not
proof of remote freshness, and an unreachable remote is not an empty queue. Do not dispatch from
an independent writer clone as though it shared the local claim lock; see spec016.

## Execution health: measure, intervene, verify

Each sweep records its observation time, source revisions and completeness, then measures:

- **Delivery:** actual GitHub PRs observed `MERGED` with a merge commit and `mergedAt` in
  `(observation time - 1 hour, observation time]`. Enumerate all matching pages; do not use PR
  creation, branch deletion, task closure or `updatedAt`. Report closed-unmerged PRs separately
  by `closedAt`, the last actual merge, and elapsed time since it. Failed reads make the metric
  unknown, never zero or healthy.
- **Work and ownership:** complete native counts by stored status, admitted eligible backlog,
  live agents by role, actual author assignments, persistent claims, and open PRs split by draft
  and ready. Join task ID, actor, agent, worktree and PR; show missing links and disagreements.
  Count planning work separately from implementation and review, including active work whose
  native status is wrong. Report the mismatch; do not silently recategorize the stored count.
- **Age:** for each nonclosed task report its current-status age and source under
  [spec016's history contract](../../../docs/specs/016-native-beads-task-authority.md#current-status-age).
  Preserve exact, lower-bound and unknown results. `updated_at`, notes and unchanged history
  snapshots are not progress and must not reset the clock. Missing age evidence is an
  observation fault to assign, not a young task.
- **Stages:** identify each active task/PR's current wait and elapsed time from its actual start
  evidence: assignment/start acknowledgement, settled candidate, validation process, PR creation
  and ready event, independent-review dispatch/result, provider admission/completion, CI and
  merge readiness. Keep total status age distinct from the current attempt's age. Record source
  timestamps and unknown intervals; overlapping work is not summed into elapsed time.

Show the oldest work and slowest stages with the observed cause, accountable agent, next concrete
action and a timestamped checkpoint. Separate admission/ownership, agent execution, orchestration
handoff, local validation, CI, independent review and provider waits. An hour-long review with
seconds of measured probes is not evidence that tests took the hour; identify the unmeasured
remainder. Queued, skipped and rate-limited review signals do not satisfy completed review.

The owner's delivery floor is **at least one merged PR per trailing hour**. Below it is a
**pipeline bug**, even when an external service limits capacity, not a normal blocked summary.
Reuse the existing open native pipeline bug for the continuing incident; otherwise file one
through the tasks skill. Assign an agent a concrete intervention against the limiting stage,
record its evidence and checkpoint in that bug, and continue independent useful work. At the
checkpoint compare the result with the original failure and measure delivery again. Missing a
checkpoint requires a revised action or escalation with cause, not another identical idle note.
If nothing authorized can remove an external blocker, retain the bug and name the precise
decision/evidence needed; do not call that recovery.

An intervention is not successful merely because a note, plan or PR was created. Keep the bug
open until its change has passed the full observed merge gate, the limiting-stage correction
has been exercised, and a later sweep observes the delivery floor. Record those recovery
observations; a later missed window is a new failure to reconcile, not permanently healthy
history. Never close unwanted PRs, weaken review or invent approved work to improve the number.

### Recommend capacity from measured flow

Use the [Kanban Guide's flow measures](https://kanbanguides.org/the-kanban-guide/2025.5/#flow-metrics):
work started but unfinished, throughput, age and completed elapsed time, with explicit boundaries.

The heartbeat's LLM orchestrator makes the allocation decision; no fixed author count, per-status
WIP limit, ready-reserve size or stage timer is enforced here. Keep admission, atomic ownership
and review gates hard. Recommendations adapt to the delivery goal, observed flow and available
agents. State the evidence and uncertainty behind each decision, rather than filling idle agents
or treating a calculated average as an instruction to start more work.

Choose and report an observation window and comparable work classes. For each native status,
measure time-weighted WIP and completed visits: sample count, mean elapsed time, a stated tail
percentile and maximum. Include waiting and blocked ownership in committed WIP; separate active
service from waiting only where timestamps support it. Track re-entry/rework visits and total
residence per task without treating repeated snapshots as independent samples. Show current ages
and right-censored visits separately; unfinished work is not a fast completion. Report missing
history and the population excluded, not just the successful sample.

Use spec016's status-history evidence rules. Before the new statuses have usable observations,
their sample count is zero, not the old `in_progress` duration relabeled as planning or review.
Use actual dispatch/acknowledgement/finished notes, process events and GitHub timestamps as
separate, explicitly named proxy cohorts when their boundaries are known. PR creation-to-merge,
an independent-review attempt, and native review residence are different measurements. Preserve
their sample sizes, censoring and comparability limits; do not pool them to manufacture precision.
Capture real future transitions and results in native evidence so missing history does not become
a permanent excuse for not learning.

[Little's Law](https://web.mit.edu/urban_or_book/www/book/chapter4/4.4.html) supplies a baseline,
not a WIP controller: for a stable, consistently bounded flow,
`mean occupancy = throughput × mean elapsed time`. To reason per status at the target merge rate,
use `target merges/hour × observed visits to that status per merged PR × mean hours per visit`.
Measure the conversion: tasks, taskless spec/intent PRs, revisits and merges are not interchangeable.
Do not round this result into compulsory slots, or increase WIP because congestion lengthened the
cycle time. Check the limiting service first: available service capacity divided by service demand
per merged PR bounds sustainable throughput. More authors cannot fix insufficient review allowance.

Show the historical throughput distribution, age/tail risk and a next-window delivery outlook,
including the unfinished queue, review demand, rework and known external waits. State assumptions,
sample sizes and forecast uncertainty. An empirical percentile is not a confidence guarantee;
one merge/hour on average does not guarantee a merge in every rolling hour. Sparse, censored or
changing-policy data calls for a provisional recommendation and explicit next observations, not
an invented probability or a claim that the owner's delivery floor was met.

For each status, record **observed WIP → recommended allocation → orchestrator decision**, with
the source cohort, limiting cause, assigned agent action and next evidence checkpoint. Compare
actual results with that recommendation on the following sweep and revise it. Choose checkpoints
from task progress, relevant historical elapsed-time/age distributions and actual provider events,
not universal countdowns. Collect settled agents/processes at handoffs before waiting on unrelated
work; unchanged notes do not establish advance.

Pull eligible work when an author and downstream capacity can use it; otherwise prioritize
finishing, reviewing or unblocking committed work. Replenish approved plans when expected
consumption over the observed planning lead time threatens to exhaust eligible work, accounting
for variability and uncertainty rather than a fixed reserve. Missing data does not justify idle
sweeps: assign a bounded approved prerequisite or missing measurement while preserving existing
ownership. Persist changed recommendations, decisions and results in native notes, not a new
scheduler, dashboard, timer or task store.

### Provider capacity

Use current PR status histories, provider comments and accessible read-only usage/configuration
evidence. For each pending PR report required rounds from `.agents/skills/pr-review`, actual
completed rounds, missing current-head coverage and remaining demand. Keep the minimum-round
shortfall distinct from a further review needed after a repair. Compare aggregate pending demand
and the demand needed for hourly delivery with the allowance actually observed for the author
identity; do not assume one author lane is one provider slot.

Distinguish the owner's plan entitlement, effective allowance/refill, admitted reviews and
completed reviews. CodeRabbit's [rate-limit documentation](https://docs.coderabbit.ai/management/rate-limits)
describes per-developer rolling and adaptive limits, not a universal repository quota. A nominal
ten-per-hour plan is not proof of ten available now; a one-per-hour diagnostic is not proof every
identity has that limit. Without account-specific evidence the cause of the difference is unknown.
Rate-limited pushes consume no review and do not delay refill; completion time plus an hour is
not a reset calculation.

Delegate read-only diagnosis of a capacity mismatch, batch settled repairs before publication,
and overlap eligible independent review/validation across disjoint work. Requests and merge gates
remain in the review skill. Do not rotate identities to evade limits, change billing/admin
settings, buy capacity, waive rounds or hold validated work solely on an invented global quota.
If observed allowance cannot sustain required review demand, record that constraint and its
owner-supplied decision separately from agent-removable delays; the pipeline bug remains open.

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
./tools/work-state list --all --limit 0 --json
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

Apply the current evidence-based allocation decision above. Compare planned write sets, not only
`metadata.refs` (references are reads); preserve blocked claims and uncertain ownership even
when delivery is below target. Review backlog and provider evidence constrain new execution without
turning unrelated planning into forbidden work.

## 5. Pick and dispatch

```shell
./tools/work-state ready --label planned
```

Take eligible planned work before planning new work, lowest priority number first. For every
candidate, read `show ID` and recheck the full plan, acceptance, accepted specs and approved intent
chain. Native ready is dependency eligibility, not a spec validator. Send each executor the full
Beads ID and its non-overlapping ownership; it uses its unique actor and atomically claims before
creating a code worktree. A losing claimant stops, not a second implementation.

## 6. Replenish approved work continuously

When the measured flow calls for replenishment, account for existing authors and reviews
and delegate the next useful approved prerequisite. If existing evidence does not settle what
to build, assign an agent a bounded real development workflow: build Plasmosome, run its actual
commands and services, and compare behavior with its documentation and approved goals. Explore
performance, security, documentation and usability through concrete scenarios, not another
queue scan. Reuse known proof; this is not an exhaustive checklist to repeat each sweep.

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
