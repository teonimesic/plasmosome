---
id: 012
title: How work enters the tree — the chain, the gates, and what a pull request must carry
status: accepted
intents: [008]
---

## Behavior

A reader can determine whether a change was wanted, whether it was allowed to start and whether
someone read it before it merged. The chain from intent to spec to task, the starting gates and
the review evidence make those questions answerable without relying on the author's memory or
self-attestation. Skills instruct agents; this spec is the floor those instructions answer to.

Task records now live wholly in shared native Beads under
[`016-native-beads-task-authority.md`](016-native-beads-task-authority.md), not task Markdown.
The owner's full-task-storage decision replaces this spec's former task-file schema and sweeps.
Intents/specs and their states stay in Git; GitHub supplies review and merge facts. This revision
changes record storage and selectors, not the closed admission gates, structural PR shapes or
owner approval authority.

## Contract

### The chain and its two structural shapes

Every work PR names a Beads ID, whose `metadata.spec_ids` links to specs, whose `intents:` links
to intents. Every accepted spec names intents; the former spec001 amnesty is closed and empty.
No accepted spec can stop the upward walk with an empty list. A task's `metadata.intent_ids`
copies the first-seen ordered union of its specs' intent lists; a missing copy is repairable
metadata, not a second approval gate where the chain already closes through its specs.

Exactly two shapes carry no task:

- A PR filing an intent, which is the top of the chain and has no parent.
- A PR filing a spec, which still names the intent it serves and lacks only the task for the
  implementation that follows.

This list is closed here, not in a skill the exempted change is also editing. Adding a third
requires a separate revision of this spec. No size, kind or category of change is exempt.
“No spec describes my area” is not a structural shape: propose a spec under a wanted goal, or
put a draft intent to the owner, or drop the change with a reason. A decision record can inform
a task but is not its spec or an alternative top of the chain.

New task filing names an existing spec; it may be draft while the task awaits planning. Imported
unmapped legacy tasks retain valid history but must be mapped before becoming planned or being
claimed. Recording their evidence or correcting their notes does not require inventing a goal.
A review finding is not authority to create unmapped work.

### Starting gates and owner approval

The gates on starting are two:

1. A spec may not become `accepted` until every intent it names is `approved`.
2. Task implementation may not start until every spec it names is `accepted`.

Drafting is not implementation. A draft spec can name a draft intent; a mapped task may have
an owned native planning phase while its design or implementation prerequisites are unresolved.
That claim authorizes planning only. Its governing spec lands accepted before an implementation
claim, transition to implementation or code branch. This corrects the former undifferentiated
“claim” wording, which hid active planning as blocked implementation. Plans, acceptance and
copied links remain the complete gate record, not another owner approval process. Spec016 alone
defines native planning ownership and implementation admission; the two taskless shapes above
are unchanged.

Intent approval originates with the owner. An agent may record or relay that actual decision,
but may not generate one and cite it as an independent check. The approval workflow and visible
draft-PR wait are in `docs/intents/README.md`; decision008 explains why this remains an instruction
rather than an authenticated software boundary. A work PR reaching an unapproved goal is waiting
on the same owner, not a stalled author expected to bypass that wait.

The planner accepts a spec once its reviewed PR merges under an approved intent. No additional
owner spec-approval gate is introduced. Adding a new starting gate requires revising this spec,
not an extra sentence in a skill.

### What a merged PR carries

- A link upward through its native task, or one of the two structural shapes above.
- A review that read the commit being merged, not a signal that is equally green when review was
  queued, skipped, rate limited or never started.
- An answer to every finding, by a fix or a reasoned disagreement, with threads resolved on the
  change actually being merged.

Evidence binds to the validated head. The merge operation refuses a different head rather than
checking one SHA and racing into merging another. The review skill defines reviewer selection,
round counts, endpoints and how empirical claims are checked; none of those may weaken this
floor. Native task closure follows observed GitHub merge evidence and cannot substitute for it.
Task status changes and closure are Beads mutations, never follow-up code commits or closure PRs.

