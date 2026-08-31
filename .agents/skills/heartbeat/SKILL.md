---
name: heartbeat
description: The start-of-session routine — reconcile pending PRs, reviews, tasks and specs against reality, then pick the next piece of work. Use at the start of every working session, and whenever you are asked what to do next.
---

# The heartbeat

Run this at the start of every working session, in the order given. It ends by handing you the
next piece of work.

It exists because nothing that matters is remembered — everything that matters is a file. This
routine replaces the session to-do list entirely. A list held in a chat window disappears when
the session does, and the next agent starts from nothing.

The order is not arbitrary: finish what is already in flight before starting anything new. Steps
1 and 2 find the work that is one reply away from merging and get it moving again; step 3 makes
the queue trustworthy; step 4 finds work that was described and then quietly dropped; step 5 clears
the worktrees that only look busy so step 6 can count what is really running; only then do you pick.
The formats and the layers those files use are in `.agents/skills/tasks`.



**1. Pending PRs.**

```shell
gh pr list --state open
```

For each one, answer three questions: is CI green, are the review rounds for its diff size done,
and are all conversations resolved? `main` requires conversation resolution, so a single
unresolved thread is what is blocking the merge — not CI, and not a missing approval.

**2. Pending reviews.** `gh pr view --json` has no `reviewThreads` field; review threads come
from the GraphQL API only.

```shell
gh api graphql -f query='{repository(owner:"teonimesic",name:"plasmosome"){
  pullRequest(number:NNN){reviewThreads(first:100){pageInfo{hasNextPage}
  nodes{id isResolved path comments(first:1){nodes{body}}}}}}}' \
  --jq '.data.repository.pullRequest.reviewThreads|
        if .pageInfo.hasNextPage then "MORE PAGES — do not treat this queue as clear" else empty end,
        (.nodes[]|select(.isResolved==false))'
```

Read `pageInfo` before believing an empty result: a PR with more threads than one page returns a
clear-looking queue that is not clear. The query fails loudly instead.

The threads still open are the queue, and clearing it comes before opening anything new. They
belong to the agent that wrote the PR: resume it and let it reply, fix and resolve in the thread.
A fix pushed without a reply leaves the merge blocked, and a thread answered by anyone else costs
the review the independence it was for.

**3. Pending tasks.** Reconcile `tasks/` against reality: the open PRs from step 1 and the
`task-*` branches on the remote.

```shell
git ls-remote --heads origin 'task-*'
```

**Ask the PR first, before anything else.** A squash merge deletes the branch and closes the PR,
so a finished task looks exactly like an abandoned one — no branch, no open PR. Releasing on that
evidence alone recycles work that already shipped.

```shell
gh pr view <number> --json state,mergeCommit
```

For every task carrying a `pr:`: `MERGED` means set `done` and record the merge commit in
`evidence:`. Only a `CLOSED` PR, or a claim with no `pr:` at all and no branch, may go back to
`planned`. Record in `## Notes` what established the truth. A stale queue is worse than no queue,
because people believe it.

**4. Pending specs, and unmapped work.** Work that was described and then dropped hides in two
places: a spec still in draft that no task implements, and an intent with no spec at all.

```shell
for f in docs/specs/*.md; do
  grep -q '^status: draft' "$f" || continue
  id=$(sed -n 's/^id: *//p' "$f" | head -1)
  grep -lq "^specs:.*\b$id\b" tasks/*.md 2>/dev/null || echo "spec $id: draft, no task implements it"
done

for f in docs/intents/*.md; do
  id=$(sed -n 's/^id: *//p' "$f" | head -1)
  grep -lq "^intents:.*\b$id\b" docs/specs/*.md 2>/dev/null || echo "intent $id: no spec written"
done
```

Both loops derive the ids from the files, so a record numbered anything is covered. Silence means
nothing is pending. For anything they print, decide out loud: dispatch a planner for it, or say
why not. The decision is yours; the plan is not.

Then read the chain the other way, for work nothing asked for:

```shell
grep -l '^specs: \[\]' tasks/*.md
grep -l '^intents: \[\]' docs/specs/*.md
```

These are not planner dispatches. A task naming no spec may not be planned and so may not be
started, and a spec naming no intent may not be planned either — so for each one the answer is to
map it to something that already exists, to put the question to the owner in your own words, or to
recommend dropping it. **Never close the gap by writing an intent yourself**; that is the one move
`.agents/skills/tasks` rules out, because an intent written to justify a filed task approves the
work on the owner's behalf.

Both lists shrinking over sessions is the signal that the queue is being fed by the plan. Both
growing is the signal it is being fed by the review process instead.

**5. Clean up.** Remove the worktrees of branches that have merged, and prune. A worktree left
behind pins its branch and blocks the next person from deleting it. **This runs before the count
and not at the end of the session**, because a skipped cleanup and a working agent look identical
from the directory — reorder the two and the next step counts finished work as live and dispatches
too little.

The directory cannot tell you which is which; GitHub can. A worktree whose branch has an open PR is
live work even when nobody is typing in it, and one whose branch merged is not.

```shell
git worktree list --porcelain |
  awk '/^worktree /{p=$2}
       /^branch /{sub("refs/heads/","",$2); if (p ~ /\/\.worktrees\//) print p"\t"$2}
       /^detached$/{if (p ~ /\/\.worktrees\//) print p"\t-\tDETACHED"}' |
  while IFS="$(printf '\t')" read -r dir branch known; do
    [ -n "$known" ] && { printf '%s\t%s\t%s\n' "$dir" "$branch" "$known"; continue; }
    state=$(gh pr list --head "$branch" --state all --limit 100 --json state \
      --jq 'if any(.[]; .state=="OPEN") then "OPEN"
            elif length==0 then "NONE"
            elif any(.[]; .state=="MERGED") then "MERGED"
            else "CLOSED" end') || state=UNREACHABLE
    printf '%s\t%s\t%s\n' "$dir" "$branch" "${state:-UNREACHABLE}"
  done
```

