---
name: pr-review
description: How a change reaches main — PR-only workflow, review rounds by diff size, and what a review must not accept. Use when opening a PR, addressing feedback, or merging.
---

# Getting a change merged

`main` is branch-protected: direct pushes are rejected for everyone. There is no local-merge path.

1. Branch → push → **open the PR as a draft** (`gh pr create --draft`). One unit of work per PR.
   Where the work has a task, the PR body links it once — `task: NNN` on its own line at the
   bottom. Do not quote `done_when` or restate the plan; that is what the task file is for, and
   putting it at the top buries the thing only you can write.
2. While it is a draft, get your own house in order: the gate green, and the independent review
   done and acted on. **CodeRabbit does not review draft PRs**, so nothing you do here spends a
   round. Mark it ready (`gh pr ready <number>`) only when you would be content for someone to
   read it as it stands.
3. Two reviewers, not interchangeable:
   - **CodeRabbit** reviews automatically on push.
   - **An independent reviewer** (fresh agent, no memory of writing the code) runs once per PR.
     Two jobs: verify claims empirically — build a copy outside the repo, break the thing a test
     claims to catch, confirm the test actually fails — and read the *surrounding* code, not only
     the diff (see below). A third job, on every PR that has a task: read the diff against the
     `## Acceptance` list of the spec its `specs:` field names, and say line by line which are
     met.

     **Run it on a top-tier model.** This is the one step whose whole job is to disagree, so it
     is where capability matters most. Reaching for a smaller model because the review is "just a
     check" is the specific mistake: a weak reviewer produces agreeable text and no findings,
     which reads exactly like a clean pass.

     **A different model family would be better, and is not available here** — every model on
     offer is a Claude one, so name that limit rather than counting two different model names as
     independence achieved. The proxy we can actually apply is a model different from the
     author's: two agents on one model correlate, so a review that only confirms the author's
     reasoning looks like diligence and is not evidence. Today that means `model: "fable"` on the
     Agent tool, or `opus` where the author was Fable — the difference is the point, not the
     name. A same-model review still finds real defects; it just cannot be counted as this pass.

     **If you can spawn the reviewer, do; if you cannot, ask the orchestrator.** Whether a
     dispatched agent has the Agent tool varies by how it was dispatched, so check what you have
     rather than assuming either way. Neither answer is an excuse to skip the step or to review
     your own PR on your own model.

     **The output goes on the PR as an issue comment**, not only into chat, opening with
     `Model: <name>` and the head SHA it read, then saying what was examined, what was found,
     what the author changed, and what was declined with the reasoning. Naming the model is what
     makes a weak-model review visible afterwards instead of indistinguishable from a strong one.
