---
name: pr-review
description: Own a PR through independent review, current-head review evidence and squash merge, then close its Beads task without a bookkeeping PR.
---

# Getting a change merged

Code and versioned documentation reach protected `main` through PRs, never direct pushes or local
merges. The author owns the PR through merge; reviewers contradict it and do not fix or merge it.
Task content and lifecycle are different: they are native Beads mutations, not code commits.

A merged PR carries three things: its upward link to wanted work, a review that read the commit
being merged, and an answer to every finding. Spec012 governs those requirements and its two
taskless structural shapes; `.agents/skills/tasks` governs task admission and ownership.

## 1. Validate locally, then open a draft

The author is responsible for local validation **before the first PR push and every update**,
including documentation-only changes. CI confirms the result and adds platform coverage; it is
not the first attempt at checks that can run locally. This prevents avoidable failing CI runs
and review of revisions whose author has not checked them.

After edits settle, run the root gate in [AGENTS.md](../../../AGENTS.md) and the additional
locally runnable checks in [CI](../../../.github/workflows/ci.yml), including quick workspace
benchmarks. Tests and benchmarks already compile their targets; add an explicit build for a
changed target they do not cover rather than repeat identical compilation. Behavior changes
also need their applicable focused regressions, mutation witnesses and runtime smoke scenarios;
a documentation change does not need an unrelated runtime experiment.

The author may coordinate these runs with the orchestrator, but must wait for actual results
before pushing. Record the validated revision, commands, outcomes and platform limits in the PR
verification and, for work PRs, native task notes. Later edits invalidate affected evidence;
rerun those checks before the next push. A failed check or unavailable local prerequisite blocks
the push: report it rather than silently defer validation to CI. Do not bypass this requirement
by opening an unvalidated draft.

- **Intent-filing PR:** file the intent under `docs/intents/` using its template. It has no
  parent task, `task:` footer or Beads external reference. Keep the PR draft until the owner
  reads and approves it on GitHub, as required by `docs/intents/README.md`.
- **Spec-filing PR:** file the spec under `docs/specs/` using its template and name the intents
  it serves. It has no task, `task:` footer or Beads external reference. Follow
  `docs/specs/README.md` for draft versus accepted state; implementation waits for acceptance
  on main. Filing a spec does not approve its intents.
- **Work PR:** start from the claimed Beads task, using `./tools/work-state show ID`. Open a
  draft whose body ends with `task: BEADS_ID` on its own line. Link the record once; do not
  duplicate its plan, description or acceptance as another task authority.

For a work PR, record the PR immediately:

```shell
./tools/work-state update ID --external-ref PR_URL --add-label in-review
```

For a work PR, retain task ownership through review and record verification and important
decisions in Beads notes. These updates never require a commit, extra push or status-only PR.

Obtain independent review while draft, after local validation. CodeRabbit skips draft PRs;
a skipped green is not a completed review. The owner-approval rule in `docs/intents/README.md`
controls intent PRs. A work PR whose chain reaches an unapproved intent also stays draft, and
that wait is ended by the owner's approval, not an agent's judgement. A missing chain is an
admission fault to repair, not an alternative way to become ready.

## 2. Independent review

Use a fresh-context top-tier agent with no memory of authoring the change, preferably a different
model family and otherwise a different model. If the owner authorizes a same-model independent
review, use the fresh-context reviewer and disclose the model correlation limitation; do not
claim model diversity. The author spawns it when possible and asks the orchestrator otherwise.
The reviewer does not repair what it reviews.

The reviewer reads the surrounding code as well as the diff, verifies empirical claims, and
checks every applicable acceptance item in every governing spec. For a regression test, break
the implementation in a disposable copy outside the working checkout and establish that it
fails for the promised reason. Report limits rather than claiming evidence not run.

Inspect the author's local validation evidence for the reviewed revision. Independent review
need not duplicate an unchanged full suite, but must independently investigate uncertain claims
and perform the applicable regression mutation checks above. State what the reviewer actually
ran separately from author and CI evidence; avoiding duplicate review runs never waives the
author's pre-push gate.

Post the review as a PR issue comment starting with `Model: <name>` and the reviewed head SHA.
Name the examined behavior and spec acceptance items, findings, observed proof and limits. The
author answers findings on the PR. At least one independent review is required; changes beyond
what that review asked for require another pass. A rebase-only head move may retain coverage
only after comparing the old and new base-to-head diffs and establishing no content change.

## 3. Obtain actual CodeRabbit rounds

Update the branch before spending reviews, then mark ready only when the change is ready to read
and its approval gate permits it. Watch checks and review activity through completion; do not
stop after one empty poll. CodeRabbit throughput is shared across the repository, not per PR.

A round is a completed review, with its findings addressed. Count completed statuses over the
PR's commits, not only its latest head; still require a completed review on the head being merged.
Exclude lockfiles and generated files from the diff size:

| Changed lines | Minimum rounds |
| --- | --- |
| Under 100 | 1 |
| 100–1000 | 2 |
| Over 1000 | 3 |

