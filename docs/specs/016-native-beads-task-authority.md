---
id: 016
title: Complete task records in shared native Beads
status: accepted
intents: [015]
---

## Behavior

All task content and coordination live in native Beads: title, description, plan, acceptance,
notes, references, evidence, lifecycle, priority, dependencies and ownership. Intents and specs,
including their approval and acceptance states, remain versioned Markdown. Task mutations do
not edit, stage or commit anything in the code checkout, on `main` or a work branch.

This replaces spec014's split between Markdown task content and a custom coordination ledger.
The basis is approved intent015 and the owner's explicit decision to move **all task content**,
including plans and notes, into shared native Beads and retire `tasks/*.md`. That decision changes
task storage, not who approves an intent, and approves no unrelated draft intent. The amendment
is recorded in intent015; spec012 still governs admission and the PR review floor.

Every linked worktree of one clone discovers the same persistent embedded store. Commands from
those worktrees serialize locally; native atomic claim decides which actor owns a task. Remote
Dolt publication is explicit replication and backup, not global exactly-once dispatch among
independent writer clones. No custom ledger API, writer lease, authority epoch, freshness envelope,
background reconciler or claim of cross-clone fencing survives from spec014.

## Contract

### Runtime and storage boundary

`./tools/work-state` is a thin launcher for pinned Beads **1.1.2**, using Python **3.11+** standard
library facilities on macOS and Linux. There is no dependency install, shell command interpolation,
Dolt server or daemon. The existing `tools/work-state-beads-1.1.2.toml` is the immutable release
pin, not a second source of mutable work data.

The only launcher-specific command is:

```shell
./tools/work-state install --archive PATH --bd PATH
```

It checks the current target's archive filename, archive SHA-256 and extracted binary SHA-256
against that pin before installing native `bd` at the absolute Git common directory's
`plasmosome-beads/bd`. It rejects missing/unsupported targets and mismatches without accepting an
unverified binary. It does not change user PATH or install another global executable.

All other arguments are native Beads argv. For each invocation, the launcher discovers the
absolute Git common directory from the caller's checkout and fixes:

```text
runtime     = <absolute-git-common-dir>/plasmosome-beads/bd
BEADS_DIR   = <absolute-git-common-dir>/plasmosome-beads/store
command lock= <absolute-git-common-dir>/plasmosome-beads/command.lock
```

The runtime directory, store and private invocation directory are created with mode `0700`.
Existing directories must belong to the current effective user and have no group/other POSIX
mode bits. Refuse unsuitable ownership or modes without rewriting them; the operator inspects
and repairs the filesystem before retrying.

An OS `flock` covers the full native invocation, including reads, initialization and explicit
sync commands, and is released when the process ends. All linked worktrees use this same lock
and store, not their individual Git directories. A checkout switch, worktree removal or new
session does not create a new queue. Native stdout, stderr and exit status propagate to the
caller; launcher refusals must be distinguishable failures, not native success with no action.

The launcher refuses database/directory/global/repository-routing overrides that could escape
this boundary, including `--db`, `--directory`, `-C`, `--global` and `--repo`, in supported flag
forms and relevant environment variables. A caller-supplied `BEADS_DIR` cannot redirect it.
A native command that performs a claim requires an explicit nonempty `--actor` or `BEADS_ACTOR`;
a silent shared Git username is not a claim identity. Actors distinguish sessions for coordination,
not authentication of people or proof that an agent obeyed a planning gate.

The launcher refuses effective `close --continue` and `--suggest-next` before any mutation.
Pinned Beads 1.1.2's automatic continuation changes the next step to `in_progress` without
assigning ownership. Both suggestion paths also skip native history auto-commit in JSON mode,
including `--continue --no-auto`. JSON can come from flags, aliases or configuration, so the
boundary rejects these convenience flags rather than reproducing native output configuration.
Close normally, inspect eligible next work, recheck its admission/dependencies and explicitly
`update NEXT_ID --claim` with a unique actor. Inspection is not reservation; a competing claim
may win. Ordinary JSON close remains available. Native `close ID --claim-next` uses an atomic
claim and requires explicit identity.

Native init is non-destructive and uses `--skip-agents --skip-hooks --non-interactive`. It must
not discover and auto-commit the code repository: use a private working directory, Git discovery
ceiling and no-Git-operations configuration. No checkout instructions, hooks, staged files, code
commits or branches are changed. Caller file inputs remain relative to the invocation worktree,
even when native execution uses a private cwd; stdin remains usable. Repeated init must refuse
or preserve an existing store, never replace it.

