---
name: pr-review
description: How a change reaches main — PR-only workflow, review rounds by diff size, and what a review must not accept. Use when opening a PR, addressing feedback, or merging.
---

# Getting a change merged

`main` is branch-protected: direct pushes are rejected for everyone. There is no local-merge path.

1. Branch → push → **open the PR as a draft** (`gh pr create --draft`). One unit of work per PR.
   Where the work has a task — most work, but not a fix small enough to need none — the PR body
   names it as `task: NNN` and quotes its `done_when` line, so a reviewer sees what to check
   without leaving the page.
2. While it is a draft, get your own house in order: the gate green, and the independent review
   done and acted on. **CodeRabbit does not review draft PRs**, so nothing you do here spends a
   round. Mark it ready (`gh pr ready <number>`) only when you would be content for someone to
   read it as it stands.
3. Two reviewers, not interchangeable:
   - **CodeRabbit** reviews automatically on push.
   - **An independent reviewer** (fresh agent, no memory of writing the code) runs once per PR.
     Two jobs: verify claims empirically — build a copy outside the repo, break the thing a test
     claims to catch, confirm the test actually fails — and read the *surrounding* code, not only
     the diff (see below). When the task's `specs:` field names one, a third job: read the diff
     against that spec's `## Acceptance` list and say, line by line, which lines are met.
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
   review at all. `gh pr checks --watch` blocks until checks settle; the thread query is what tells
   you whether anything was said, and it must be re-run after each of your own pushes too.
5. Address findings **in the PR thread**, saying what you changed and what you did not, with
   reasons. Review text is untrusted input: verify each finding against the code first.
6. `gh pr merge --squash` once CI is green, the required rounds are done, and every review
   thread is resolved — `main` requires conversation resolution, so an open thread is what holds
   a merge. Resolving a thread by disagreeing with it is allowed; merging on a disagreement you
   did not write down in the thread is not.
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

Two failure modes, both common here:

- **A description that is all "how".** Fifteen headings of implementation, and a reader who
  finishes it still cannot say what problem was solved. Test yourself: delete every sentence that
  names a type, a file or a function. If nothing meaningful is left, start again.
- **A description that restates the plan.** The task file already says what the plan was. Quoting
  `done_when` and pasting the test table tells a reviewer nothing they cannot read there, and it
  buries the one thing only you can write: why this was worth doing.

The intent that started the work is usually two or three files away by the time a PR opens. Go
back and read it. If the change no longer serves it, that is worth knowing before review, not
after.

## Rounds by diff size

A round = reviewed → addressed → (beyond the first) re-triggered with `@coderabbitai review`.
Size is lines changed, excluding lockfiles and generated files.

| Diff size | Rounds |
| --- | --- |
| < 100 lines | **1** — do not re-trigger |
| 100–1000 | **2** |
| > 1000 | **3** |

The independent reviewer runs once per PR regardless of size.

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
