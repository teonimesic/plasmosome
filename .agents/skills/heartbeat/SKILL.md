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
1 and 2 close out work that is one reply away from merging; step 3 makes the queue trustworthy
again; step 4 finds work that was described and then quietly dropped; step 5 clears the worktrees
that only look busy so step 6 can count what is really running; only then do you pick.
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

**4. Pending specs.** Work that was described and then dropped hides in two places: a spec still
in draft that no task implements, and an intent with no spec at all.

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

**5. Clean up.** Remove the worktrees of branches that have merged, and prune. A worktree left
behind pins its branch and blocks the next person from deleting it. **This runs before the count
and not at the end of the session**, because a skipped cleanup and a working agent look identical
from the directory — reorder the two and the next step counts finished work as live and dispatches
too little.

The directory cannot tell you which is which; GitHub can. A worktree whose branch has an open PR is
live work even when nobody is typing in it, and one whose branch merged is not.

```shell
git worktree list --porcelain |
  awk '/^worktree /{p=$2} /^branch /{sub("refs/heads/","",$2); if (p ~ /\/\.worktrees\//) print $2}' |
  while read -r b; do
    printf '%s\t%s\n' "$b" "$(gh pr list --head "$b" --state all --json state \
      --jq 'if any(.[]; .state == "OPEN") then "OPEN" elif length == 0 then "NONE" else "FINISHED" end')"
  done
```

`FINISHED` is work that is over. `OPEN`, or no PR at all, is live and stays. A branch can carry more
than one PR — a closed attempt and the open one that replaced it — so the state has to be read
across all of them, not off the first: reading only the first removes a worktree an agent is still
using. The `awk` filter keeps the primary checkout out of the list — it is nobody's agent, and
counting it puts you one over every time.

```shell
git worktree remove .worktrees/<branch> && git worktree prune
```

**6. Agents running.** Dispatching is the one thing only the orchestrator does, so it is the
constraint on how much is ever in flight. What reaches `main` is limited by that, and one agent at a
time means the queue moves at the speed of one agent. **Three running in parallel is the standing
goal.** Fewer than three is a problem to fix here, not a state to note and move past.

**The live rows from step 5 are the count.** Do not re-list the worktrees here, and do not count
task files instead — each row is an agent at work or an open PR, which is what "running" means.

```shell
grep -lE '^status: (in_progress|in_review)' tasks/*.md
```

That is a cross-check, not the count. Both statuses are lines someone wrote: a task claiming either
one with no live row behind it is a stale claim for step 3 to reconcile, not an agent. However many
short of three you are is how many tasks you dispatch in the next step, taken from the unblocked
work the queue already holds.

**Disjointness is the constraint, not a nicety.** Two agents editing the same file produce two PRs
that fight, and whichever merges second pays for it. Compare the `refs:` of the candidates and take
ones that do not overlap — with each other, and with what is already running. The floor is three
disjoint tasks, never three tasks.

If the queue does not hold three unblocked disjoint tasks, say so plainly in the report and dispatch
what it does hold. Do not manufacture overlapping work to reach the number. Coming up short is a
real signal: either the queue needs filing, or something is blocking more than it should.

**7. Pick.** Look at `planned` before `todo`: a `planned` task already has a plan someone wrote,
so it is ready to hand to an executor, while a `todo` still needs planning. Within each, take the
lowest `priority:` number, not the newest one.

```shell
grep -l '^status: planned' tasks/*.md
grep -l '^status: todo' tasks/*.md
```

Step 3 puts released claims back to `planned`, so this is also how abandoned work returns to
circulation.

**8. File.** Anything you learned this session that must outlive it becomes a task file before the
session ends — dispatch a planner to write it. Reconciling `status:` and `evidence:` in step 3 is
the only writing into `tasks/` you do yourself.
