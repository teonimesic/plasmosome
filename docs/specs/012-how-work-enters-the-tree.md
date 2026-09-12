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
Intents/specs and their states stay in Git; GitHub supplies review and merge facts. Specialists
own design, execution, independent contradiction and knowledge maintenance. Main coordinates
their work; an owner question pauses the affected work, not the rest of the project.

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

Before a planner accepts a spec, a fresh-context independent reviewer examines its complete
contract under the PR-review skill. The review must be able to refuse unnecessary abstractions,
duplicate authorities, speculative goals, unsupported claims and acceptance nobody can observe.
It asks what existing solution suffices and what can be removed, not how many roles agree.
Findings return to the spec author; the author does not supply its own independent verdict.

The planner accepts the reviewed spec under approved intents in the final reviewed commit
before merge. It must reach accepted `main` before downstream work enters logical **todo**.
Todo means spec016's admitted `open` + `planned` candidate, with its implementation prerequisites
clear; **backlog** means filed work not yet admitted, not another queue or custom status.
Native ready and claim do not validate specs, prose or all dependencies. The executor rechecks
admission and claims for itself, verifying owner and phase before work. Active planning, review
and blocked work retain their distinct native phases under spec016.

No additional owner spec-approval gate is introduced. Adding a new starting gate requires
revising this spec, not an extra sentence in a skill.

### Responsibility, not another agent engine

| Responsibility | Owns | Does not replace |
| --- | --- | --- |
| Main | Pipeline health, allocation, non-overlapping assignments and help communicating with the owner | Spec/plan authorship, development, empirical QA or knowledge curation |
| Architect or tech lead | The smallest wanted spec, native plan, acceptance and technical decisions | Owner intent approval or independent review of its own design |
| Research specialist | Primary sources, bounded comparisons, observations and explicit uncertainty | Implementation authority or proof that a documented integration runs |
| Development author | Implementation, validation, its PR, responses, repairs and merge | Independent review or required QA evidence |
| Independent reviewer | Contradiction of the candidate and governing acceptance, with scope and head identified | Repairing the author's change or merging it |
| QA specialist | Applicable independent runtime, UX, security or performance evidence and its limits | Quiet repairs, merge authority or source inspection presented as execution |
| Knowledge curator | Reusable knowledge, its evidence, accounting, consolidation and retirement | Main's allocation role or the originating task's history |

The original author owns findings through merge while available. Reviewers return findings,
not a replacement implementation; confirmed recovery follows spec016. A settled implementation
enters independent PR review and appropriate QA, with native review/repair phases following the
actual work. Reuse unchanged validation evidence where the review contract permits it, but
independently investigate uncertain claims. A read-only reviewer is not an execution-capable QA
agent merely because both assess quality.

Use existing capable agents and selective instruction loading before adding role definitions.
These are responsibilities, not simultaneous headcounts, one profile per discipline, a fixed
author/WIP limit or a new scheduler. Main's allocation remains a judgement informed by actual
work and review capacity; another task's wait is not authority to take its owner's work.

Rotate product, research, UX and performance discovery through bounded questions drawn from
current gaps and previous evidence. Research can identify a measurement; an execution-capable
specialist performs it. Record the revision, scenario, observed result, limits and cleanup in
the relevant native task. Reuse matching work; new work follows the chain above. Missing
authority leads to the appropriate proposal, not an unmapped task or a speculative queue.
Do not repeat a known experiment without a changed input, implementation or question.

### Bounded reusable knowledge

Specialists contribute evidence through their originating native tasks: the repeated failure
or decision, proposed changed action, source/revision, observation, limits and existing guidance
it duplicates or contradicts. A contribution is not a new rule. The curator checks currency,
recurrence and authority, then retains, merges, replaces, retires or defers it with a reason.
The existing rule-evidence requirements in `AGENTS.md` still apply; knowledge maintenance does
not convert an untested writing rule or one-off incident into accepted guidance.