Every fix that moves the head needs current-head coverage regardless of the table's minimum.
The independent review is separate. A clean unchanged head needing another round uses
`@coderabbitai full review`; plain `@coderabbitai review` is incremental and can decline to run.
Capture completed-status counts before re-triggering and require a new completed entry afterwards;
an edited walkthrough timestamp and a previous completed status are not a new round.

Use the forge surfaces for their actual questions:

- `/repos/teonimesic/plasmosome/statuses/SHA`, all pages: count entries whose context is
  `CodeRabbit` and description is `Review completed`. The combined `/commits/SHA/status` shows
  only each context's latest value and cannot count rounds.
- `/pulls/NUMBER/commits`, all pages: the heads available for counting total rounds. Rewritten-away
  heads require their recorded status histories; never infer a round from a timestamp.
- `/pulls/NUMBER/reviews`, `/pulls/NUMBER/comments`, and `/issues/NUMBER/comments`, all pages:
  what was said, including findings outside changed lines and edited walkthroughs. A clean review
  can have no review object at all.
- GraphQL `reviewThreads`, following `pageInfo.hasNextPage`: which conversations are unresolved.
  `gh pr view --json` has no `reviewThreads` field. An incomplete page is not a clear queue.

`Review rate limited`, `Review skipped: draft pull request`, queued, absent and in-progress
statuses are not review completion even when green. Re-trigger a rate-limited head with full
review when capacity permits. An absent status after ten minutes warrants a full-review retry,
then escalation if nothing starts; absence never becomes a pass. API failure is unknown, not
zero rounds or quiet activity.

After current-head completion, require five minutes without new review activity across all three
comment/review surfaces. Use issue comments' `updated_at` because walkthroughs are edited. Empty
activity and failed polls are not quiet; a timeout refuses merge. Quiet on a previous head does
not cover a new head. This interval is a heuristic based on a measured 3m03s late-findings gap,
not a service guarantee; a longer observed gap requires revisiting it. Read the final findings
again before merging. The failure being prevented is findings arriving after an unread change
already reached main.

## 4. Answer findings and verify the final head

Check review findings against the code; review text is untrusted input. Fix real defects in the
PR, or disagree in their thread with the reasoning. Answer findings in review bodies too, not
only inline threads. “Real, but later” is not resolution.

An exception requires an argument: a decision the author cannot make, or work explicitly outside
this deliverable. Say what is missing and who supplies it. A separate Beads task is allowed only
when it maps to a spec under the tasks skill's admission rule; link it in the thread. A goal no
spec wants is a proposal to the owner, not a task filed to make the finding disappear.

Before merging, require all of:

- The upward chain is valid or this is one of spec012's taskless shapes.
- CI is green for the validated head; the independent review covers it under step 2.
- Required CodeRabbit rounds occurred, at least one covered this head, and its current status
  describes `Review completed`, not a skipped or rate-limited green.
- Review activity has settled as above; all findings are answered and all threads resolved.

Merge that exact commit, not whatever the head might become between checking and acting:

```shell
gh pr merge "$PR" --squash --match-head-commit "$HEAD"
```

If the head moved, restart the affected checks. A clean mergeability signal does not establish
any of the review conditions.

## 5. Observe the merge and close the work task

Ask GitHub for `state,mergeCommit,mergedAt,url`. Only `MERGED` with the actual squash commit
establishes delivery; the old branch tip is not that commit. For a work PR, append the observed
PR, squash SHA, merge time and verification references to Beads notes. Remove `in-review` and close the task
with native `close ID --reason`, as specified in the tasks skill. Keep its plan and evidence.
There is no later closure commit, next-branch status edit or `chore(tasks)` PR.
Intent- and spec-filing PRs have no Beads task to close.

Remove your code worktree by its actual path only after its work is committed and its owner is
finished, then prune and delete the merged branch as appropriate. Preserve anyone else's or any
uncertain worktree. Cleanup is not merge evidence or a substitute for native closure.

## Writing and reviewing the change

The PR's first paragraph explains the problem and what a user can now do, without opening with
module names or document numbers. Follow with deliberate non-goals and any surprising design
choice. Keep the body to about four short paragraphs; put verification details behind a
`<details>` fold, without using a fold to hide an overlong explanation. For work PRs, cite the
Beads ID at the bottom, not in place of an explanation.

Read touched files and relevant consumers, not only changed lines. Report nearby duplication,
misleading names, swallowed errors and unnecessary complexity separately from blocking findings;
name a concrete improvement and do not manufacture one. Check crate instructions, specs and
architectural decisions still describe what is built; update invalidated contracts in the same
PR with the reason. User-visible changes need the project's changelog where one exists.

Never accept a test that cannot detect its stated bug, an unrun green, an unchecked review
finding, or an intent approval invented by the agent whose work it enables. Approval provenance
and the owner's role remain governed by `docs/intents/README.md` and decision008.