Remove the `MERGED` rows, and the ones you have argued into `DROP` below:

```shell
git worktree remove <path> && git worktree prune
```

**Remove by path, never by branch.** Worktree directories are named by whoever created them and
rarely match the branch — `.worktrees/task-016` holds `task-016-work`, and a `docs/x` branch would
need a nested directory that does not exist. That is why the listing prints the path first.

**Finish by sorting every row into one of four buckets, and write the result down** — in your
report, or a scratch file. Step 6 reads the buckets and never the states, and a classification held
only in your head does not survive the gap between the two steps.

**Everything starts in `SETTLE` and has to be argued out of it.** A state added to this pipeline
tomorrow therefore blocks the count rather than being silently skipped, which is the safe
direction.

- **`REMOVE`** — a row whose PR is `MERGED`. The cleanup above is this bucket.
- **`LIVE`** — a row an agent is actually working in. An `OPEN` PR is the usual sign, but it is
  not proof: a PR with unanswered threads and nobody on it is the stalled work step 2 sends back
  to its author, not a slot in use. It becomes `LIVE` when you have resumed that author. A
  `CLOSED` or `NONE` row counts the same way, on the same evidence.
- **`DROP`** — a row you have established nobody is in and nothing is coming back to: litter from
  a dead agent, or a `CLOSED` PR nobody is reworking. Remove it the same way, but only on that
  evidence — `MERGED` needs no argument, this does.
- **`SETTLE`** — everything you have not established either way, and the only bucket you may not
  leave rows in. `CLOSED` is abandoned, not finished: step 3 puts that work back on the queue and
  the branch may be being reworked right now. `NONE` means nothing was ever pushed — an agent
  mid-first-change, or litter. `DETACHED` and `UNREACHABLE` are answers you did not get, and
  GitHub being unreachable is not the same as a branch having no PR. Ask whoever owns it, or look
  at who is in the directory.

**Never let the next step work out liveness from the states itself.** That is the seam three
review findings in a row landed on: `CLOSED`, then `NONE`, then `DETACHED` and `UNREACHABLE` were
each preserved here as possibly-live and then left out of the count, which would have dispatched an
agent on top of work nobody could see. You can tell this is still happening if a heartbeat ever
dispatches while a row sits in `SETTLE`.

**6. Agents running.** Dispatching is the one thing only the orchestrator does, so it is the
constraint on how much work is ever in flight. **Three running in parallel is the standing goal.**

**Count step 5's `LIVE` bucket.** Nothing else: do not re-list the worktrees, do not count task
files, and do not re-derive liveness from the states — step 5 already decided that, and deciding it
twice is what has gone wrong before.

**A row still in `SETTLE` means you do not have a count.** An incomplete count reads as spare
capacity and puts another agent on top of work you cannot see. Empty that bucket first.

```shell
grep -lE '^status: (in_progress|in_review)' tasks/*.md
```

That is a cross-check, not the count, and it is loose in both directions. A task claiming either
status with no `LIVE` row behind it is a stale claim — go back to step 3 and reconcile it now,
rather than leaving it for a later session. A `LIVE` row with no task file behind it is ordinary:
not every branch has one.

However many short of three you are is how many tasks you dispatch, and step 7 picks that many
rather than one.

**Pick tasks that will not write the same files.** Two agents editing one file produce two PRs that
fight, and whichever merges second pays for it. `refs:` is the first thing to compare, but it lists
what an executor must *read*, so it neither proves a collision nor rules one out — read the
`## Plan` for what the work will actually change.

**Review throughput is the real ceiling, not agent count.** The review budget is repo-wide —
roughly ten an hour across every PR — so three agents pushing hard exhaust it for everyone, and
the next PR to land gets a green that reviewed nothing. Count the reviews already spent this hour
before dispatching a third, and prefer one agent on a large piece of work to three on small ones
when the window is nearly gone. Four authors dispatched into an exhausted window produce four
unreviewed merges, not four reviewed ones.

If the queue does not hold three that clear that bar, dispatch what it does hold and say so in the
report. Do not manufacture overlapping work to reach the number, and name which constraint bound:
an empty queue and a queue where everything collides with something running need different fixes.

**The failure this prevents, concretely.** One agent running while the queue held unblocked tasks
touching different files — capacity sitting idle with nothing technical in the way. You can tell
whether this step is working by whether a heartbeat ever ends with one agent live and two
dispatchable tasks left untaken.

**7. Pick.** Look at `planned` before `todo`: a `planned` task already has a plan someone wrote,
so it is ready to hand to an executor, while a `todo` still needs planning. Within each, take the
lowest `priority:` number, not the newest one — as many as step 6 said you were short.

```shell
grep -l '^status: planned' tasks/*.md
grep -l '^status: todo' tasks/*.md
```

Step 3 puts released claims back to `planned`, so this is also how abandoned work returns to
circulation.

**8. File.** Anything you learned this session that must outlive it becomes a task file before the
session ends — dispatch a planner to write it. The only writing into `tasks/` you do yourself is
step 3's reconciliation, and only for a claim whose author is gone: an author still open closes its
own task, as `.agents/skills/pr-review` has it.
