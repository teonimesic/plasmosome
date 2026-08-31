---
id: 020
title: Rewrite the five commits on main that credit a model as an author
status: todo
priority: 2
specs: []
intents: []
refs: [.githooks/attribution-guard]
done_when: >-
  `./.githooks/attribution-guard main` prints `attribution-guard: clean`, and GitHub's
  contributor list for the repository shows only people.
pr:
evidence:
---

## Why

Five commits on `main` carry `Co-Authored-By: Claude …`, including the repository's first commit.
GitHub reads that trailer and lists the model as a contributor, so the repository publicly credits
a tool as an author. A person is accountable for every commit here; a model is something that was
used, like an editor or a compiler.

The guard added alongside this task stops new ones. It cannot touch what has already landed,
because these are on `main` and rewriting them means force-pushing a protected branch.

```text
11a94c3 feat(core): the controller answers its control socket (#14)
57fdbc4 docs: put the working rules in the repo as agent skills (#2)
0250ad3 test(membrane): prove the no-orphan contract holds under signal pressure (#1)
fb952db docs: per-crate README, AGENTS and CLAUDE for every crate
002e821 feat: Plasmosome — a composable, OS-enforced capability kernel for AI agents
```

**Do not start this while any pull request is open.** A rewrite changes every commit hash from the
oldest rewritten commit forward, so every open branch is left pointing at commits that no longer
exist and has to be rebuilt by hand. The cost is small when nothing is in flight and large when
anything is. Wait for an empty `gh pr list`.

The mechanics are `git filter-repo --message-callback` or an interactive rebase from the root
commit, then a force-push with branch protection briefly lifted. Both need care around the root
commit, which is one of the five.

## Plan

## Notes

**The five commits carry twelve trailer lines between them, not five.** A reword that deletes one
line per commit leaves eight behind. The count the guard reports today:

```text
002e821  1
0250ad3  1
11a94c3  2
57fdbc4  7
fb952db  1
```

`57fdbc4` and `0250ad3` are squash merges, so the message is every squashed commit's message
concatenated. Each contributes its own trailer, and all but the last sit mid-body.

**The guard used to see only three of the five.** It read trailers with git's trailer parser
(`%(trailers:...)`), which recognises only the block that closes a message, so the mid-body
trailers in `57fdbc4` and `0250ad3` were invisible to it. That was a bypass as much as a
miscount: a squash merge is the ordinary way a commit reaches `main` here, and one carrying a
model trailer passed the control meant to stop it. The guard now offers every paragraph to that
same parser, and reports all five. Verify the count before and after the rewrite with the command
in `done_when` — it is the same instrument in both places.
