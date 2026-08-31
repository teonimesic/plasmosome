---
id: 020
title: Rewrite the five commits on main that credit a model as an author
status: todo
priority: 2
specs: []
intents: []
refs: [.githooks/attribution-guard]
done_when: >-
  no commit reachable from main carries a Co-Authored-By trailer naming a model,
  and GitHub's contributor list for the repository shows only people.
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
