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
blocked
update ID --description BODY --design PLAN --acceptance CRITERIA
update ID --remove-label needs-plan --add-label planned
update ID --claim --actor UNIQUE_SESSION_ACTOR
dep add CHILD PREREQUISITE
update ID --status blocked --append-notes REASON
update ID --external-ref PR_URL --add-label in-review
update ID --append-notes EVIDENCE
update ID --remove-label in-review
close ID --reason REASON
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
| `open` + `needs-plan` | Filed; not yet eligible for dispatch |
| `open` + `planned` | Planned candidate; native ready also excludes dependency blockers |
| `in_progress` | Claimed by an author; work underway |
| `in_progress` + `in-review` | Same author owns PR review; `external_ref` identifies it |
| `blocked` | Explicitly blocked; notes identify cause and native edges identify prerequisite tasks |
| `closed` | Delivered with merge evidence, or explicitly cancelled with its distinct reason |

`planned` and `needs-plan` are mutually exclusive labels, not statuses. Beads 1.1.2's custom
planned status was probed: native update accepts it but ready omits it and claim rejects it.
Do not configure it or translate old `todo`, `in_review` and `done` into custom native statuses.
The native software has other features; this table is the chosen repository task workflow.

Before adding `planned`, the planner supplies a complete design and acceptance, resolves every
spec link to accepted Git content and checks its approved intent chain. The executor re-reads the
record and governing documents before claim, including a direct-ID assignment's planned label
and active dependency blockers. Intent approval remains exclusively the owner's instruction
under `docs/intents/README.md`; the launcher does not infer or write it. Native ready filters
dependencies; native claim checks open status and assignee but does not check dependency edges.
Neither validates Markdown specs or prose adequacy. These admission checks belong to the agents;
the adapter promises atomic competing ownership, not atomic dependency or spec validation.

A successful native claim atomically assigns the actor and moves the issue to `in_progress`.
A competing actor is refused. Repeating a claim as the same actor is idempotent, so each executor
must have its own session identity. Dispatch names the Beads ID; its executor claims before code
worktree creation or execution, and a losing executor causes no implementation or further dispatch
for that task. Neither chat assignment nor branch naming is ownership.

Ownership is persistent, not an expiring lease. Recovery is explicit: establish the previous
author is gone, inspect its PR for actual merge evidence, append the reason and observations,
then release/reassign through native updates if it was not delivered. Missing branches and failed
network queries cannot establish abandonment. A released task retains `planned` only if its plan
and chain remain valid; otherwise it returns to `open` + `needs-plan`. Preserve prior PR references
in history and notes when selecting a replacement current reference.

A dependency edge CHILD → PREREQUISITE makes the child wait for the prerequisite. Do not remove
real dependencies to get a claim accepted. An external blocker uses `blocked` and notes; after
resolution its existing owner resumes, or a confirmed release returns it to the eligible queue.
Blocking does not automatically abandon ownership.

Review is native status `in_progress` plus `in-review` and the current PR URL. The author keeps
ownership and answers actual GitHub threads; task notes retain links and verification evidence.
After GitHub reports `MERGED`, append the PR URL, actual squash commit and observed merge time,
remove `in-review`, and close with the merge reason. Squash branch ancestry, a deleted branch,
a successful CLI invocation alone or a closed-but-unmerged PR cannot establish delivery.
Cancellations record cancellation, never a fabricated merge. No status-only code PR follows.

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

Map old `todo` to `open` + `needs-plan`, `planned` to `open` + `planned`, `in_progress` to its
native counterpart, `in_review` to `in_progress` + `in-review`, and `done` to `closed`. Reconcile
stale statuses from GitHub with notes preserving original values and evidence. If an old claim
has no reliable actor, establish its owner or record a blocker rather than inventing one. Legacy
missing links survive as history, not new work admitted around the gate.

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
4. **Real native task behavior:** create, read and edit every content field; plan with labels;
   show ready with `--label planned`; verify a dependency excludes its child until resolved;
   block/resume, review-reference/label and close with actual merge evidence. Demonstrate that
   unplanned work is not selected by the prescribed dispatch query. Do not claim native enforcement
   of the agent-reviewed spec/intent gates.
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
   helper or obsolete shadow-runtime caller remains active. Root, heartbeat, planning, tasks and
   PR-review entry points all direct task operations to native Beads, while Git still governs
   specs/intents. A fresh-context review follows a Beads ID end to end and finds no closure PR or
   worktree task-copy requirement. Run the repository gate after integration, not amid concurrent
   edits, and report exactly what passed and what was not exercised.

## Out of scope

Distributed exactly-once claims/effects across independently writing clones; a custom transition
engine or approval service; moving specs/intents into the task store; task Markdown compatibility
shims; automatic sync, remote reconciliation, background servers or paid service requirements.