Dolt auto-commit is on for durable native mutation history. Auto-push and auto-export are off,
and caller overrides cannot silently turn ordinary work into publication or tracked export.
Native administrative commands do not grant permission to remove history or redirect production
storage. Ordinary agents do not invoke destructive reset/GC/compaction/history-skipping operations
on task records. The launcher is a storage adapter, not an authentication sandbox for arbitrary
malicious native commands.

### Native record and workflow API

| Meaning | Native representation |
| --- | --- |
| Identity | Imported `plasmosome-NNN` or documented collision-resolved ID; new native hash ID |
| Why and body | `description` |
| Execution plan | `design` |
| Definition of done | `acceptance_criteria` (CLI `--acceptance`) |
| Notes and evidence | `notes`, appended with `--append-notes` |
| Upward links | `metadata.spec_ids` and `metadata.intent_ids`, ordered arrays of three-digit strings |
| Reading/reference links | `metadata.refs`, an array |
| Legacy provenance | `metadata.legacy_id` plus preserved source snapshots and migration provenance |
| Current PR | `external_ref` (CLI `--external-ref`) containing its URL |
| Ownership | Native assignee set by `update ID --claim` |
| Blockers | Native dependency edges and, for external blockers, status plus explanatory notes |

The task title and every field above are writable in Beads, never imported read-only projections
of task Markdown. Plans do not move into specs. Spec and intent IDs remain permanent separate
three-digit Git namespaces; task creation does not scan branches for the next number. Preserve
ordered metadata arrays and unrelated metadata keys when editing a link.

The chosen operations are native, with an explicit issue ID whenever an existing issue is
addressed; the native last-touched default is unsafe in a shared store:

```text
create --type task --title TITLE --description BODY --acceptance CRITERIA
       --labels needs-plan --metadata JSON
show ID [--json]
list [--all] --limit 0 [--json]
ready --label planned
list --status planning --limit 0
list --status review --limit 0
blocked
update ID --description BODY --design PLAN --acceptance CRITERIA
update ID --remove-label needs-plan --add-label planned
update ID --claim --actor UNIQUE_SESSION_ACTOR
update ID --claim --status planning --actor UNIQUE_SESSION_ACTOR
update ID --status in_progress
dep add CHILD PREREQUISITE
update ID --status blocked --append-notes REASON
update ID --external-ref PR_URL --status review
update ID --append-notes EVIDENCE
close ID --reason REASON
history ID --limit 0 --json
vc status --json
dolt pull
dolt push
```

`--body-file`, `--design-file` and `--metadata @file.json` are native file-input alternatives;
they do not authorize a tracked task export. The actual runtime retains native help and native
error reporting. This is not the old `import`, `heartbeat observe/apply`, `claim`, authority-mode
or `contract-test` wrapper API.

### Lifecycle, admission and ownership

| Stored status and labels | Meaning |
| --- | --- |
| `open` + `needs-plan` | Filed, awaiting a planner |
| `planning` | Owned, active work on this task's executable plan |
| `open` + `planned` | Admitted implementation candidate; ready also excludes dependency blockers |
| `in_progress` | Owned implementation, including active review repairs |
| `review` | Same author owns a submitted candidate through review and merge; `external_ref` identifies its PR when opened |
| `blocked` | No current task work can advance; notes identify cause and the phase to resume |
| `closed` | Delivered with merge evidence, or explicitly cancelled with its distinct reason |

Configure native `status.custom` as `planning,review` through `config set status.custom` at the
coordinated cutover below. These are actual stored statuses supported by pinned Beads 1.1.2,
not labels or a second state engine. `planned` and `needs-plan` remain mutually exclusive labels.
The former says the plan and implementation admission hold, not that a planner is working.
Do not configure `planned`, `todo`, `in_review` or `done` as statuses. The former review label
`in-review` is retired; readers use actual status, not that historical label.

A planner first reads the whole mapped task and its uniquely resolving spec/intent chain and
records the bounded planning assignment. Drafting a design does not authorize implementation:
draft specs and implementation prerequisites may remain while useful planning proceeds.
Keep those prerequisites and acceptance intact. An implementation blocker does not turn active
plan-writing into `blocked`; use `planning` while the planner can advance this task's design,
and `blocked` only when it cannot. Taskless intent/spec authorship keeps spec012's two shapes
and its own PR stage; do not pretend its PR is the implementation task's review or delivery.

