---
id: 019
title: A dispatched planner leaves a trace before any push
status: draft
intents: [008]
---

## Behavior

When the orchestrator dispatches a planner — an agent told to plan the implementation of a
draft spec, or to author a spec an approved intent still lacks — the dispatch exists only in
that orchestrator's memory until something is pushed. The next sweep reads the same queue,
finds the same unimplemented spec or unspecced intent, and dispatches a second planner onto
the same subject. This spec makes the dispatch itself a record: before the planner is
launched, the dispatcher writes who was dispatched, for what, when, and who to ask, into the
shared native store that every linked worktree already reads. A planner that has pushed
nothing is still visible, because its visibility never depended on a push.

The record answers, from the store alone, the three questions a later sweep has. Is someone
already on this subject? Then skip, and write down which record said so. Did that planner
finish? Then record the outcome and treat the subject as free again. Did the planner stop
before producing anything? Then name the evidence that proves it, and record a replacement
the same way as the first dispatch. Silence, elapsed time and missing branches prove nothing;
only entries on the record move a dispatch between these states.

This is coordination in the store [spec016](016-native-beads-task-authority.md) already
defines, for work whose chain entry [spec012](012-how-work-enters-the-tree.md) already
governs. It adds no scheduler, lease, fence, daemon, second queue or new status, and grants
no admission: a recorded planner still cannot claim, plan or implement outside the existing
gates. Its guarantees hold for sweeps and agents sharing one clone's linked worktrees;
independent clones and prevention of simultaneous dispatch are outside this contract, as they
are outside spec016's.

## Contract

A **sweep** is one heartbeat's pass over the queue. A **planner** is an agent dispatched to
produce a plan or a spec. Every dispatch names one **subject**: a spec or an intent, by its
three-digit ID. The record of a dispatch is called the **carrier**.

### Subjects and carriers

The carrier is a native record in the shared store, never Git content, a chat transcript or an
orchestrator's memory. An unpushed branch is not a carrier. Which record carries a dispatch
follows one rule: the record of the work the planner will update.

- A design planner plans inside the mapped task, so the task is the carrier. When the sweep
  hits a draft spec no task implements, it first enumerates open tasks for one naming that
  spec, files the task when none exists (filing against a draft spec is valid under
  spec012), then dispatches. The planner's claim into `planning` follows spec016 and is
  itself visible.
- A spec author dispatched because a mapped task lacks its governing spec does not take that
  task's phase or claim; its dispatch is a dated note on the task, which is the carrier.
- A spec author for an approved intent with no spec has no task to anchor on and may not
  invent one: unmapped queue work is refused under spec012. Its carrier is a standalone
  native record of type `chore`, labelled `planner-dispatch`, carrying the intent in
  `metadata.intent_ids`. It never becomes queue work — no `needs-plan`, no `planned`, no
  claim, no `metadata.spec_ids` — and closes only with a terminal reason: an accepted spec
  serving that intent exists on main (the reason names its PR), the intent stops wanting
  one, or reconciliation closes it as a duplicate of the surviving carrier for the same
  intent. One carrier per subject; there is no shared coordination record.

Standalone carriers are discovered mechanically: a full `list` by the `planner-dispatch`
label enumerates them, and each names its intent. A sweep creating one enumerates first and
uses the existing carrier naming its subject instead of creating a second. They are not
chain records and nothing walks through them upward.

### The dispatch entry and the receipts

Before launching, the dispatcher appends a dated **dispatch entry** to the carrier, naming the
subject, the dispatched planner's actor, the dispatching sweep's actor, the time in UTC, a
recovery contact, and — for a replacement — the earlier entry it succeeds. The dispatcher
launches only after that append succeeds; a refused or failed write means no launch and
leaves no entry, so the subject is as it was before the attempt: the next sweep, or the
same dispatcher, retries on the same carrier, again appending its entry before any launch.

The planner appends a dated **start receipt** to the same carrier as its first act on the
subject, naming its actor and session. A refused or failed append is retried; a planner that
still cannot write stops and reports through the dispatch entry's recovery contact rather
than working invisibly. It appends a dated **outcome** when it stops: a spec author names
the spec PR or the failure; a design planner names the published design and acceptance, the
phase it left the task in, or the failure. A planner stopped by its supervisor has that stop
recorded as its outcome by whoever stopped it; when the planner could not record its own
outcome, the recovery contact records it from that stop report. Completion and failure are
both outcomes; only a completion names a deliverable.

