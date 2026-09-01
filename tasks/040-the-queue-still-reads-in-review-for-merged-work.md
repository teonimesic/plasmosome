---
id: 040
title: The queue still reads in_review for merged work, and cites an amnesty that is closed
status: in_review
priority: 3
specs: [012]
intents: [008]
refs:
  [
    .agents/skills/tasks/SKILL.md,
    docs/specs/012-how-work-enters-the-tree.md,
    docs/specs/README.md,
    docs/specs/001-control-protocol.md,
    tasks/029-conformance-tasks-do-not-name-their-intent.md,
  ]
done_when: >-
  every task whose `pr:` names a pull request GitHub reports MERGED carries `status: done` and a
  non-empty `evidence:` naming a commit that is that pull request's squash commit and an ancestor
  of `main` — checked per task with `gh pr view <n> --json state,mergeCommit` and
  `git merge-base --is-ancestor`, never against the file's own claim — with one named exception:
  task 003 stays `in_review`, carrying a dated note saying why, until the audit workflow's first
  scheduled run completes green, because its done_when requires that run and merging the workflow
  does not produce it; every task whose `specs:` names spec 001 carries an `intents:` line equal
  to spec 001's own, and task 018 carries the union of what specs 001 and 008 carry; tasks 009,
  010 and 011 are untouched, because task 029 already owns their backfill; and every passage
  `grep -rni amnesty docs .agents` prints describes the spec-side amnesty as closed and empty,
  with `grep -rn 'intents: \[\]' docs .agents` matching no sentence that asserts spec 001 carries
  it — its hits are the two template skeletons, the past-tense clauses in spec 012 recording that
  the amnesty closed, the clause about what a task carries, and the skills' general rule, none of
  which asserts it of spec 001 today.
pr: https://github.com/teonimesic/plasmosome/pull/69
evidence:
---

## Why

Ten merged pull requests still read `status: in_review` with an empty `evidence:`, so the greps
the queue is worked from return review work that finished days ago. Six tasks under spec 001 still
carry the blank `intents:` the closed amnesty used to excuse, and `docs/specs/012` and
`docs/specs/README.md` still describe that amnesty as open — spec 001 has named its intents since
the mock refusal landed, and the skills already say so. The records trail the tree; this task
walks them forward.

## Plan

The deliverable, in one sentence: the task queue and the two spec documents read true against the
tree at the head this branch forked from, per the edit table in the pull request.

Out of scope: the `intents:` of tasks 009, 010, 011 and 012, which task 029 owns; every task whose
`specs:` is `[]`, which waits on the legacy mapping list in `.agents/skills/tasks`; the format of
existing `pr:` fields; and all code.

Read only the files in `refs:` and the task files the edit table names. No new tests: the change
is records, and the gate in the root `AGENTS.md` proves the tree still holds.

Done when the `done_when` above reads true and the gate in the root `AGENTS.md` is green, each
exit code read bare rather than through a pipe.

STOP when done — do not start the next piece of work.

## Notes
