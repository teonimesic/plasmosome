---
id: 032
title: Every pull request has a task, and three review rules that contradicted each other
status: done
priority: 2
specs: [012]
intents: [008]
refs:
  [
    .agents/skills/pr-review/SKILL.md,
    .agents/skills/tasks/SKILL.md,
    .agents/skills/heartbeat/SKILL.md,
  ]
done_when: >-
  `.agents/skills/pr-review` states that the commit being merged reads `Review completed` even
  where the rounds table would forbid a second round; states that `@coderabbitai review` is
  declined on an unchanged head, names `@coderabbitai full review` for that case, keys the round on
  the status history growing, and counts rounds across the pull request's commits rather than on
  one head, so a change that addressed a finding is not asked to show more rounds on the merged
  head than a single head can carry; and states that a pull request whose chain reaches an
  unapproved intent stays a draft. `.agents/skills/tasks` requires a task of every pull request,
  says the pull request may file its own, states what the chain then costs, no longer exempts a
  change by size or by kind, names the two shapes that carry no task, distinguishes them — an
  intent's pull request has nothing above it, a spec's names an intent and lacks only a task —
  and says that list is closed by `docs/specs/012-how-work-enters-the-tree.md` rather than by the
  skill. `.agents/skills/heartbeat` step 1 asks whether an open pull request has a task, and says
  a draft waiting on the owner is not stalled work waiting on an agent. Each rule any of the three
  files gains names, in its own text, what can refuse it, and is stated in exactly one of them —
  the other mentions are pointers, carrying no restatement of the rule beside them.
pr: https://github.com/teonimesic/plasmosome/pull/59
evidence: squash commit 17221cd on main; pr-review, tasks and heartbeat now state that every pull request has a task, rounds are counted across the pull request's commits, and the merged head is always one a review read
---

## Why

Four rulings from the owner, three of which settle a contradiction that was already costing
something.

`pr-review` told an agent two incompatible things about a small pull request: merge only a commit
that reads `Review completed`, and spend one round without re-triggering. Addressing a finding
makes a new commit, so an agent that fixed something on a small change either merged bytes no
review had read or spent a round the table forbade.

`@coderabbitai review` is declined on a head the reviewer has already seen, and the decline is
invisible: it hides in a collapsed block, it edits the walkthrough so `updated_at` moves, and the
status still reads `Review completed` from the round before. A 233-line pull request owing two
rounds came within a step of merging on one, and the round that was eventually obtained found a
real defect.

And the task requirement said one thing while every merged pull request did another. The table
exempted changes under ~20 lines; practice exempted the whole skills and process category at any
size.

## Plan

Written and executed together, as one documentation change across the three skills that carry the
rules. No product code.

## Notes

**The open question these notes used to carry has been answered by writing the spec.** This task
named no spec, and the honest description of it was that it broke the rule the same change writes.
The reason was the second row of the requirement table — behavior an existing intent wants and no
spec yet describes — with `docs/intents/008-an-ai-native-way-of-building-plasmosome.md` as that
intent, in its own words: "strict guardrails and well defined processes for its own development".

`docs/specs/012-how-work-enters-the-tree.md` is that spec, accepted under intent 008, and it landed
in a pull request of its own before this one. So this task names it, the rule closes with no
exception, and the two-PR ordering held: the contract was reviewed apart from the rules it governs.

**The skill text is the work under that spec, not a second copy of it.** Two of spec 012's
acceptance items land here — the closed list of shapes carrying no task, and the distinction
between them — and both are written as pointers at the spec rather than as a rule this file could
later widen on its own authority.

**Where the rule needed saying in more than one place, only one file says it.** The permission to
file a task in the same pull request is stated in `.agents/skills/tasks` and pointed at from
`pr-review` step 6 and `heartbeat` step 1, which name the condition and the remedy's location
without restating it. A pointer sitting next to a restatement is not a pointer, which is the shape
spec 012's one-statement clause refuses.

**Task 028 is mooted by this change and is untouched.** It asks what the spec threshold should
count; this change deletes the threshold. It needs closing on a later branch, since a `done` flip
cannot ride the pull request it describes.

**The round counter this change adds was wrong in its first form, and the correction is the part
worth reading.** It counted `Review completed` statuses on one commit, and the merge gate then
asked the merged head to carry the number the rounds table requires. But rounds accumulate across
heads while statuses accumulate per head: addressing a finding moves the head, and the new head
carries only the round that read it. Run against PR #58 — 239 lines, two rounds owed and had — the
head-only count returns 1 and refuses it, as it also refuses #57 and #55. Nothing honest satisfies
that gate; the only escape is re-triggering until the number comes up, which spends a budget that
is repo-wide and roughly ten an hour to re-prove a round already paid for. The gate is now two
conditions that are each true of the thing they name: `rounds <pr>` over the pull request's commits
for how much review happened, and `completed_on <head>` at least 1 for whether any of it read the
commit being merged.

**Ruling four now binds, because #43 merged.** Intents carry `status: draft | approved`, so the
`pr-review` step 2 paragraph describing a chain that reaches an unapproved goal is no longer
written against a state that does not exist. It refuses nothing at this moment — all thirteen
intents on `main` read `approved` — and that is written into the skill rather than left for a
reader to discover, because a rule that has never yet refused anything is one nobody can tell is
working. The textual conflict with #43 in that step landed the way it was expected to: #43 went
first and this branch rebased through it, keeping #43's paragraph unchanged and adding the layer
below it.
