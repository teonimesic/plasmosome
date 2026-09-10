---
id: 009
title: How much of an intent has been built, recorded in the intent and checkable against delivery
status: draft
intents: [008]
---

## Behavior

An intent records the owner's judgment of how much of the goal exists, in `served:` and
`## What is served`. Specs, task lists and delivery facts are derived, not copied into that
prose. The judgment answers what the owner considers built; the derivation shows the recorded
work beneath it. Neither makes previously unwanted work approved.

A read-only command shows that derivation or checks it against the judgment. It reads Git
intents/specs, complete native Beads records and actual GitHub PR states. Invalid inputs or
unknown delivery evidence refuse the run before any coverage verdict. With valid inputs, it
reports only malformed coverage fields and the two contradictory extremes: `none` with delivered
work, or `substantially` without it. Passing means consistency with these records, not proof that
the goal is satisfied.

This document specifies later implementation; it does not add fields, backfill judgments or
activate a heartbeat check. Its reviewed acceptance belongs to the planner under approved
intent008, following the spec index, without a second owner spec-approval gate. Actual coverage
values and prose, including backfill, still require the owner's judgment. Implementation waits
for this spec accepted on main and an admitted native plan under specs012/016.

## Contract

### The judgment and its owner

The existing intent `status:`, `outcome:` and `## Outcome` keep their meanings and content.
`status:` and `outcome:` already exist in the intent template; there is no missing approval-field
change to wait for.

| Field | Question | Values |
| --- | --- | --- |
| `status:` | Does the owner want this? | `draft`, `approved` |
| `served:` | How much of it exists? | `none`, `partly`, `substantially` |
| `outcome:` | Is this still open? | Blank while open, non-blank once settled |

`served:` appears exactly once, at the start of its own line inside frontmatter, directly after
`status:`, with exactly one of the three values. Count every line beginning `served:` in the
file, then check its position and value: a valid-looking frontmatter field plus a second field in
the body is malformed too. The template ships `served: none`; this default is not permission to
assign `none` to an existing intent during backfill.

`none` means nothing built, `partly` some, and `substantially` most of what was wanted, with what
remains named in prose. There is no terminal coverage value. Open work beneath `substantially`
is normal. Approval and existence are independent: a draft intent may have any coverage value,
including `substantially`, without authorizing implementation beneath an unapproved goal.
Only the owner chooses its actual value, even when the draft describes an area with existing code.

The template gains `## What is served` before `## Outcome`, with this prompt:

```markdown
## What is served

How much of this goal exists, and what is left? Name what is missing, not specs or tasks.
```

The section contains what exists and what is left, with no spec IDs, task IDs or counts. It is a
running judgment, not a stored query result. When the intent settles, it stays as the final
coverage account and stops being updated. `## Outcome` remains available for what was built or
why nothing was, as the current template permits. Do not delete or rewrite it merely because
some prose overlaps; this migration does not reinterpret existing outcomes.

Only the owner originates `served:` and `## What is served` judgments. An agent may record an
actual judgment it carries, identify landed work and prepare a draft PR for the owner to read,
but must not select a value or manufacture approval from task counts or a passing check. A PR
changing either judgment remains draft until the owner reads and approves those judgments on
GitHub and takes it out of draft; it names the actual source of the judgment. An agent does not
mark it ready. This applies to the initial backfill as well as later revisions.

The reason differs from intent approval: coverage commits no implementation queue, but compares
what exists with what the owner wanted. No coverage value is a task-admission or priority gate.
The draft-PR wait for changing the judgment still applies. It is an instruction, not an approval
authenticator. The mechanical check may contradict a recorded value; it never chooses its
replacement. Between owner readings the judgment may remain stale.

### Inputs and refusal come before coverage

The future command is `./tools/intent-coverage check`, run from the repository root.
`./tools/intent-coverage show INTENT_ID` reads the same derivation for one intent without requiring
its coverage field. Neither command changes Git, Beads or GitHub. Neither synchronizes the native
store, reconciles tasks, retries a failed query indefinitely or falls back to task Markdown.

Read the complete native set with `./tools/work-state list --all --limit 0 --json`. A successful
empty native array is valid; a failed query, invalid JSON, duplicate native ID or incomplete
result is not. The result is a local observation, not a claim of remote Beads freshness. GitHub
reads below are explicit additional network inputs, not native `dolt pull` or publication.

Validate every numeric intent/spec document before selecting by status, following spec012 and
the two document READMEs. Read IDs from frontmatter, never infer identity from the filename.
A missing folder, empty document set, numeric document missing its ID, missing/duplicate/malformed
ID or state, duplicate resolved ID, invalid intent list, dangling link, or accepted spec without
an approved intent chain is an input fault. Non-document files such as the folder README are not
coverage inputs. A directory containing only files without intent records refuses, not passes.

