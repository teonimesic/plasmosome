---
name: tasks
description: How work is written down, found, and closed — intents, specs, and the task queue. Use when picking up work, filing new work, closing finished work, or at the start of any working session.
---

# Finding, filing and closing work

Work is written down in three places. **`docs/intents/`** records why someone wanted something,
in their own words, before anyone designed it. **`docs/specs/`** records what a thing must do —
behavior, contract, acceptance — agreed before it is built. **`tasks/`** holds the units that
actually move: one file each, with a status, a priority, and a plan for whoever will do the work.

Tasks sit at the top level of the repository, not under `docs/`. `docs/` is finished reference
you read to understand the system. `tasks/` is the live queue you work from and change every day.
Keeping them apart stops one from being mistaken for the other.

Most work needs only a task. Some needs a spec first. Very little needs an intent — the table
below decides. To pick up work right now: run `grep -l "^status: planned" tasks/*.md`, take the
file with the lowest `priority:` number, and follow its `## Plan`.

## When each one is required

| Change | Intent | Spec | Task |
| --- | --- | --- | --- |
| Trivial fix finished in the same session (under ~20 lines, no contract touched) | no | no | no — the PR is the record |
| Work that outlives the session, or is handed to another agent | no | no | yes |
| 100 lines or more, or it touches enforcement or revocation semantics, or a public contract | no | yes — accepted before building starts | yes |
| Owner-originated capability work, or a change of direction | yes | yes | yes |

## Who writes what

- **Intent** — the owner. An agent may transcribe it word for word. Never summarize it.
- **Spec** — the planner, using the strongest model available. It becomes `status: accepted`
  only after the owner has read it.
- **Task, and its `## Plan`** — the planner.
- **Execution** — the next model down, in its own worktree, reading the task and the files the
  task names, and nothing else.

**The `## Plan` belongs in the task, never in the spec.** A spec says what must be true and
outlives many tasks. A plan is tied to one branch and is stale the day it merges.

## Numbering

`NNN-slug.md`, three digits, in each of the three directories. Each directory numbers from 001
on its own, so a spec 002 and a task 002 are unrelated. `docs/intents/` does not exist yet; the
first intent creates it.

## Templates

Fill these in as they stand. Anything marked optional may be left blank — a blank section is
better than filler, because filler reads as something that was considered.

### Intent — `docs/intents/NNN-slug.md`

```
---
id: 004
title: short name for the thing wanted
date: 2026-08-30
originator: who asked
---

The why, in the originator's words, unedited: what they want, and what made
them want it. No design and no solution — those come later, in a spec.

## Outcome
Optional, added later: what was built, or why nothing was.
```

An intent has no status field. It records a moment and never changes state.

### Spec — `docs/specs/NNN-slug.md`

```
---
id: 002
title: what the thing is
status: draft
intent: docs/intents/004-slug.md
---

## Behavior
What it does, seen from outside.

## Contract
Names, types, states, errors. What a caller may rely on.

## Acceptance
The list a reviewer checks the diff against. One checkable line each.
```

`status:` is `draft`, `accepted` or `superseded`. Leave out `intent:` when there is no intent
file — a spec driven by a defect, or written before intents existed. New specs use all three
headers; `docs/specs/001-control-protocol.md` predates this shape and keeps its own.

### Task — `tasks/NNN-slug.md`

```
---
id: 003
title: short name for the unit of work
status: todo
priority: 2
spec: docs/specs/002-slug.md
refs: crates/foo/AGENTS.md
done_when: one sentence a stranger can check, or a short list of them
pr:
evidence:
---

## Why
One to three lines, or a pointer to the intent.

## Plan
Written by the planner. Blank while the task is `todo`.

## Notes
Dated appendices. Blank until someone has something to add.
```

`spec:` and `refs:` are optional; `refs:` takes one path or a list. `pr:` and `evidence:` stay
empty until the work reaches review and then merges.

## Priority

- **1** — something else is blocked until this is done.
- **2** — a known defect, or the next capability.
- **3** — nothing is waiting on it.

## Lifecycle

| Status | Means | Entering it requires |
| --- | --- | --- |
| `todo` | filed | `done_when:` filled in |
| `planned` | ready to hand to an executor | `## Plan` written; if the change crosses the spec threshold above, `spec:` names an accepted spec |
| `in_progress` | claimed | branch `task-NNN-slug`, in the executor's own worktree |
| `in_review` | PR open | `pr:` set |
| `done` | squash-merged | `evidence:` not empty |

**A status records a decision someone made, not what is true right now.** `status: in_review`
means someone wrote that line; it does not mean a PR is open. `gh pr view` is the truth about a
PR. Check before believing the file.

## Writing a `## Plan` for a stranger

The executor reads the task and nothing else. It was not in the conversation the plan came out
of and has no memory of it. Every plan carries:

- The deliverable, in one sentence.
- What is explicitly out of scope.
- The exact files to read, and an instruction not to explore beyond them.
- A test table: each test's name, and what it proves.
- The definition of done, including the gate in the root `AGENTS.md`.
- "STOP when done — do not start the next piece of work."

If you catch yourself writing "as discussed" or "the usual approach", stop and write the thing
out instead.

## Finding things

```
grep -l "^status: planned" tasks/*.md
grep -h "^title:" $(grep -l "^status: todo" tasks/*.md)
grep -l "^priority: 1" tasks/*.md
grep -rl "docs/specs/002" tasks/ docs/specs/
```

## Checking whether a task is really done

`gh pr merge --squash` puts a new commit on `main` and does not merge the branch. **Afterwards
the branch tip is not an ancestor of `main` — the squash commit is.** So any check shaped like
"is this branch merged into main" answers no for work that shipped weeks ago. Never verify a
task that way.

Ask GitHub instead:

```
gh pr view <number> --json state,mergeCommit
```

`state: MERGED` with a merge commit is the proof. Put that commit hash, or the PR URL, in
`evidence:`.

## Queue changes travel by pull request

`main` is protected, so changing a task file is a commit like any other.

- Filing a task, and every status flip up to `in_review`, rides the work branch itself.
- `done` flips are batched into the next work PR, or into a `chore(tasks): close NNN` PR of
  their own.

## Heartbeat

Run this whenever you pick the work back up.

1. **Verify.** Run `gh pr list` and compare it against every task marked `in_review` or
   `in_progress`. Where the file and GitHub disagree, correct the file and record in `## Notes`
   what established the truth. A stale queue is worse than no queue, because people believe it.
2. **Close.** Batch the `done` flips into one `chore(tasks)` PR.
3. **Pick.** The highest-priority `planned` task — the lowest `priority:` number — not the
   newest one.
4. **File.** Anything you discovered that outlives this session becomes a task. Not a note in a
   session to-do list that disappears when the session ends.

## Tooling

There is none, deliberately. The greps above are the whole interface, and plain files can be
read and fixed by anyone without running anything.

Write `tools/tasks.py` — `list`, `show`, `next`, `check` — when one of these happens, and not
before: more than 15 open tasks, or the first time a file is malformed, a task is marked done
that is not, or a status drifts from reality without anyone noticing.

**If the tool and the files ever disagree, the files win.**