Entries are append-only. A wrong entry is superseded by a later dated entry that says so;
nothing rewrites or removes an earlier entry. Native history keeps what was written when.

### What the record can prove

Only dispatch entries, start receipts and outcomes bear state; skip entries, unresolved
reports, retractions and corrections only annotate, because a retraction withdraws exactly
the dispatch entry it names and a correction supersedes exactly the entry it names. The
subject's state is the state of its **owning dispatch**: the earliest dispatch entry that
is neither retracted nor superseded and is not yet closed by an outcome. A closed dispatch
stops owning, which is what lets a replacement succeed a dead one. A carrier with no owning
dispatch — never written to, or every dispatch retracted or superseded — has no current
dispatch, and a sweep treats the subject as free, reusing that carrier. Four states, and
only these establish them:

- **Pending launch** — the owning dispatch has no receipt or outcome yet. In flight; sweeps
  skip.
- **Started** — the owning dispatch has its planner's start receipt and no outcome. In
  flight; sweeps skip.
- **Completed** — the owning dispatch closed with an outcome naming a deliverable. A design
  planner's completion frees the subject. A spec author's completion does not: the subject
  stays in flight until the named spec PR is observed merged and accepted on main, and
  sweeps skip citing it; the carrier then closes with that reason, while a PR closed
  unmerged is recorded and frees the subject. A stale in-flight entry beneath a completion
  suppresses nothing either way.
- **Positively dead** — the owning dispatch closed with an outcome recording that the
  planner stopped before any deliverable, with the observation named: the planner's own
  final report, the dispatcher's observed launch failure, or the supervisor's observed
  termination. Each names who observed, what, and when.

Nothing else is evidence of death. Silence, elapsed time, a missing branch, worktree or PR, a
failed forge or network query, and a timeout establish nothing and dispatch nothing. A sweep
that finds only absence leaves the dispatch standing, reports the subject as unresolved to
the entry's recovery contact, and records that report on the carrier. A launch that never ran
(dispatcher-observed) and a planner that ran and stopped are different outcomes and are
recorded differently, because only the second leaves a recovery question.

### Skipping, and two dispatches at once

Before dispatching onto a subject, a sweep reads the carrier and decides from the record
alone. A subject that is pending, started, unresolved, or whose completion names a spec PR
not yet merged accepted is in flight: the sweep dispatches nothing and appends a dated skip
entry citing the carrier and the entry that decided it.

The launcher's lock covers one command, not a read-decide-write-launch sequence, so two
sweeps overlapping inside that window can both record a dispatch. The record reconciles this
without a fence: of two live dispatch entries for one subject, the earlier one owns it. A
dispatcher that finds an earlier live entry beneath its own appends the retraction of its
own entry first — a refused or failed retraction is retried and escalated like any failed
write, and the planner is stopped only after the retraction is recorded — then leaves the
earlier dispatch standing; a retracted dispatch is not live even before its planner has
stopped. After reconciliation a subject has at most one live dispatch. The same window
exists at standalone-carrier creation, so the reconciliation repeats one level up: of two
carriers naming one intent, the earliest-created owns the subject, and a sweep that finds a
sibling beneath its own moves its dispatch there, then closes its carrier as a duplicate
citing the earlier one. Moving a dispatch means appending dated entries to the survivor
that restate the moved dispatch and its receipts and outcomes, each citing the closed
record and the entries it restates; nothing is deleted from the closed record, and state
classification reads the survivor alone. A closure write follows the same rule — retried,
then escalated — and the duplicate is not treated as closed until its closure is recorded.
The same window exists when the sweep files a task for a draft spec: of two tasks naming
one draft spec, the earliest-created is the carrier, and the dispatcher who finds a sibling
beneath its own retracts its dispatch entry there, stops its planner if one was launched,
and closes that task as a duplicate citing the earlier one, transferring entries the same
append-only way. A task mapped before any dispatch decision — the ordinary case — is
unaffected. Preventing the overlap in the first place would need a lease or fence that
spec016 deliberately omits; this spec omits it too.

### Recovering a dead dispatch

