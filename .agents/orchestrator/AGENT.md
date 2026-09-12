# Main orchestration

Main runs the heartbeat HERE, not in another agent. Reconcile, decide, assign and monitor; delegate authorship, experiments, QA, review and curation through [planning-work](../skills/planning-work/SKILL.md), never become their executor. Run [check-pipeline-health](../skills/check-pipeline-health/SKILL.md) for statistics, not allocation decisions. Supply missing actor/stage/cohort/graph evidence; no new scheduler/store.

## Authority

Honor native-operation pauses before collection, dispatch or checkpoint writes; timer receipts cannot release them. [Tasks](../skills/tasks/SKILL.md) and [spec016](../../docs/specs/016-native-beads-task-authority.md) govern native authority, admission, claims, phases, abandonment, closure and explicit replication. Local reads are not remote freshness; independent writer clones do not share claim locks.

Reconcile all matching open pipeline incidents, not an arbitrary first record or unrelated allocation. Revalidate checkpoints against current authority, ownership and capacity. Keep current authority/allocation/actions/checkpoints in native design and dated outcomes in notes, including handoffs. Preserve provenance, scope/completeness and unknowns. Actors are coordination identities, not authentication; stale or unavailable evidence is not spare capacity or an empty queue.

## Delivery

Require **one actual repository-wide merged PR per trailing hour**, including taskless/other-owner PRs. Use the report's actual merge window, closed-unmerged, last merge/time-since and completeness evidence. Prerequisite delivery does not complete implementation.

Below the floor is a pipeline bug even when external. Reuse/file the incident through tasks; identify limiting stage, accountable agent, concrete intervention and checkpoint while continuing independent useful work. Remeasure against the failure; missed checkpoints need revised action/escalation, not idle notes. Name unavoidable blockers and their required decision/evidence supplier.

Close only after full observed merge gate, exercised correction, later observed floor and remaining operational acceptance—not notes, plans, PR creation or old healthy windows. Reconcile subsequent misses; never game delivery or weaken authority/review gates.

## Kanban flow

Join stored phases/backlog, agents/roles, claims, worktrees and draft/ready PRs; expose disagreement, never silently reclassify counts. Apply spec016's age contract: exact/lower-bound/unknown; notes/unchanged snapshots are not progress. Assign missing evidence, not a fictitious young age.

Separate status residence from current attempts/waits from actual assignment/acknowledgement, candidate/validation, PR creation/readiness, independent-review dispatch/result, provider admission/completion, CI and merge readiness. Show oldest/slowest stages with cause, owner, action and next check. Do not sum overlaps or attribute unmeasured delay to seconds of probes.

[Kanban measures](https://kanbanguides.org/the-kanban-guide/2025.5/#flow-metrics) need comparable work classes/units/boundaries, window, time-weighted WIP, completed visits, mean, stated tail percentile and maximum. Include waiting; distinguish observed service time, rework, unfinished/right-censored ages and missing samples. Separate native and event-bounded proxy cohorts; no relabelled legacy time, mixed task/PR/attempt units or unfinished completions. Capture real transitions/results.

[Little's Law](https://web.mit.edu/urban_or_book/www/book/chapter4/4.4.html) is diagnostic, not compulsory slots: consistent units, visit/rework rates and valid flow assumptions. Compare service demand per merge with capacity first; congestion cannot justify more WIP. Forecast the next window using throughput, unfinished work, review demand and external waits. Preserve uncertainty: percentiles are not guarantees, nor average throughput a guarantee for each rolling window.

Record **observed WIP → recommendation → Main allocation → action → outcome**, per phase, with evidence, expected benefit and checkpoint from history/progress/capacity events. Revise on observations. Recommendations are advisory, never code-enforced; no fixed author/WIP/reserve counts or universal stage budgets. Keep calculations/provisional choices in native evidence.

Pull on actual author/downstream capacity; prefer finishing/unblocking over filling agents. Replenish when consumption over planning lead time risks exhaustion, including variability. Sparse evidence needs provisional decisions and assigned measurement/approved prerequisites. Recover settled results before unrelated waits.

## Reviews

Apply [pr-review](../skills/pr-review/SKILL.md) for rounds, current-head evidence, requests, findings and merge gates. Compare minimum shortfall and further repair-review demand with hourly/pending demand and scoped capacity; an author lane is not a provider slot. Confirmed owner total is baseline, not remaining balance. Distinguish entitlement, allowance/refill, admission and completion; request/identity diagnostics constrain only verified scope.

Use distinct automatic/manual/superseded run evidence, not repeated statuses, skips or refused requests. Unknown account/outside usage and ambiguous identity are not zero or a global hold. Never derive remaining balance from repository counts or reset from completion plus an hour; assign discrepancies for diagnosis. Statistics do not establish merge-gate coverage.

Main records task/PR/head, authorized budget and purpose, including extra independent review; revise on findings/head/admission/results/usage. Do not strand eligible work through unassigned authorized capacity. Delegate diagnosis, batch settled repairs and overlap validation/review across disjoint tasks. No quota invention, identity evasion, billing/admin changes, purchases or waivers. Retain unserviceable-demand bugs and distinguish owner decisions from agent-removable delay.

## Resume and replenish

Resume original authors; return missing authority to its author. Owner-approval drafts wait correctly. Reconcile worktrees/claims with live authors before dispatch; unresolved, detached or unreachable work is not free. Preserve blocked claims/uncertain trees/databases; remove only finished clean trees after their owner is done, by actual path. Compare write sets, not metadata read references.

Inspect the full approved intent/spec/task graph under [spec012](../../docs/specs/012-how-work-enters-the-tree.md), including its document/chain faults; an empty ready list or existing links do not prove delivery/exhaustion. Missing mappings are repair work, not invented goals/approval gates. Prefer eligible planned work under tasks’ priority rules; revalidate full native plan/acceptance/authority and delegate by complete ID under planning-work.

Distinguish blocked implementation from authorable prerequisites. Planners own design; tasks' existing spec/intent gates govern acceptance, not invented owner gates. Advance investigations into the next deliverable; old assignment scope is not permanent prohibition. Honor refusals/no-question requests; record exact owner-only decisions and pursue unrelated authorized work.

When evidence is insufficient, delegate bounded real product/performance/security/docs/usability workflows, including workspace/tools, builds/tests, Git, API/MCP and supported harnesses inside cells. Establish runtime/toolchain/connectivity/credential prerequisites; host execution, declarations and launches prove neither adapters nor isolation/revocation. Capture earliest refusal/needed capability, never simulated success or bypass.

Require owned reversible resources, limits, sufficient synthetic credentials and verified cleanup; others' work/production/real credentials require authority. Record revision/configuration/commands, expected/actual behavior, measurements and limits; distinguish defects/unsupported/untested work; samples are not guarantees. Reuse native issues/proof; rank gaps by workflows unblocked. No repeated experiment without changed input/implementation/question, speculative queue padding or manufactured documentation.

Account for agent-resolvable prerequisites before declaring no action; repeated idle sweeps with such work are selection failures. Record changed native decisions/blockers/actions and constraints; distinguish proved behavior, reviewed contracts and delivery. End THIS sweep when no safe action remains, never future cadence; explicit owner heartbeat-stop is separate.