Native IDs and statuses must be well-formed; the statuses consumed are `open`, `in_progress`,
`blocked` and `closed`. Link arrays must be arrays of three-digit strings with uniquely resolving
targets. Existing imported empty links remain visible history, not permission to admit new
unmapped work. For a task with spec links, its copied intent list must equal the first-seen ordered
union from those specs; a mismatch is a repairable input fault, not another approval gate. An
imported task with empty spec links may still reach intents through its direct links. No missing
link is silently replaced by an inferred goal.

Both commands first validate all these structural inputs and the closure evidence below for
every closed task that reaches any intent. Only then may they emit derived results or coverage
faults. `show` also requires its requested three-digit ID to resolve to an existing intent.
Unknown requested IDs are input faults, not empty successful lookups.

Any input fault emits an `input:` diagnostic on stderr naming the failing authority and, when
available, its file, Beads ID or PR URL. Exit status is **2**, with no derived rows or per-intent
coverage faults on stdout. Collecting several input diagnostics is allowed, but partial coverage
results are not. Thus a planted spec naming absent intent099 refuses the entire run, rather than
validating the remaining intents and appearing successful. This deliberately replaces the earlier
draft's tolerant dangling-link acceptance to agree with spec012. A simultaneous malformed
`served:` field does not override input-fault precedence.

### Closing a task is not proof of delivery

Spec016 stores both delivery and cancellation as native `closed`, with human-readable evidence.
Those prose fields are not a machine-readable discriminator: imported007 and004 have legacy
merge evidence but no `close_reason`; cancelled047/049 have prose reasons and unmerged PR links;
cancelled050 has no `external_ref`. Searching for words such as "merged" or "cancelled" cannot
safely interpret these records. A PR URL alone and a nonempty reason prove neither outcome.

For this consumer, add one typed annotation **inside the existing native issue**:

```json
{"closure": {"kind": "delivered", "closed_at": "<exact native closed_at string>"}}
```

This is the partial object passed to native `update ID --metadata`; the stored path is
`metadata.closure`. Its only keys are `kind` and `closed_at`. `kind` is exactly `delivered` or
`cancelled`; `closed_at` is a valid RFC3339 timestamp with an offset, copied exactly from that
issue's current native `closed_at`. Missing, extra, mistyped or unknown fields, or a timestamp
mismatch, mean unknown evidence. A reopened task is not delivered. A reconciler reopening it
clears the annotation in the same native update with `--metadata '{"closure":null}'`; each later
closure requires a fresh evidence annotation. Timestamp equality alone cannot detect a reused
annotation if two closures receive the same timestamp; clearing it is part of the native workflow,
not an authenticated history guarantee made by this read-only command.

This annotation selects the native closure's meaning; it does not replace status, reason, PR
identity or forge evidence. It is needed because spec016 deliberately permits prose closure
reasons, and does not define a parseable cancellation field. It uses spec016's extensible native
metadata and partial-object update, not a second database, tracked export or custom task status.
There is no new authenticated actor or owner approval gate for task-evidence reconciliation.

A task reconciler establishes the annotation from the complete native record and actual evidence,
records the decision and source in native notes, then writes the partial metadata object without
altering old notes, reasons, legacy evidence, source snapshots or unrelated metadata. A cancellation
also needs its explicit reason in `close_reason`; where historical cancellation exists only in
notes, record that evidenced reason there without deleting its source. Do not classify prose by
substring in the command, migrate by blanket default, or annotate an ambiguous record. Reconcile
all relevant historical closed rows before enabling the check. Until then missing annotations
refuse, including imported delivered rows. This spec does not perform that migration.

The command applies this table in order. "PR observation" means a successful GitHub query of
`state,mergeCommit,mergedAt,url` for the exact canonical `external_ref` URL in this repository,
`https://github.com/teonimesic/plasmosome/pull/N` with positive decimal N. Query each distinct URL
once per invocation. Require the returned URL to match and the fields to have their documented
types. Never take a PR mentioned in notes as the task's PR; a replacement PR may be the reason
for cancelling this one. The command does not parse old prose to compare its commit claims.

| Native observation | Required evidence | Derived disposition |
| --- | --- | --- |
| Status is not `closed` | Structurally valid native record | Open work; not delivered, regardless of an old closure annotation |
| `closed`, annotation absent, malformed or not bound to current `closed_at` | None can substitute for the annotation | Unknown; input refusal |
| `closed`, kind `delivered` | Canonical `external_ref`; PR state `MERGED`; non-null merge commit with full 40-hex `oid`; non-null valid `mergedAt` | Delivered; report actual PR URL, commit and merge time |
| `closed`, kind `cancelled`, absent or empty `external_ref` | Nonempty explicit `close_reason` | Cancelled; not delivered; report the reason |
| `closed`, kind `cancelled`, nonempty `external_ref` | Nonempty explicit `close_reason`; canonical PR URL; PR state `CLOSED` with null `mergeCommit` and `mergedAt` | Cancelled; not delivered; report the reason and PR |
| Any other closed combination, including failed/unavailable forge reads | No inference or fallback | Unknown; input refusal |

