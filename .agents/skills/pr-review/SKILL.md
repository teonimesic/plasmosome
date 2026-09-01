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

   **One kind of PR you never mark ready: one that proposes an intent, or moves one to
   `status: approved`.** It stays a draft until the owner has read it here and approved it, and
   they are who takes it out of draft. This is where their reading actually happens, so the draft
   flag is where the waiting is visible — `gh pr list` shows it, and no agent has a reason to flip
   it on their behalf. It is a convention and not a boundary: an agent *can* mark one ready, just
   as it can write `approved` into a file. See
   `docs/decisions/008-approving-an-intent-is-an-instruction.md`.

   **The same wait reaches one layer down: a change whose chain reaches a goal the owner has not
   approved is not yours to take out of draft either.** Walk the pull request's task to the spec it
   names, and that spec to its intent. Where that intent's `status:` is not `approved`, the change
   descends from something nobody has agreed to want, and it stays a draft until that intent reads
   `status: approved`. **The wait ends when the owner approves the intent, and in no other way** —
   not on a conversation an agent judges to have settled it, and not on the agent's own reading of
   the goal. If the answer is that the goal was never wanted, that is the owner's to say on the
   intent, not something to settle here. Why nothing mechanical holds any of this is
   `docs/intents/README.md`, which has the rule in full, and
   `docs/decisions/008-approving-an-intent-is-an-instruction.md`; this adds only the reach.

   Unlike the rest of this section the walk is mechanical — a walk over `specs:` and `intents:`,
   both of which are lists and may hold more than one id, ending on an intent's `status:` — so do
   it rather than assume it. **It binds today and refuses nothing today, and both halves are
   worth knowing.** It binds because `docs/intents/` carries a real `status:`, `draft` or
   `approved`, rather than conferring approval by the file existing. It refuses nothing while
   `grep -L '^status: approved' docs/intents/[0-9]*.md` stays empty, as it is today, because then
   no chain can reach a draft intent. **The numeric prefix is load-bearing**: `docs/intents/*.md`
   also matches `README.md`, which carries no `status:` line, so `-L` reports it forever and the
   check never reads empty. (`grep -l` does not have this problem — a `README.md` simply does not
   contain the pattern — which is why the neighbouring draft-spec probe needs no such prefix.)

   The first draft intent that acquires a spec and a task is when this starts stopping something,
   and you can tell it is working by whether that pull request is still a draft.

   What the walk does not reach is an incomplete chain. A spec with an empty `intents:` is a spec
   that skipped the gate, and a task naming no spec is a mapping question; `.agents/skills/tasks`
   has both rules and this paragraph restates neither.
   What matters here is only that none of them is an unapproved goal, so none of them holds a
   pull request in draft.
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

   **A green has four meanings, and two of them are "no review happened."** The `CodeRabbit`
   context reports `success` whether it reviewed and found nothing, reviewed and posted findings,
   never ran at all, or was skipped for a draft — two of that PR's six greens carried the
   description `Review rate limited`. Only the description tells them apart:

   ```shell
   gh api repos/teonimesic/plasmosome/commits/<sha>/status \
     --jq '.statuses[] | select(.context=="CodeRabbit")
           | "\(.updated_at) \(.state) \(.description)"'
   ```

   `Review completed` is the only one that is a review. **`Review rate limited` is not a pass and
   will not become one** — waiting is pointless, because nothing more is coming. Re-trigger with
   `@coderabbitai full review`, because the head has not moved since it was rate limited, and wait
   for a `Review completed` on the current head. Merging on a
   rate-limited green ships a change nobody reviewed, and neither the check state nor an empty
   thread list will ever say so.

   **`Review skipped: draft pull request` is the other green that is not a review**, and a draft
   collects it per push rather than once per pull request. Measured while this was written: the
   three drafts open at the time — #44, #52 and #59 — each carried that description with a `Review
   queued` before it, and on #59 three of four pushed heads collected the pair while the fourth
   collected no `CodeRabbit` status at all, still none minutes later. So it is not something to
   wait out and not something to re-trigger; it is what a draft looks like, and marking the pull
   request ready is what queues a real review. Its absence on a draft is not a fault either, which
   is the absent-check paragraph below. This matters more than it used to: a pull request that
   waits in draft for an approval, as step 2 now has some of them do, sits on one of those two
   states for as long as it waits, and neither of them is a review.

   **The first of those four is invisible to the reviews endpoint, which makes that endpoint
   useless for this question.** A review that finds nothing creates **no review object at all** —
   it posts its walkthrough as an issue comment and stops. The endpoint returns zero entries, not
   an entry with an empty body. PR #39 read `Review completed` with zero reviews and one issue
   comment; #34 and #36 read the same status with two and four. So zero reviews cannot tell a
   clean pass from a review that never ran, and to anything that counts them the two are
   identical.

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

   **`@coderabbitai review` does nothing when the head has not changed.** The reviewer is
   incremental, and on a commit it has already read it declines:

   > Already reviewed the last commit. Use `@coderabbitai full review` to rerun a review of the
   > entire changeset.

   **Every surface you would poll reads as though the round happened.** That refusal arrives
   inside a collapsed `<details>` block in a short bot comment, so nothing about the comment's
   visible line says it is a refusal. It edits the walkthrough, which moves that comment's
   `updated_at` — the field `wait_for_quiet` reads issue comments on — so the loop sees activity,
   then sees it settle, and reports quiet on a content-identical refresh. And the head's status
   still reads `Review completed`, left there by the round before. Both of step 6's review
   conditions are satisfied by a round that never ran.

   **So on an unchanged head the command is `@coderabbitai full review`.** Use it whenever you are
   asking for a round rather than reacting to a push: a second round after a clean first one, a
   retry after a rate-limited green, and the ten-minute escalation below. Plain
   `@coderabbitai review` is for a head carrying commits it has not seen, which is the case that
   triggers itself anyway.

   **Then poll the status history growing, not a timestamp moving.** Take the count before you
   ask and after:

   ```shell
   completed_on() {
     local pages
     pages=$(gh api "repos/teonimesic/plasmosome/statuses/$1" --paginate --slurp) || return 1
     printf '%s' "$pages" |
       jq '[.[][] | select(.context=="CodeRabbit" and .description=="Review completed")] | length'
   }

   rounds() {
     local sha total=0 n
     local url="repos/teonimesic/plasmosome/pulls/$1/commits"
     for sha in $(gh api "$url" --paginate --jq '.[].sha'); do
       n=$(completed_on "$sha") || return 1
       total=$((total + n))
     done
     printf '%s\n' "$total"
   }
   ```

   **`rounds` takes a pull request number and `completed_on` a commit, and the split is the whole
   point.** Rounds accumulate across heads; completed statuses accumulate per head. Address a
   finding and the head moves, so the new head carries the one round that read it and no memory of
   the ones before. Counting on the merged head alone therefore asks a small pull request that
   fixed something to show two rounds on a commit that can only ever have one — a condition nothing
   honest satisfies, whose only escape is re-triggering until the number comes up. That would spend
   the repo-wide budget to re-prove a round already paid for.

   Measured across merged pull requests, `rounds` against the head each one merged on:

   | PR | Lines | `rounds <pr>` | `completed_on <merged head>` |
   | --- | --- | --- | --- |
   | #42 | 51 | 2 | 1 |
   | #57 | 139 | 2 | 1 |
   | #58 | 239 | 3 | 1 |
   | #61 | 337 | 6 | 2 |
   | #43 | 483 | 6 | 2 |

   The right-hand column is what a head-only count sees: it refuses #57 and #58 outright, both of
   which owed two rounds and had them. **Force-pushing is the limit of this.** `pulls/<n>/commits`
   lists the commits the branch holds now, so rounds spent on a head that was later rebased away
   are not counted and the number reads low. That is the safe direction — it never credits a round
   that did not happen — but on a heavily rebased branch expect to argue the count up from the
   status histories rather than down.

   **`--paginate` applies `--jq` per page, which is why this one does not use `--jq`.** A commit
   with more than one page of statuses would print a count per page and the caller would read the
   first. `--slurp` returns the pages as an array of arrays — `.[][]` flattens them — and `gh`
   refuses `--slurp` together with `--jq`, so the filter moves into a separate `jq`. That pipe is
   why the call is assigned first and its status read bare: a failed poll must return non-zero
   rather than a number, for the reason `wait_for_quiet` spells out below. A commit is unlikely to
   reach a second page; none of this costs anything.

   An unknown commit returns an empty list rather than a 404, so `rounds` answers `0` for it. That
   is the safe direction — nothing merges on a zero — but it does mean a mistyped SHA reads as an
   unreviewed one rather than as an error.

   `/commits/<sha>/status` collapses a context to its latest value, so a second round leaves it
   reading exactly like the first; `/statuses/<sha>` lists every entry posted against that commit,
   so a round that ran adds to it. One head measured while this was written held five entries
   there and one in the combined status.

   **Count the completed ones, not the entries.** That history also grows on `Review queued`,
   `Review in progress` and the skipped green, none of which is a round — a count of everything
   moves when nothing has been reviewed, which is the failure this whole section is about, one
   endpoint further in.

   **This bites hardest when the first round was clean.** Nothing to fix means nothing pushed,
   which means an unchanged head, which means the request for the second round is the one that
   gets declined — so the pull requests that skip a required round are the ones that looked best.
   A 233-line change owing two rounds came within a step of merging on one round this way, and the
   second round, once it had actually been obtained, found a real defect.

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
   `@coderabbitai full review` — the head has not moved, so the plain form is declined; if that
   produces nothing either, escalate. **Never read absence as a
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

   **Where the finding is not a defect but a goal nobody has written down**, the reasoning can go
   into a `draft` intent instead of only into the thread. That is still the drop: nothing is
   started, and it becomes work only if the owner approves it and a spec names it. It is not a
   route to filing the task anyway.
6. `gh pr merge --squash` once CI is green, the required rounds are done, **the independent
   review is on the PR as an issue comment opening with the literal `Model:` marker and the head
   it read, with nothing changed since that head beyond what that review asked for** (step 3; for
   a rebase-only move, step 4's diff-of-diffs settles it), `rounds <pr>` from step 4 returns at
   least the number "Rounds by diff size" requires **and** `completed_on <head>` is at least 1, so
   that enough review happened and some of it read the commit you are merging — **read the history
   for both, never `/commits/<sha>/status`, which collapses the context to its latest value, so a
   re-trigger that comes back rate limited erases a round that really happened** — the head's
   `CodeRabbit` status reads
   `Review completed` rather than `Review rate limited` — **that condition can only add rounds,
   never remove one**, so where it and the table disagree, take the larger; "Rounds by diff size"
   says what it costs — the review queue has been quiet for
   five minutes (step 4), and every review thread is resolved — `main` requires conversation
   resolution, so an open thread is what holds a merge. Resolving a thread by disagreeing with it
   is allowed; merging on a disagreement you did not write down in the thread is not.

   **And the pull request has a task.** `.agents/skills/tasks` has the rule, what it costs, the
   two shapes that carry no task, and what to do when a pull request arrives here without one.

   **That condition is checked here rather than by a script, and the choice is deliberate.** Two
   forms of it are mechanical, and neither is honest as a gate. *Touches a file under `tasks/`*
   refuses two legitimate shapes outright: a spec's pull request touches no task file by design,
   because the task rides the work branch that follows it, and an intent's touches none either.
   *Names one in the body* lives somewhere the guards in `.githooks/` cannot read at all, and
   somewhere the squash-merge run on `main` does not have, so it could only ever answer on one of
   the two events CI sees. A guard that refuses real work is worse than a rule agents read, and it
   is worse in a way that is hard to undo: the first false refusal teaches everyone to reach for
   the bypass.

   So it sits in this list, with the other conditions that need a judgement, and
   `.agents/skills/heartbeat` step 1 looks for it again on every open pull request — one check at
   the point of merging, one that repeats. Neither is a boundary. Both are read.

   **Three of the conditions above are what a merged pull request carries, and it is worth naming
   them as three:** a link upward, a review that read the commit being merged, and an answer to
   every thread. **The middle two bind to the commit, not to the pull request** — a review that
   read an earlier head reviewed a different change, and a thread answered on an earlier head may
   have been reopened by what came after, so neither is a property the pull request keeps once its
   head moves. The link upward binds to the change instead, and survives a new head.

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

A round = reviewed → addressed → (beyond the first) re-triggered. `@coderabbitai review` is the
form to use there, because addressing a finding moved the head; where nothing moved it, step 4 has
the form that is not declined.
Size is lines changed, excluding lockfiles and generated files.

| Diff size | Rounds |
| --- | --- |
| < 100 lines | **1**, plus one more each time addressing findings moves the head |
| 100–1000 | **2** |
| > 1000 | **3** |

**Where this table and the merge gate disagree, the merge gate wins.** Step 6 requires the commit
you merge to read `Review completed`, and addressing a finding makes a new commit. A small pull
request whose one round finds something therefore owes a second review, on the head that carries
the fix — and a third if that one finds something too, for the same reason and with no ceiling
other than a round that changes nothing. Answering a finding by written disagreement moves no head
and owes nothing further. The old text told it not to re-trigger, and the two cannot both be
honoured: an agent following the table merges bytes no review has read, and an agent following
step 6 spends a round the table forbids. The table is the line that gives way.

**The cost is real and is not being rounded off.** That pull request now spends two reviews out of
a budget that is repo-wide and roughly ten an hour, and the **1** above is only reachable when the
round finds nothing — which is to say the number in the table is a floor and not a rule. It is
chosen anyway, because the alternative is merging a change no review ever read, which is the
failure the whole of step 4 exists to prevent. A clean small pull request still costs exactly one.

The independent reviewer is separate from the rounds above and runs at least once per PR. Diff
size never adds an independent pass; a rewrite beyond what that review asked for does — step 6.

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
- An intent moved to `status: approved` in the same change as the work it authorizes, or in a PR
  that does not say where the owner's approval came from. Nothing mechanical checks either — see
  `docs/decisions/008-approving-an-intent-is-an-instruction.md`.