Before adding `planned`, the planner supplies a complete design and acceptance, resolves every
spec link to accepted main content and checks its approved intent chain. Native ready filters
dependencies; native claim checks status and assignee, not dependencies or Markdown admission.
The implementation author rechecks the record, `planned`, governing main revisions and all
active implementation prerequisites before taking `in_progress` or opening its code worktree.
An existing planning claim is not permission to skip that check. Intent approval remains the
owner's instruction under `docs/intents/README.md`, never something the launcher infers.

For an unowned open task, a unique session actor acquires planning with
`update ID --claim --status planning`; implementation uses `update ID --claim`. Native claim
atomically assigns ownership and initially selects `in_progress`; the combined planning
invocation then applies `planning` before releasing the launcher's command lock and commits
the final state to native history. This is not a claim that both native updates are one
transaction. A losing actor is refused before the requested status update and does no work.
Read back both assignee and final status before work. If the combined command fails after
claim, preserve the acquired owner and inspect history; that owner finishes the missing
planning transition, rather than releasing or re-claiming to hide the partial result.

Same-actor claim is idempotent only where native claim accepts the current status; in particular
it refuses `planning` and `review`. Owners change phase with ordinary `update ID --status`,
not another claim. Those updates are cooperative, not authenticated owner checks. A planner
who continues as implementer retains its actor and explicitly takes `in_progress` after the
gate. For a planned handoff to another executor, the current owner publishes design/acceptance,
sets `open`, clears assignee and adds `planned` in one update; the next executor independently
checks admission and atomically claims. If the plan is drafted but admission is still pending,
retain `needs-plan`, record what remains, and wait as `blocked` without falsely admitting it.
The orchestrator dispatches the task ID, never claims on an executor's behalf.

Ownership is persistent, not an expiring lease. Recovery is explicit: establish the previous
author is gone, inspect its PR for actual merge evidence, append the reason and observations,
then recover the work's actual phase if acceptance remains unfinished. Missing branches and
failed network queries cannot establish abandonment. Preserve prior PR references in history
and notes when selecting a replacement current reference.

If a settled candidate still belongs in `review`, do not release it into `open` + `planned`
or re-claim it into `in_progress`. The orchestrator coordinates one replacement author, not a
replacement independent reviewer, and excludes competing recovery or dispatch for this task
through the handoff readback. The replacement revalidates admission, the candidate and current
PR, then re-reads the expected previous assignee and `review` status. With that evidenced
release authority, it uses `update ID --assignee UNIQUE_ACTOR` without a status change and
verifies its assignee and `review` before doing work. This native assignment is cooperative,
not an atomic claim or compare-and-swap; without exclusive handoff coordination, or if the
expected record changed, leave ownership unchanged and resolve the conflict first. An
assignment or note alone does not restart review residence. This explicit recovery handoff
does not authorize an orchestrator to claim implementation work on an executor's behalf.

For other confirmed releases, clear assignee and set `open`. Retain `planned` only if the
plan and chain remain valid; otherwise use `needs-plan`. A candidate that is no longer settled
or admitted needs its actual remaining work and reason recorded, not blind preservation of
`review` or an implicit assumption that implementation is eligible.

A dependency edge CHILD → PREREQUISITE makes implementation of the child wait. Planning may
proceed only where that prerequisite does not prevent the assigned design work; keep the edge.
Never delete real dependencies to make ready list a task. Record the phase and cause before
setting `blocked`; retain the owner. When the cause clears, that owner resumes the actual phase
with an ordinary status update, or an evidenced release returns the task to `open`.