### Records must remain discoverable

Read each authority using its own record structure. A task is a native issue, with the field,
metadata and lifecycle schema in spec016, not a frontmatter line selected from task files.
Enumerate complete native results when auditing the queue, including closed records when checking
history; a default-limited page or failed query is not the complete set. Missing required fields,
invalid links or an unavailable store are explicit faults rather than evidence of no work.

Numeric Markdown intent and spec documents still declare their IDs and status once in
frontmatter, with states from the sets in their respective READMEs. Validate all such documents
before selecting a queue by status. A missing, empty, duplicate, malformed or unknown state must
not hide a record. IDs and link targets resolve uniquely; no “take the first match” recovery.
Missing folders or present-but-empty document sets refuse rather than reporting a clean sweep.
The task directory is retired and is neither a required input nor a fallback for this check.

A task mutation does not change the code checkout, index, HEAD or hooks. Task read/write paths
in the root and skills consistently use `./tools/work-state`, including planning, dispatch,
review and recovery. The native adapter enforces storage sharing and command serialization;
it does not claim to validate whether prose fulfills a spec or whether the owner really approved
an intent. These remain agent checks and the recorded human decision where applicable.

### A process rule must be contradictable

A rule in this layer names what can refuse it: a mechanical check over the appropriate records,
a person's judgement visibly recorded, or a concrete failure somebody can count. An agent's own
statement that it followed the rule is none of those. For example, an open work PR with no task
surviving a heartbeat is an observable failure; green review status that also means “skipped” is
not evidence of review.

Write an operational rule and each closed set once; other agent-facing mentions point there.
Remove older copies when editing that rule. This spec describes the contract, skills describe
operations, and spec016 defines native task representation. Historical superseded specifications
are not active alternatives. A spec can be corrected with the reason when reality shows its
design wrong; the closed lists bind where that change is reviewed, not whether change is possible.

## Acceptance

- A fresh agent can follow a work PR's native ID through all specs to their intents. The tasks
  skill identifies the two taskless shapes, preserves their distinct parentage and refuses size
  or category exemptions and an unmapped area masquerading as a third shape.
- Planning and dispatch read `show ID`, the native design and acceptance, and every governing
  spec/intent. Planning ownership is visibly distinct from implementation admission, including
  active planning while implementation awaits prerequisites. Ready does not itself validate
  that gate; spec016 supplies the unique-actor claim and phase-transition proofs.
- The owner-approval rule is in the intent README; heartbeat treats a draft waiting on that
  approval as waiting on the owner, and review never promotes it on an agent's own judgement.
- PR review requires current-head empirical/review evidence and responses to every finding.
  A skipped, rate-limited, failed, absent or partial review observation cannot pass as completed.
- Numeric intent/spec records with missing, empty, duplicate, malformed or unknown status,
  duplicate IDs, missing links or an accepted-to-unapproved chain are reported as faults, not
  silently omitted. Missing and present-but-empty document sets both refuse. These are record
  checks, not a new task Markdown sweep or approval authenticator.
- Native queue inspection uses full enumeration for audits and reports store failures distinctly
  from empty results. Imported incomplete legacy records cannot acquire planned/claimed status
  by exploiting their age, and are not discarded to make the queue look clean.
- Root and workflow skills contain no active task-file creation, copying, numbering, state-flip,
  status-only PR or file-based closure instruction. Task evidence stays in native notes with
  actual forge references; specs/intents remain Git content.
- Newly edited operational rules have one authoritative statement and a concrete refusal or
  observable failure. Their implementation and limitations are described honestly, not inferred
  from self-reported compliance.

## Out of scope

A mechanical approval authenticator or CI gate on PR-body task links; reviewer-specific round
counts; vision/architecture/decision content; intent outcome measurement governed by spec009;
and native storage/replication implementation governed by spec016. No clause creates a new
product goal or approves a draft intent.