Keep each policy once. Role instructions identify responsibilities and relevant entry points;
procedures reference this contract rather than copying its limits. Preserve detailed evidence
in native records and Git/PR history, outside active reusable instructions. The curator owns
knowledge changes through independent review, not Main and not every agent appending a lesson.

The initial project-controlled instruction budgets are:

| Account | Maximum `o200k_base` content tokens, except the count row |
| --- | ---: |
| Each scoped instruction file, including `AGENTS.md` or its equivalent | 1,600 |
| Each task-agent definition, metadata and body | 1,000 |
| Active project skill count | 8 |
| Each skill and its mandatory instruction-bearing support closure | 3,000 |
| Aggregate active skill instruction closure | 12,000 |
| Rendered project skill catalog | 600 |
| Each agent's loaded project reusable-instruction packet | 12,000 |
| Initial task brief, counted separately | 2,000 |

Record the tokenizer/encoding version, revision, paths, hashes, method and observed counts.
Full files include frontmatter. These named-encoding counts are not model billing, wire tokens
or the entire conversation. Count the assembled packet separately: applicable scoped files,
selected role, catalog, required/read/autoloaded instruction bodies and wrappers. Repeated
injection counts each occurrence. Shared canonical support counts once in the aggregate and
within each skill's required closure for its individual limit.

Include hidden active skills, aliases, shadow copies and mandatory instruction-bearing
references. Moving instructions into an archive, task boilerplate or another file cannot evade
the relevant account. Distinguish ordinary task evidence from reusable instructions by use,
not filename. Report other effective instruction roots separately; project ownership does not
authorize changing user-managed or global guidance.

Before each reusable change, the curator measures its before/after inventory and loading.
Overflow requires evidenced consolidation, replacement or retirement, or deferring the
candidate outside active instructions. Never silently enlarge a limit or truncate safety and
approval rules. A necessary scoped exception records its reason, scope, expiry and restoration
condition; it does not raise the standing budget. A permanent budget change needs a separately
reviewed revision supported by observed loss under the existing bound and the cost of changing it.

Budget adoption is a distinct operational deliverable. Remeasure the accepted tree and safely
consolidate any existing excess before adoption, with independent review of preserved decisions
and counterexamples. Until that proof exists, preserve accepted guidance and report adoption
incomplete: no grandfathered compliance, destructive pruning or claim that accepting this spec
made the current library fit. This does not stop unrelated admitted work.

Retirement requires current replacement or obsolescence evidence, migrated references and
autoload names, and actual discovery proof across effective roots. Hiding a catalog entry is
not retirement; archived material must stay outside active discovery and mandatory reads.
Lack of recent use alone does not retire a safety rule. Already-loaded session text does not
disappear when its file is retired.

Prove both isolated cold-project discovery and the actual caller's project root, selected
role/model, catalog, loaded bodies and packet counts. A successful in-repository child does
not prove a parent launched elsewhere discovers those instructions. File presence, supported
source code and a refresh banner are not observed loading. Any later authorized caller
adoption must also preserve extensions, running timers and artifact references; otherwise
record the precise operational blocker. This contract authorizes no self-restart or relocation.

### Questions go to the owner without transferring the task

The owner selected native OMP Collab for live interaction. It is a transport, not project
approval authority: full links grant broad bearer control and expose shared session history;
display names do not authenticate the owner. Keep those links private and out of task/PR
evidence. Selection does not prove activation, phone access, direct worker replies, tool
permission handling or automatic Beads integration, and authorizes no additional gateway or
router/public-hostname exposure.

End the shared room with `/collab stop` when withdrawing its bearer authority. This disconnects
the room without stopping the underlying agent; a later start creates a new room and keys.
`/collab view` only presents a read-only link: it does not revoke an earlier full-control link.
Before declaring revocation complete, observe that existing guests disconnect and old full
links cannot reconnect, read further live content or submit accepted writes, including against
any replacement room. A leak or uncertain revocation keeps affected work blocked. A replacement
link does not establish human identity or approval, and revocation cannot recall copied history.