Enter `review` when a settled candidate is handed to independent review, including local review
before a PR exists. Record the actual PR URL as soon as it opens. Keep ownership while reviews,
CI or an expected provider retry are pending. Active implementation repairs take `in_progress`,
then return to `review` on resubmission; a real external impediment uses `blocked` with the
resume phase. Do not flip status for a note, review reply or ordinary polling interval.
After the full PR-review gate and observed GitHub `MERGED`, append the PR URL, actual squash
commit and `mergedAt`. This proves source publication, not completion of the task's entire
acceptance. Before closing, the author re-reads the complete native acceptance and verifies
every remaining requirement, including any coordinated live configuration, migration, recovery
or operational measurement. Close with the merge and acceptance evidence only when all required
work is complete. Otherwise retain ownership, record the unfulfilled criteria and next action,
and use the phase of the actual continuing work; a blocker retains its cause and resume phase.
Do not close early and reopen later, invent a transition for evidence, or equate a taskless
prerequisite spec PR with delivery of its implementation task. The concrete failure is a closed
task whose required live behavior has never been exercised. A branch tip, deleted branch,
successful CLI invocation or closed-but-unmerged PR is not source delivery. Cancellations record
their distinct reason. No status-only code PR follows.

### Current-status age

Measure the current uninterrupted stay in the actual native status, not time since the last
note, label or assignment edit. `updated_at` changes for those edits; `started_at` belongs to
native claim and is not the entry time for planning, review or a later implementation visit.
Neither is a status-age source. Re-entry after another status starts a new stay. Preserve
earlier visits when reporting time in each status; do not sum them into the current age.

Collect `vc status --json`, `show ID --json`, `history ID --limit 0 --json`, and another show
and vc status. A changed generation or task during this capture is a concurrent observation,
not an atomic snapshot: retry a bounded capture or report unknown. The launcher serializes
individual commands only. Record the observation time in UTC and the native generation.

History rows carry `CommitHash`, `CommitDate` and `Issue.status`. Pinned native history emits
unchanged issue snapshots at unrelated commits and sorts by commit date, not parent lineage.
For established continuous history, walk the current-status run back to its oldest row before
the preceding different status; later notes and unchanged rows do not move that boundary.
Use that row's commit time as the durable native status-entry time, not its Issue.updated_at
or the latest history commit. Convert timestamp offsets before subtraction. This measures
committed status residence, not CPU time or sub-command execution latency.

Every reported age includes its evidence class and source:

- **Exact:** the entry transition and uninterrupted path to the observed generation are
  established, including native auto-commit success. A witnessed transition followed by a
  complete, known single-writer linear history with a consistent clock provides this proof.
  State the boundary hash/time, observation time and resulting duration at source precision.
- **Lower bound:** a continuous suffix in this status is established from a known observation
  to now, but its entry is not. Report at least that suffix duration and its start evidence;
  never name that observation the actual transition.
- **Unknown:** history is unavailable, incomplete or ambiguous, current readback disagrees,
  commit ordering/clock evidence is unreliable, or continuity cannot be established. Report
  the reason and any candidate boundary separately, without inventing an age or treating it
  as zero.

The native history JSON has no parent graph or completeness certificate. Timestamp adjacency,
a matching head, an old `updated_at`, or two equal endpoint observations alone cannot prove
no intervening exit/re-entry. In particular, imported, merged, restored or compacted history
needs independently recorded continuity evidence before a candidate becomes exact or a
historical suffix becomes a lower bound. Native SQL is unavailable in pinned embedded mode;
do not bypass the launcher, query raw store tables, add a cache or fabricate a timestamp to
fill that gap. Fresh coordinated transitions can establish future measurable residence without
rewriting historical entry times. Measurement artifacts are evidence, not a mutable task store.

### Coordinated lifecycle cutover

Apply the revised lifecycle to the live store only after this contract is reviewed and accepted
on main. The integration owner explicitly pauses other native writers, dispatch, coverage
reads and remote pulls from before evidence capture through final readback. Per-command locking
does not provide that pause. Capture full records, comments, native generation and configuration
and retain a restorable backup under the recovery rules below.

Inspect existing custom statuses before setting `planning,review`; unexpected values require
reconciliation, not silent removal. Resolve each active task with its actual owner and work:
active executable-plan authorship becomes `planning` even with implementation prerequisites;
submitted implementation candidates become `review`; no advance possible remains `blocked`.
An uncertain owner or phase remains unresolved with evidence, not a guessed release or status.
Do not assign a taskless spec PR as an implementation PR to make the transition look complete.