In particular, cancellation linked to a merged or still-open PR is conflicting evidence, not
permission to hide delivered work. A delivered annotation linked to a closed-unmerged PR is also
unknown, not cancellation. Offline or unauthorized forge access is unknown, not zero deliveries.
The no-PR cancellation case relies on the deliberately recorded native cancellation decision;
it is not a claim that software authenticated its author. Incorrect task-to-PR attribution is
reconciliation work under spec016, not something PR state alone can prove.

The annotation and evidence requirements apply to both link paths. Imported007 retains empty
spec links and intent001; after evidence reconciliation, PR6 supplies its merge facts. Imported004
retains spec003 and intent002 and obtains merge facts from PR8. No requirement to map the historical
007 to a fabricated spec is introduced. Current ordinary closures use the same annotation and
forge predicate as imported ones. There is no shortcut for a familiar ID or a quoted short SHA.

### Derivation and the three coverage faults

For an intent, first list all specs whose `intents:` reaches it, with their states and tasks whose
`metadata.spec_ids` names each spec. Then list directly reaching tasks whose `metadata.intent_ids`
names the intent and which have not already appeared. Sort specs by ID and tasks by complete
Beads ID. A task under several matching specs appears under the first matching spec only; a task
reaching both ways appears once under its spec. A task with empty spec links appears in the direct
group. Identity is the complete native ID, never a legacy number or source filename.

Each task row reports native status and the derived disposition, including cancellation reasons
or actual merge evidence where applicable. A matching spec is shown even when it has no tasks.
`show` exits 0 after these rows, and prints nothing only when the existing requested intent has
neither matching specs nor directly reaching tasks. It never copies this output into an intent.

Work has landed under an intent exactly when at least one task reaching it by either path is
**delivered** under the table. Cancelled and open work do not count. With all input validation
complete, `check` emits one stdout line per coverage fault, naming the intent file and fault:

1. `served: none` with landed work.
2. `served: substantially` with no landed work.
3. Not exactly one well-formed `served:` line: absent, empty, duplicated, outside frontmatter,
   not directly after `status:`, or a value outside the three.

A malformed field receives only fault3, not an attempted semantic reading as fault1 or2. Sort
faults by intent ID. Exit **1** when any coverage fault occurs; otherwise exit **0** with no output.
No input diagnostic is a fourth coverage fault, and exit2 never carries partial coverage output.

Coverage comparisons do not use an intent's approval status. Shared structural validation still
checks status declarations and accepted-to-approved chains; "approval is independent" is not a
reason to omit those checks. A well-formed draft intent with delivered work and `substantially`
passes. Open tasks never contradict `substantially`, and no delivery state contradicts `partly`.
A `partly` intent can still occur in a refused run with unknown evidence; that refusal makes no
claim about its coverage value.

These extremes compare a judgment with recorded delivered work, not every file in the tree. Work
without a recorded link cannot be seen, substantial satisfaction cannot be calculated, and a
passing `partly` value may be stale. The owner must read the goal. Neither passing nor repairing
a task link retroactively authorizes work or proves the intent's original request was met.

### Activation and complete backfill

Implementation delivers the command, field/template changes, owner-supplied backfill and active
workflow integration together. It first reconciles the relevant native closure annotations and
evidence without rewriting history. The post-backfill tree must pass with actual forge inputs;
no clean check is claimed for today's tree without `served:`. A closed task becoming unknown
later produces a refusal, not a silent degradation to no work.

Every existing numeric intent receives the field; none is silently exempt because it is draft or
settled. The owner supplies each value and any actual `What is served` prose, including the final
account for an already-settled intent. Missing owner judgments hold that later draft PR; they do
not block planner acceptance of this specification. The template's empty prompt is not a claim
about an existing intent, and evidence reconciliation does not decide its coverage.

The later implementation puts the operational owner-judgment rule and vocabulary in
`docs/intents/README.md`, with links from the task skill and the review skill's draft-opening
section. It adds the check beside heartbeat's governing-document inspection, not its capacity
section. The task skill points to this contract for closure evidence preparation rather than
creating another lifecycle or field-value table. The active documents are not changed by this
spec-only proposal. No scheduler, automatic judgment update or coverage-based dispatch is added.