4. **Watch for the review rather than waiting for it.** CodeRabbit posts minutes after a push,
   and again after every later push, so an agent that checks once and stops has stalled. Poll
   until the review lands and the thread count stops moving:

   ```shell
   gh pr checks <number> --watch
   gh api graphql -f query='{repository(owner:"teonimesic",name:"plasmosome"){
     pullRequest(number:NNN){reviewThreads(first:100){pageInfo{hasNextPage}
     nodes{id isResolved path comments(first:1){nodes{body}}}}}}}' \
     --jq '.data.repository.pullRequest.reviewThreads|
           if .pageInfo.hasNextPage then "MORE PAGES — do not treat this queue as clear" else empty end,
           (.nodes[]|select(.isResolved==false))'
   ```

   The agent that wrote the change owns this loop until its PR merges. Do not hand a half-reviewed
   PR back and call the work finished — a new comment after you stopped looking is the same as no
   review at all. `gh pr checks --watch` is for `gates`, which does settle; the `CodeRabbit`
   context is not a completion signal and is covered below. The thread query tells you what was
   said inline, and every query here must be re-run after each of your own pushes too.

   **A green check and an empty thread queue are not a clean pass.** A finding CodeRabbit cannot
   attach to a changed line — anything outside the diff — goes in the review body instead. It
   never becomes a thread, never moves the thread count, and the `CodeRabbit` status still goes
   green. Two Major findings arrived that way on PR #26, and both signals said there was nothing
   to read. Ask for the reviews themselves before believing a pass:

   ```shell
   gh api repos/teonimesic/plasmosome/pulls/<number>/reviews \
     --jq '.[] | select(.user.login=="coderabbitai[bot]") | "\(.submitted_at)\n\(.body)"'
   ```

   **The status settles before the findings do, so reading the right endpoint at the wrong moment
   still misses them.** `SUCCESS` does not mean the review finished. On PR #26 the `CodeRabbit`
   context went green for head `f0a3b0e9` at 15:24:58 with no push and no re-trigger after it, and
   findings kept landing until 15:28:01 — **3m03s** later. That pass produced ten review
   submissions in total. An absent check is not a passing one either: between your push and
   CodeRabbit starting there is no `CodeRabbit` context at all, so a poll for "not pending" reads
   clear before anything has run.

   **A green has three meanings, and one of them is "no review happened."** The `CodeRabbit`
   context reports `success` whether it reviewed and found nothing, reviewed and posted findings,
   or never ran at all — two of that PR's six greens carried the description `Review rate
   limited`. Only the description tells them apart:

   ```shell
   gh api repos/teonimesic/plasmosome/commits/<sha>/status \
     --jq '.statuses[] | select(.context=="CodeRabbit")
           | "\(.updated_at) \(.state) \(.description)"'
   ```

   `Review completed` is the only one that is a review. **`Review rate limited` is not a pass and
   will not become one** — waiting is pointless, because nothing more is coming. Re-trigger with
   `@coderabbitai review` and wait for a `Review completed` on the current head. Merging on a
   rate-limited green ships a change nobody reviewed, and neither the check state nor an empty
   thread list will ever say so.

   **There is a fourth meaning, and it makes the reviews endpoint useless for this question.** A
   review that finds nothing creates **no review object at all** — it posts its walkthrough as an
   issue comment and stops. The endpoint returns zero entries, not an entry with an empty body. PR
   #39 read `Review completed` with zero reviews and one issue comment; #34 and #36 read the same
   status with two and four. So zero reviews cannot tell a clean pass from a review that never
   ran, and to anything that counts them the two are identical.

   That settles which signal answers which question. **Did a round happen** is the status
   description on the head you are about to merge, and nothing else — not the check state, not a
   review appearing, not the thread count. **What did it say** is the reviews endpoint, read only
   after the description has already told you a review exists. A wait keyed on a review appearing
   never trips on a clean one and hangs until it times out; a merge gate keyed on the same
   evidence cannot see the difference between "nothing to say" and "never spoke".

   **The dangerous moment for a rate-limited green is the push you just made, not a quiet PR.**
   It arrives on a new head seconds after you act, which is exactly when a wait is primed to
   accept it as the fresh review it was waiting for — the check went from absent to green, on the
   commit you just pushed, in about the time a real review takes. A PR sitting untouched will not
   produce one. So the moment to read the description most carefully is the moment you are most
   convinced the review just happened.

   The budget is **repo-wide, not per-PR** — roughly ten reviews an hour across everything — so
   the cause is usually somewhere else: another agent's pushes, or your own earlier rounds. Being
   the only one pushing right now is not evidence that there is budget left.

   **When a PR is both behind and unreviewed, rebase before spending the review, never after.** A
   review is spent on one head, and rebasing makes a new one; updating the branch afterwards pays
   twice for the same diff, and in an exhausted window the second payment may not be there to
   make. Get the branch up to date first, then spend the review on the head you will merge.

   A rebase that changes nothing may not cost a review: CodeRabbit is said to re-stamp an existing
   completed review onto the new head when the diff is unchanged. **That is unconfirmed here** —
   across every review this repository has received, each Run ID appears on exactly one head, so
   we have never seen it happen. Treat it as something to check, never to count on. If a new head
   reports `Review completed` without a review of its own, confirm the diff really is unchanged
   before letting it stand for the old one:

   ```shell
   diff <(git diff <old-base>..<old-head>) <(git diff <new-base>..<new-head>)
   ```

   Empty output means the rebase was content-neutral, so the earlier review covers the merged
   bytes exactly. Any output means it does not, and the new head needs its own review.

   Nothing announces that the findings have stopped. **Wait for quiet instead**: track the newest
   timestamp across the three places CodeRabbit writes, and treat the queue as clear only once it
   has not moved for five minutes.

   ```shell
   PR=<number>

   newest() {
     local b r c i
     b=repos/teonimesic/plasmosome
     r=$(gh api "$b/pulls/$1/reviews"   --paginate --jq '.[].submitted_at') || return 1
     c=$(gh api "$b/pulls/$1/comments"  --paginate --jq '.[].created_at')   || return 1
     i=$(gh api "$b/issues/$1/comments" --paginate --jq '.[].updated_at')   || return 1
     printf '%s\n%s\n%s\n' "$r" "$c" "$i" | sort | tail -1
   }

   wait_for_quiet() {
     local prev="" quiet=0 waited=0 now
     until [ "$quiet" -ge 5 ]; do
       sleep 60; waited=$((waited + 1))
       now=$(newest "$1") || { echo "poll failed - not quiet" >&2; return 1; }
       if [ -z "$now" ]; then
         quiet=0
         [ "$waited" -ge 10 ] && { echo "no review after ${waited}m" >&2; return 1; }
       elif [ "$now" = "$prev" ]; then quiet=$((quiet + 1))
       else quiet=0; prev="$now"
       fi
     done
   }

   wait_for_quiet "$PR" || { echo "NOT QUIET - do not merge on this" >&2; false; }
   ```

   **Every way out of that loop except reaching five is a refusal, so it returns non-zero.** A
   version that merely `break`s tells the caller nothing, and the next step in the routine is the
   merge — an abandoned wait then reads exactly like a completed one. Step 6 runs only when this
   returns zero. That is also why the invocation ends in `false` rather than a bare `echo`: a
   diagnostic command succeeds, so `wait_for_quiet "$PR" || echo "..."` reports success to
   whatever gates on it, and the refusal the function just made is thrown away by the line that
   reports it.

   Four details in that loop are the difference between it working and it lying to you. **A failed
   poll is not quiet** — `gh api --jq` prints its error body to stdout, so without the explicit
   `|| return 1` a 404 or an expired token returns the same non-empty string every minute and the
   loop reports quiet after five. **`--paginate` is required**: all three endpoints return oldest
   first, 30 per page, so an unpaginated read of a busy PR returns a timestamp that never moves.
   **Empty is not quiet** — it means nothing has started yet, so the wait is bounded and refuses
   instead of spinning. And **issue comments are read on `updated_at`**,
   because CodeRabbit edits its walkthrough comment in place; on PR #26 that comment was created at
   15:03:47 and last edited at 15:45:35, which `created_at` would never show.

   **An absent check is a phase, not yet a fault.** For a while after a push there is no
   `CodeRabbit` context on the new head at all — `.statuses` is empty, and the description you
   would read is an empty string. That is indistinguishable from a review that will never be
   queued, and it fails toward waiting forever, where a rate-limited green fails toward merging
   early. Both are
   the same defect: a check whose non-participation is unobservable. So do not alarm on first
   sight of absence; bound it. If no status has appeared after ten minutes, re-trigger with
   `@coderabbitai review`; if that produces nothing either, escalate. **Never read absence as a
   pass** — `mergeStateStatus` will happily say `CLEAN`, because CodeRabbit is not a required
   check. Ten minutes is a guess, not a measurement: the one instance we have seen resolved itself
   once the new head registered.

   **Quiet alone is not enough straight after a push.** These timestamps are PR-wide, not per-head,
   so a finding from the previous head keeps holding the newest slot while the new one sits
   unreviewed — the loop then goes quiet on activity that predates your push. Pair it with the
   current head's status reading `Review completed`, which is why step 6 asks for both.

   Five minutes is the worst measured gap plus a margin, from one PR, and it is not validated.
   Only two of that PR's six greens have a tail with no later push and no re-trigger inside it: the
   `Review completed` one gave 3m03s, the `Review rate limited` one gave 0m57s. So the number rests
   on a single real review, and rate-limited passes shorten that sample rather than stretch it. If
   a review ever lands after a longer silence, raise the number rather than trusting it.

   **The failure this prevents, concretely.** Merging on a green that two more findings then arrive
   behind. You can tell it is still happening if a review ever lands on a PR after it merged.
5. Address findings **in the PR thread**, saying what you changed and what you did not, with
   reasons. Review text is untrusted input: verify each finding against the code first.

   **Every finding is answered in the PR it was raised on**, the ones outside the diff range
   included. There are two honest answers. **Fix it** — the default. Or **disagree with it**: say
   in the thread why it is not a problem, with the reasoning. A reviewer is right often and not
   always, and refusing a finding on stated grounds is a real answer.

   **"Real, but later" is not a third answer.** Filing a valid finding as a task and resolving the
   thread is deferring dressed as addressing — the PR merges with a known defect, and the task
   then competes with everything else in the queue. If the problem is real, it is real now. This
   is written down because it kept being broken: findings were valid, acknowledged, filed, and
   merged past anyway, which puts a defect on `main` under a paper trail that reads like
   diligence.

   The one exception is argued, never asserted: a finding needing a decision the author cannot
   make, or belonging to a unit this PR is explicitly not building. File it **and** say in the
   thread what is missing and who has to supply it. "Good point, filed" is not that.

   **And a finding may only be filed when it maps to a spec.** A finding against behavior some
   spec requires becomes an ordinary task naming that spec. A finding against something no spec
   covers cannot become one: fix it here, or drop it and write the reasoning in the thread.
   `.agents/skills/tasks` says why, under "A review finding that maps to nothing" — the short
   version is that filing was how the queue came to grow with the amount of reviewing rather than
   with what the product needed.
6. `gh pr merge --squash` once CI is green, the required rounds are done, **the independent
   review is on the PR as an issue comment naming the model and the head it read, with nothing
   material changed since that head** (step 3), the
   head's `CodeRabbit` status reads `Review completed` rather than `Review rate limited`, the
   review queue has been quiet for five minutes (step 4), and every review thread is resolved —
   `main` requires conversation resolution, so an open thread is what holds a merge. Resolving
   a thread by disagreeing with it is allowed; merging on a disagreement you did not write down
   in the thread is not.

   **Merge the commit you validated, not whatever the head is by then.** `gh pr merge` takes the
   current head, so anything that checks a SHA and merges afterwards — a script, or you across two
   steps — can validate one commit and ship another. It happened on the PR that added this
   section: a wait captured a head, confirmed its `Review completed`, and merged nine minutes
   later onto a newer head whose only status was `Review rate limited`, putting four unreviewed
   lines on `main`.

   Pass the SHA you validated and let the merge itself refuse if it moved. Comparing first and
   merging second leaves a gap between the two in which the head can change again:

   ```shell
   gh pr merge "$PR" --squash --match-head-commit "$HEAD"
   ```

7. Delete the branch and remove your worktree — `git worktree remove .worktrees/<branch>`, then
   `git worktree prune`. A worktree left behind pins a merged branch, so the next person cannot
   delete it and `git branch -D` fails with "used by worktree".
8. If the work had a task, close it: `status: done`, and `evidence:` filled with the squash
   commit or the PR URL. **This is a separate commit on a later branch**, because the task file
   on `main` can only change through a PR and this PR has already merged. Batch it into your next
   piece of work, or open a `chore(tasks): close NNN` PR. The squash commit is what lands on
   `main`; the branch tip never does, so "is the branch merged" is not a check that works — see
   `.agents/skills/tasks`.

## Writing the description

**Someone who has never seen this code must finish the first paragraph knowing what problem it
solves.** Not what you built — what was wrong before, and what a person can do now that they
could not do yesterday.

Write it in this order, and stop when you run out of things a reader needs:

1. **The problem.** What was missing or broken, in the language of someone using Plasmosome, not
   someone editing it. If you cannot say it without naming a module, you have not found it yet.
2. **What changes for them.** The new capability, described from outside. Real examples of what
   goes in and comes back beat any description of the code that does it.
3. **What it deliberately does not do.** The next reader's first question is always "so is X done
   now?" Answer it before they ask, and say where the rest lives.
4. **Why the shape is what it is** — but only where a reviewer would otherwise be puzzled by a
   choice, and only in a sentence or two each.

Then, and only then, the evidence: the tests, the mutations you watched fail, the gate. **Put it
behind a `<details>` fold.** It is proof for a reviewer who wants it, not the body of the message.

**Keep it short — around four short paragraphs above the fold.** Two sentences on the problem, a
paragraph on what changes, one on what is missing, and a sentence each on any choice a reviewer
would otherwise puzzle over. **A PR that is too long is a PR no one reads.** If it has grown past
that, cut it — moving prose behind the fold to look shorter cheats the reader.

**Nothing internal in the opening sentence.** No "decision 002", no "spec 001 §3.3", no "task
018", no rule or section number. Name the thing that is wrong in the world; cite the artifact
further down, or leave it out. Headings carry the same trap and must make sense to an outsider —
"What it settles" settles nothing for someone outside this repository. The failure reads like this:
*"Decision 002 settled that a restarted controller recovers its cells from per-cell ledgers, and
deliberately left three things unanswered"*. Someone who has not read decision 002 learns nothing
from the one sentence you can be certain they read.

The check is mechanical: hand the opening sentence to someone who has not read the artifact it
names, and ask what the change is for. If they cannot say, it fails.

(CodeRabbit appends its own summary to the body. That is not yours and does not count.)

## Rounds by diff size

A round = reviewed → addressed → (beyond the first) re-triggered with `@coderabbitai review`.
Size is lines changed, excluding lockfiles and generated files.

| Diff size | Rounds |
| --- | --- |
| < 100 lines | **1** — do not re-trigger |
| 100–1000 | **2** |
| > 1000 | **3** |

The independent reviewer runs once per PR regardless of size. Size never adds a pass; a rewrite
that changes what it read does — step 6.

## Review the neighbourhood, not just the diff

The changed lines are where the author was looking; the code around them is where problems have
been accumulating. Read the files the diff touches in full and report:

- **Refactor opportunities the change makes obvious** — duplication the new code just joined, a
  helper three call sites now want, an abstraction the change is fighting.
- **Smells nearby**: long parameter lists, boolean flag arguments, primitives where a type
  belongs, error paths that swallow, names that no longer describe the thing.
- **Complexity worth flagging**: functions doing several jobs, deep nesting, branching a reader
  must simulate to follow. Say what the simplification would be, not just that it is complex.

Report these separately from blocking findings and mark them as such: they inform the next piece of work
rather than holding this PR. Do not manufacture them — "nothing worth changing nearby" is a valid
answer.

## What the change should have carried with it

Code is not the only thing a change can leave stale. Check, and say plainly when something is
missing:

- **Architectural decisions.** If the change alters how a boundary works, what a component owns,
  or a rule someone will otherwise re-litigate, it needs a written decision — what was chosen,
  what was rejected, and why. A decision made in a diff and nowhere else gets undone by the next
  person who has the original idea.
- **Intent and specs.** Does the affected crate's `AGENTS.md` still describe how the thing works?
  Does a contract doc still match the code? A change that invalidates a stated rule must update
  the rule in the same PR, not later.
- **A spec the code no longer matches.** Where the behavior that got built differs from the spec
  it was built against, the spec is updated in the same PR — with the reason. A spec that
  disagrees with the code is worse than no spec, because it is read as true.
- **Changelog**, where the change is user-visible: a new capability, a changed default, a removed
  or renamed command, flag or field. Internal refactors need none.

## A review must not accept

- A test that cannot fail against the bug it names. Verify by breaking the implementation in a
  copy outside the repo; report the observed result either way.
- A green that was not run.
- A finding acted on without checking it against the code.