Use ordinary native updates, never reopen/close, re-claim, reset or replay history. Preserve
assignees, dependencies, external references, design, acceptance, metadata and all comments;
append the old state, reason, actual phase evidence and cutover observation to notes. Remove
the obsolete `in-review` label when reconciling its record. Read back every changed record and
the complete set to prove only intended status/label/notes fields and native automatic update
timestamps changed. Record the new native generation before resuming writers. Verify every
new status transition's history
boundary. Its native age starts at cutover; any evidenced earlier real-world phase duration is
reported separately, never backdated into native history. On partial failure keep the pause,
read the affected row, preserve already successful writes and resume only unfinished changes.

### Full-task migration and historical evidence

Cutover freezes old task editing and shared-store task writers for the final import. Enumerate
all legacy task records and relevant unmerged revisions dynamically; do not assume the historical
39-record fixture is the complete queue. Preserve old worktrees and databases while reconciling
them. The migration records the selected Git source revisions and source paths.

For each imported task retain its exact legacy ID and title, why/body, complete plan, acceptance,
notes, priority, ordered spec/intent links, reference lists, PR/evidence fields and every unknown
frontmatter field or extra section. Map active fields to the native schema above and preserve
complete source bytes/snapshots with origin revision and digest in native comments, indexed by
migration metadata. Conflicting branch versions are preserved as distinct source snapshots;
choose the live value deliberately, recording the reason rather than silently overwriting one.
When different tasks used the same legacy number, allocate distinct stable native IDs and retain
the shared original number in `metadata.legacy_id` on each. The discovered legacy043 collision
uses `plasmosome-043` for the controller task and `plasmosome-delegation` for one-hop delegation.
Imported IDs are stable and a repeat import cannot duplicate tasks or overwrite subsequent
native edits. The complete native-comment archive is part of both export and restore proofs.

The original import mapped old `todo` to `open` + `needs-plan`, `planned` to `open` + `planned`,
`in_progress` to its native counterpart, `in_review` to `in_progress` + `in-review`, and `done`
to `closed`. Preserve that migration history; the coordinated lifecycle cutover above replaces
its active review representation. If an old claim has no reliable actor, establish its owner or
record a blocker rather than inventing one. Legacy missing links survive as history, not new
work admitted around the gate.

The old shadow-sync product PR84 and its custom API remain historical. Preserve PR86 benchmark
results and associated evidence in native notes/provenance (including non-Markdown attachments
where needed), not a new task Markdown record. PR87's manual file-closure path is superseded;
reconcile its relevant evidence without requiring that closure mechanism to land. None of those
experiments establishes the new runtime's acceptance by itself.

Before deleting tracked task files, demonstrate full-field/source preservation, restart survival
and a restorable backup. Then retire `tasks/*.md` and `docs/templates/task.md`, migrate every active
instruction to the native API and remove obsolete shadow-runtime consumers. There is no mutable
tracked JSONL/JSON/Markdown export standing in for the old files. Immutable source snapshots and
logical backups are historical restore artifacts, not task mutation inputs after cutover.

### Replication, backup and recovery limits

The live writer domain is one clone's Git common directory. Its worktrees share process locking
and native claims. Independent clones have independent locks and may both claim their local copy;
Dolt merge/push is replication, not a distributed dispatcher, ownership fence or exactly-once
external-effects protocol. Operate one active writer clone for a queue, or explicitly quiesce and
hand it over before another clone writes. The owner may use remote readers while accepting that
their contents are only as fresh as their last successful explicit pull.

`dolt pull` and `dolt push` are deliberate native operations through the same launcher/lock.
Configure the intended Dolt remote using the pinned native commands and record the chosen URL,
ref and successful publication generation in Beads migration/backup notes. A local mutation or
auto-commit does not publish; ordinary reads never pull. No synthetic freshness envelope is
promised: after failed/no synchronization, report remote currency as unverified rather than
claiming local success means global truth. Resolve divergence explicitly while writers are
quiescent; never force-push away task history to recover a rejected push.

Use an intended encrypted transport, such as HTTPS or SSH, for off-machine replication and
credentials. Do not send credentials or confidential task content through plaintext HTTP.
Native Dolt permits HTTP authentication; this launcher retains native transport semantics and
does not enforce TLS. Local filesystem backups remain supported.

Take a restorable native store backup with writers quiescent, plus a logical export containing
all task fields and provenance. Preserve the source Git snapshot and old stores. Demonstrate
restoration into a separate location, including contents and native history; record backup paths,
digests, native version, source generation and restore observations in Beads. Publication to a
remote is additional off-machine protection, not proof that this backup restores.