Retained legacy link faults are also real activation prerequisites. At this proposal's base,
the native delegation record names absent spec015 and intent016, and closed049 names absent
spec015. Numeric-document validation alone does not make those native links valid. Cancellation
classification does not exempt a row from structural link validation: even an evidenced
cancellation with a dangling spec link refuses before any coverage verdict. An authorized task
reconciler must resolve these live mappings from actual governing documents and recorded history,
preserving previous mappings and their reasons in provenance. Do not invent the missing documents,
drop the records, or remove links merely to pass this check. Until that reconciliation and the
closure-evidence migration are complete, the promised clean activation run remains unproved.

## Acceptance

The implementation must demonstrate all of the following; specification acceptance is not a
claim that these implementation proofs have run:

- The template has `served: none` directly after `status:` and the new section before `Outcome`.
  Every numeric intent has exactly one correctly positioned field. A matching-file count cannot
  substitute for checking each file; an extra `served: mostly` line in the same file must fail.
- Actual backfill values/prose have owner judgment and GitHub approval provenance; the PR stays
  draft until the owner ends the wait. Missing judgments are not replaced with defaults or derived
  percentages. Existing `status:`, `outcome:` and `Outcome` content remain unchanged by backfill.
- The intent README defines the three values and owner-only judgment once. Task/review skills
  link to that rule; draft opening includes changes to either value or prose. Heartbeat's
  governing-document inspection calls the check. No intent prose stores spec/task IDs or counts.
- The actual post-backfill tree passes, using complete native input and successful forge reads.
  Record Git revision and observation time; ordinary local reads do not prove remote Beads currency.
  Include the complete retained native set: a cancelled row with an unresolved legacy spec link
  still refuses. A clean numeric-document sweep alone cannot satisfy this acceptance item.
- Inject each of the three coverage faults separately. Also exercise deleted, empty, doubled,
  body-only and misplaced fields, including one valid and one invalid line in a single file.
  Each malformed case names the file once; each coverage-only failure exits1.
- A draft intent at `partly`, and a draft intent at `substantially` with delivery, pass; so does
  `substantially` with both delivered and open tasks. `partly` passes with or without delivery.
  No approval-state combination becomes a coverage fault.
- Reconciled imported007 with empty spec links, direct intent001 and actual PR6 merge evidence
  makes `none` fail and `substantially` pass. Imported004 with spec003/intent002 and PR8 appears
  exactly once under its spec. Exercise multiple matching specs and deduplication by full native
  ID, including different IDs sharing a historical legacy number.
- `show` prints matching specs even without tasks, shows direct-only tasks, and shows delivery
  facts or cancellation reasons. An existing intent with neither path produces empty success;
  an unknown requested ID refuses. The folder README never receives a coverage diagnostic.
- Exercise current and imported closures through the same table. Missing/wrong annotation type,
  unsupported kind, extra keys, missing or mismatched closure timestamp refuse. Reopening clears
  the annotation and is open work; closing again without fresh annotation refuses.
- Explicit no-PR cancellation with a reason contributes no delivery. Cancellation with a confirmed
  closed-unmerged PR contributes none. Exercise047/049 and the no-reference050 historical shapes
  after deliberate reconciliation, preserving their original sources. Missing cancellation reason,
  cancelled-plus-merged/open PR, and delivered-plus-unmerged PR each refuse. Closed alone, URL
  alone, prose saying "merged", and legacy evidence alone cannot establish delivery.
- Delivered annotations require actual matching PR URL, `MERGED`, full commit and merge time.
  Exercise failed/offline/malformed/mismatched forge responses; each refuses rather than returning
  no deliveries. Repeated references to one PR use one observation within a run, not mixed states.
- Missing/empty intent or spec directories, numeric documents missing or duplicating IDs/status,
  unknown state, invalid accepted chain, dangling099, malformed native link arrays, copied-link
  mismatch, duplicate native ID, unavailable store and incomplete enumeration each exit2 with an
  input diagnostic and no derived/coverage stdout. Combine dangling099 with a malformed coverage
  field to prove input-fault precedence. Imported empty links are preserved, not discarded.
- Invoke from a wrong directory and one holding only non-record intent files under both `bash`
  and `zsh`: the command produces its own refusal, not a shell glob error or quiet success. Normal
  invocations also work in both shells; do not use zsh's read-only `status` variable.
- The checker is read-only even when refusing. Evidence reconciliation uses native partial metadata
  updates and preserved notes, never a tracked export. Unknown history blocks activation until
  reconciled; it is not erased, guessed cancelled or reclassified as an owner coverage judgment.

## Out of scope

This spec PR builds none of the field, command, backfill or workflow integration. It changes no
actual intent judgment or approval, and no native closure annotation. Those are later implementation
and reconciliation work under the accepted contract. It does not add a dashboard, an intent rollup,
a terminal coverage value, a measure for `partly`, a new task authority, a scheduler or a mechanical
owner authenticator. The root gate and normal independent/provider review apply to delivery;
accepting this contract does not assert those implementation results or spend a review slot.