The responsible specialist asks directly through the selected, available channel, with the
GitHub proposal URL and exact revision. Before sending, record in native notes the request ID,
task, owner/actor, originating agent/session, question and decision scope, proposal revision,
channel/conversation reference, time and phase to resume. Append the nonsecret message reference
and observed send result; pending or failed delivery is not a sent or answered request.
Mark only dependent work blocked and retain its owner. Main allocates unrelated eligible work;
it may help communication but need not relay every question.

A taskless proposal keeps its request on its GitHub draft/thread. Block existing dependent
tasks without inventing an unmapped task to hold the conversation. If the originating worker
is unavailable, use explicit ownership recovery, not silent substitution or expiry.

Correlate an answer to the exact request, revision and scope and record the sender evidence
the channel actually supports. Ambiguous, stale, superseded or unauthenticated replies remain
unresolved. Duplicates cannot authorize another action; silence, disconnect, notification
receipt and a generic “approve everything” are not approval. A refusal is not delivery or
authority to cancel other work.

Distinguish a technical answer, runtime tool permission and formal intent approval. GitHub
owner reading/approval and owner-ready provenance remain canonical under the intent README
and decision008. Agents sharing the owner's GitHub account cannot authenticate human origin
by account name; Collab bearer possession or display names do not repair that limitation.
Never manufacture canonical approval from chat. Resume the retained owner in the recorded
phase only after the scoped answer, required canonical evidence, admission and dependencies
permit it. Do not blindly turn planning or review into implementation.

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
surviving Main's work check is an observable failure; green review status that also means “skipped” is
not evidence of review.

Write an operational rule and each closed set once; other agent-facing mentions point there.
Remove older copies when editing that rule. This spec describes the contract, role instructions
and skills describe operations, and spec016 defines native task representation. Historical superseded specifications
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
- The owner-approval rule is in the intent README; Main treats a draft waiting on that approval
  as waiting on the owner, and review never promotes it on an agent's own judgement.
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
- A governing spec receives independent contradiction before acceptance admits downstream todo.
  Review can refuse needless abstraction, duplicated authority and unverifiable acceptance.
  Research, authorship, review, QA and curation have distinct responsibilities; Main does not
  perform them, and original authors retain findings through merge or evidenced recovery.
- Bounded discovery changes its question based on evidence, reuses existing work and follows
  admission for new findings. It creates neither a scheduler nor speculative tasks to fill slots.
- A later knowledge implementation demonstrates all budget accounts, before/after measurements,
  safe consolidation of existing excess and preserved decisions before claiming adoption.
  Hidden aliases and mandatory support cannot evade accounting; retirement is proved in both
  cold-project and actual-caller discovery, not inferred from a moved file or truncated prompt.
- A later owner-channel integration exercises requests from planning, implementation and review;
  correlated current replies resume the same owner and correct phase only with required evidence.
  Stale, ambiguous, unauthenticated, refused, duplicate and disconnected cases cannot approve.
  An affected wait leaves unrelated eligible work runnable and preserves taskless proposal shape.
- Owner-channel proof withdraws an existing full-control room, observes old-link reconnect/read/
  write refusal and verifies replacement-room isolation. A read-only link is not revocation;
  uncertainty leaves affected work blocked without claiming already-copied history was erased.
- Source acceptance does not claim deployed roles, compliant loading, live Collab routing or
  automated blocked/resume behavior. Their native implementation tasks retain those proofs.

## Out of scope

A mechanical approval authenticator or CI gate on PR-body task links; reviewer-specific round
counts; vision/architecture/decision content; intent outcome measurement governed by spec009;
and native storage/replication implementation governed by spec016. No clause creates a new
product goal or approves a draft intent.