To recover, stop writers, preserve the current store before replacing anything, restore a verified
backup, compare logical records and history, and deliberately select the active writer store.
Do not silently resurrect task Markdown as a fallback or enable two writable authorities. Remote
failure does not erase locally committed work; it means those changes are not confirmed backed
up remotely and another clone must not be promoted on an assumption of freshness.

## Acceptance

The migration/integration owner supplies observed output, versions, relevant commit/generation
IDs and restorable artifact locations in native task notes and PR evidence. These are required
proofs of this contract, not claims already established by accepting this document:

1. **Pinned install:** the expected platform artifact installs; wrong archive filename, archive
   digest and binary digest each refuse before replacing a verified installation. Native version
   is 1.1.2. Supported-platform coverage and any platform not actually exercised are named.
2. **Shared discovery and safe init:** main and at least two linked worktrees see a task created
   from any one of them. Init and task content/lifecycle mutations leave code HEAD, index, tracked
   content, agent instructions and hooks unchanged; repeated init preserves the populated store.
   File input from a linked worktree resolves there despite the private cwd.
3. **No escape or silent actor:** alternate flag forms/environment store-routing attempts refuse
   or are bound to the shared store. A claim without explicit identity refuses. Native stdout,
   stderr and nonzero errors propagate. Ordinary commands cannot enable automatic push/export.
4. **Real native task behavior:** create, read and edit every content field; acquire actual
   `planning`, complete an admitted plan, hand off to implementation ownership, enter `review`,
   block and resume the correct phase without losing owner or prerequisites. Show ready with
   `--label planned` excludes planning/review and real dependency blockers. Demonstrate that
   a planning claim does not satisfy the implementation gate and a losing combined
   claim/status update leaves the winner's phase unchanged. Close only with actual merge
   evidence. Do not claim native enforcement of agent-reviewed spec/intent gates.
5. **Competing claims and restart:** race two processes with distinct actors from linked worktrees
   on one eligible task; exactly one claim wins and the loser does not create a code worktree or
   dispatch work. Restart clients and show the owner and complete task fields remain. Releasing
   a stopped native process releases the OS lock; it does not silently release its task claim.
6. **Complete import:** enumerate the real source set, compare all mapped fields and ordered lists,
   account for every source revision and extra field/section, and verify preserved source digests.
   Include legacy conflicts and historical PR84/86/87 evidence. A rerun duplicates nothing and
   does not overwrite a deliberately changed native task.
7. **Backup, restore and explicit sync:** restore the full quiescent backup separately and compare
   task records/provenance and history. Record a successful intended-remote publication and an
   explicit pull into a separate reader, or state the exact unavailable remote prerequisite before
   claiming off-machine durability. Ordinary reads work without network and do not synchronize;
   a failed explicit sync leaves committed task content available and is reported as failure.
8. **Single authority cutover:** no tracked task files, mutable exports, task template, task numbering
   helper or obsolete shadow-runtime caller remains active. Root, Main orchestration, pipeline
   health statistics, planning, tasks and PR-review entry points all direct task operations to
   native Beads, while Git still governs
   specs/intents. A fresh-context review follows a Beads ID end to end and finds no closure PR or
   worktree task-copy requirement. Run the repository gate after integration, not amid concurrent
   edits, and report exactly what passed and what was not exercised.
9. **Timed phases:** in an owned isolated pinned runtime, observe planning, implementation and
   review transitions and retained earlier visits. Append notes and mutate another issue;
   the current-status entry must remain unchanged although updated_at or latest history moves.
   Exercise a return to review after another status. Show exact ages only for established
   uninterrupted history, a lower bound for an evidenced suffix without its entry, and unknown
   for missing or ambiguous history. Record command outputs, boundary hashes and source
   precision, not a desired-state model or invented timestamps.
10. **Live lifecycle preservation:** after accepted review and the explicit all-writer pause,
    capture the original full set and comments, reconcile actual owner/phase evidence, and
    verify final records and native history. Preserve implementation prerequisites during
    active planning, keep taskless PRs distinct and leave uncertain claims unresolved.
    Report native cutover residence separately from older evidenced real-world phase time.

## Out of scope

Distributed exactly-once claims/effects across independently writing clones; a custom transition
engine or approval service; moving specs/intents into the task store; task Markdown compatibility
shims; automatic sync, remote reconciliation, background servers or paid service requirements.