A subject whose planner died before any deliverable is not silenced. A sweep may dispatch a
replacement by following the carrier alone: read the entries, find the positively dead entry,
append a replacement dispatch entry naming the entry it succeeds, then launch. The
replacement is a further entry on the same carrier, not a new record; the sweep after it
skips, citing the replacement. Where a native claim exists, the replacement recovers ownership
through spec016's procedure before claiming; this contract re-opens dispatch eligibility
only.

### What a dispatch record is not

Not admission: spec012's chain and spec016's gates bind the planner as they bind anyone, and
no entry flips a task's phase by itself. Not ownership: a pending or started entry authorizes
nothing on a task the writer does not own; exclusive ownership on tasks is spec016's claim
alone, and the three things this spec provides are distinct — visibility from the record,
exclusivity from the claim, recovery from the procedure. Not a reservation of spec, intent or
task numbers: the 2026-08-31 filing collision, where two agents each filed 021 and 022 from
the remote, is a numbering problem and stays out of scope. Not a cross-clone guarantee: the
carrier is visible to every linked worktree of one clone; independent clones share nothing
until explicit replication, and this spec asks none. Not a change of who decides: the
orchestrator still chooses what to dispatch, the planner still does the work, and the record
is the memory both leave where the next agent can read it.

## Acceptance

1. Pre-push visibility: after record-then-launch, `show` on the carrier from a different
   linked worktree names the planner, subject, dispatch time and recovery contact, with no
   branch, commit or PR in existence.
2. Write-before-launch: a refused or failed carrier append results in no launch; a dispatcher
   either records and launches or does neither.
3. One planner between two sweeps: two successive sweeps over one subject produce exactly one
   dispatch, and the second's skip entry cites the carrier ID and the entry that told it to
   skip.
4. States are decidable from the record alone: pending launch, started, completed and
   positively dead each name their evidence, and a fresh agent classifies a carrier without
   asking its writers, including a carrier whose entries include skips, reports, retractions
   and corrections.
5. Death evidence is positive: a planner's final report, an observed launch failure or an
   observed termination, each naming observer, observation and time; silence, elapsed time,
   missing branch/worktree/PR, failed queries and timeout establish nothing and dispatch
   nothing.
6. Retry after positive death: a fresh sweep launches exactly one replacement, recorded as a
   further entry on the same carrier citing the dead entry; the next sweep skips, citing the
   replacement; a native claim is recovered through spec016 before the replacement claims.
7. Duplicate reconciliation: two dispatch entries for one subject leave the earliest owning
   it; the later dispatcher retracts before stopping its planner, retries and escalates a
   failed retraction, and no subject keeps two live dispatches; two standalone carriers
   naming one intent, or two tasks filed for one draft spec, leave the earliest as the
   carrier and the later closed as a duplicate citing it once its closure write succeeds;
   and a moved dispatch's entries reappear on the survivor citing their source, with state
   classification reading the survivor alone.
8. Completion frees the subject: a design planner's recorded deliverable suppresses
   nothing; a spec author's completion keeps the subject in flight until the named spec PR
   merges accepted on main, and a PR closed unmerged frees it; no sweep cites a stale
   in-flight entry as a reason to skip.
9. Standalone carriers: an unspecced-intent record is chore-typed, labelled
   `planner-dispatch`, intent-linked through `metadata.intent_ids`, enumerable by label,
   never planned, claimed or mapped, and closed only by an accepted spec on main, an
   explicit cancellation reason, or duplicate reconciliation naming the surviving carrier.
10. Task-anchored dispatches add no new phase or claim semantics: design planning claims
    through spec016; a spec author on a mapped task is notes-only and changes no phase.
11. Append-only history: corrections supersede by later entries; no entry is rewritten or
    removed, and native history still shows what was recorded when.
12. Scope honesty: the record and this spec claim visibility and deduplication for one
    clone's linked worktrees only, and promise neither prevention of simultaneous dispatch
    nor cross-clone fencing.
13. Failed writes: a refused or failed receipt append is retried, then escalates to the
    recovery contact; a planner that cannot write stops rather than working invisibly, and
    whoever stops a planner records its outcome; retractions and duplicate closures follow
    the same retry-and-escalate rule, and a closure is not treated as done until written.
