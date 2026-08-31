---
id: 026
title: A rate-limited green looks exactly like a review
status: todo
priority: 1
specs: []
intents: []
refs: [.agents/skills/pr-review/SKILL.md, .agents/skills/heartbeat/SKILL.md]
done_when: >-
  the merge gate distinguishes a review that ran from one that did not. Following
  the written procedure on a PR whose head carries `CodeRabbit / success / Review
  rate limited` stops the merge and says why; on `Review completed` it does not.
  Whatever check is added is stated as a command someone can run, and the two
  merged PRs below are the worked example.
pr:
evidence:
---

## Why

The `CodeRabbit` commit status reports `success` when the review was skipped. The state is green
either way; only the description tells them apart:

- `Review completed` — the head commit was read.
- `Review rate limited` — nothing was read.

Every check the repository asks for reads the state. `gh pr checks --watch` blocks until checks
settle and then reports green. Step 6 of `.agents/skills/pr-review` merges on "CI is green, the
required rounds are done, and every review thread is resolved". None of that can tell the two
apart, so a PR can satisfy the whole procedure with its final commit unread.

It has already happened twice, on the merged heads of PR #1 and PR #26:

```
gh api repos/teonimesic/plasmosome/commits/<head-sha>/status \
  --jq '.statuses[] | select(.context=="CodeRabbit") | .description'
```

Both answer `Review rate limited`; the other 23 merged PRs answer `Review completed`. In both
cases earlier commits on the branch were reviewed and the last one was not, which is the shape
that hides best — the PR has a review history, findings were raised and answered, and only the
final state went unread.

- **PR #1** merged `28994d15` unreviewed: the signal-storm rewritten from an armed flag to a
  budget plus `pthread_sigmask`. The three earlier commits each drew a review; CodeRabbit's own
  review-info block on the third says "1 remains after this review", so the quota was gone before
  the last push. That commit has since been checked by hand — reverting `Drop` to the single-shot
  `waitpid` still turns the test red, so the redesign kept its power — but nothing in the process
  did that checking.
- **PR #26** merged `83458db1`, and the last three commits, unreviewed: 51 lines across
  `AGENTS.md` and three skills, including a rule about how every finding must be answered and a
  paragraph narrowing what decision 001's evidence requirement binds. Those are live rules that
  every agent now follows, and no reviewer read them.

**What makes it worth a `1`.** The other tasks in this queue are defects that a review would have
caught. This is the reason a review did not happen, and it is invisible from every signal the
merge procedure uses. Nothing in the PR record marks a rate-limited PR, so the only way to find
one is to go looking, which nobody has a reason to do.

**A hint about when it strikes, not a law.** On PR #26 the last three commits went up at 15:42:00,
15:42:38 and 15:45:20 — three pushes inside about three minutes. PR #1 is not that shape: four
pushes over 43 minutes, defeated instead by the hourly quota being spent across concurrent PRs.
So rapid pushes are one way in and not the only one, and a fix that only slows pushing down will
not cover the second case.

**What this is next to.** PR #26 closed the sibling hole in the same file: step 4 of
`.agents/skills/pr-review` now says a green check and an empty thread queue are not a clean pass,
because findings outside the diff go in the review body, and it gives the query for reading the
reviews themselves. That closes "the review ran and you did not read all of it". This is "the
review did not run", and the same step is where it belongs. PR #31 is editing that step; whoever
takes this rebases on it.

## Plan

## Notes
