---
name: tasks
description: How work is written down, found, and closed — the six layers, the task queue, and the heartbeat. Use when picking up work, filing new work, closing finished work, or at the start of any working session.
---

# Finding, filing and closing work

Work in this repository is written down in files, not left in a chat window. There are six
layers, from the sentence that would survive a total rewrite down to the unit of work you do this
afternoon: **vision**, **architecture**, **decisions**, **intents**, **specs**, **tasks**. Four
have folders — `docs/decisions/`, `docs/intents/`, `docs/specs/` and `tasks/`. The other two
already live in `README.md` and in each crate's `AGENTS.md`, and stay there.

Most changes need only a task. A change of 100 lines or more, or one that touches enforcement or
revocation semantics or a public contract, needs an accepted spec first. Only owner-originated
capability work needs an intent. The two tables below decide; when in doubt, file a task.

To pick work up: run the **heartbeat** at the bottom of this file, top to bottom — it ends by
handing you the next task. To file something: copy the matching skeleton out of `docs/templates/`
and fill it in.

## The six layers

| Layer | Where it lives | It belongs here if |
| --- | --- | --- |
| Vision | `README.md` — `## Why`, `## Properties` | the sentence would survive rebuilding the system a completely different way |
| Architecture | `README.md` — `## Architecture`; each crate's `AGENTS.md` | it is about how two or more pieces fit together, not about any one piece |
| Decision | `docs/decisions/NNN-title.md` | a choice was made between real alternatives someone would argue for again |
| Intent | `docs/intents/NNN-slug.md` | it says what and why, with no how |
| Spec | `docs/specs/NNN-slug.md` | it says how, it is testable, and it says nothing about who or when |
| Task | `tasks/NNN-slug.md` | you could delete it after merge and lose nothing permanent |

Anything about one piece alone goes in that crate's own docs, not in an architecture note. A
decision is never edited: when it stops holding, write a new one and mark the old `superseded`.

`tasks/` sits at the top level of the repository rather than under `docs/` on purpose. `docs/` is
finished reference you read to understand the system. `tasks/` is the live queue you work from
and change every day. A task file read as documentation misleads in both directions.

`docs/intents/` does not exist yet — the first intent creates it.

### When to promote a layer into its own file

- Write **`docs/VISION.md`** the first time an intent is turned down and there is no written
  sentence to cite for turning it down.
- Write **`docs/architecture.md`** at the first piece of reasoning that spans three or more
  crates and fits in none of their own docs. When it exists it names crates and links to them; it
  never restates a crate's contract.

Until then, do not create either file. A second copy of the vision would contradict the first.

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

## Numbering, and the fields that link the layers

Files are `NNN-slug.md`, three digits. Each folder numbers from 001 on its own, so spec 002 and
task 002 are unrelated.

Links point upward, and they are always **one-line flow lists of those three-digit ids**:

- a spec carries `intents: [003]`
- a task carries `specs: [001, 004]` and `intents: [003]`

**Both fields are always present.** Write `[]` when there is nothing to link. An absent field
would force every search to be written twice. Keeping them on one line, anchored at the start of
the line, is what keeps the two id namespaces apart and stops a search matching body prose.

## Templates

Copy the skeleton, do not retype it:

```
cp docs/templates/task.md tasks/004-my-slug.md
```

`docs/templates/` holds `intent.md`, `spec.md`, `task.md` and `decision.md`. Fields marked
optional, and sections you have nothing to put in, are left blank. A blank section is better than
filler, because filler reads as something that was considered.

A few field values worth stating outright:

- Spec `status:` is `draft`, `accepted` or `superseded`.
- Task `refs:` is the files the executor must read. `pr:` and `evidence:` stay empty until the
  work reaches review and then merges.
- New specs use all three headers. `docs/specs/001-control-protocol.md` predates this shape and
  keeps its own.

## Priority

- **1** — something else is blocked until this is done.
- **2** — a known defect, or the next capability.
- **3** — nothing is waiting on it.

## Lifecycle

| Status | Means | Entering it requires |
| --- | --- | --- |
| `todo` | filed | `done_when:` filled in |
| `planned` | ready to hand to an executor | `## Plan` written; if the change crosses the spec threshold above, `specs:` names an accepted spec |
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
grep -l '^status: todo' tasks/*.md
grep -l '^status: planned' tasks/*.md
grep -l '^priority: 1' tasks/*.md
grep -l '^specs:.*\b001\b' tasks/*.md
grep -l '^intents:.*\b003\b' docs/specs/*.md
grep -l '^intents:.*\b003\b' tasks/*.md
grep -h '^title:' $(grep -l '^status: todo' tasks/*.md)
```

## Checking whether a task is really done

`gh pr merge --squash` puts a new commit on `main` and does not merge the branch. **Afterwards
the branch tip is not an ancestor of `main` — the squash commit is.** Any check shaped like "is
this branch merged into main" answers no for work that shipped weeks ago. Never verify a task
that way.

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

## The heartbeat

Run this at the start of every working session, in this order. It replaces the session to-do
list entirely: nothing that matters is remembered, everything that matters is a file.

**1. Pending PRs.**

```
gh pr list --state open
```

For each one, answer three questions: is CI green, are the review rounds for its diff size done,
and are all conversations resolved? `main` requires conversation resolution, so a single
unresolved thread is what is blocking the merge — not CI, and not a missing approval.

**2. Pending reviews.**

```
gh pr view <number> --json reviewThreads
```

The threads still open are the queue. Work them before opening anything new.

**3. Pending tasks.** Reconcile `tasks/` against reality: the open PRs from step 1 and the
`task-*` branches on the remote.

```
git ls-remote --heads origin 'task-*'
```

Release a claim whose branch and PR are both gone — put it back to `planned`. Mark as `done`
anything that already landed, with `evidence:`. Record in `## Notes` what established the truth.
A stale queue is worse than no queue, because people believe it.

**4. Pending specs.** Work that was described and then dropped hides in two places: a spec still
in draft that no task implements, and an intent with no spec at all.

```
grep -l '^status: draft' docs/specs/*.md
```

For each draft spec id, an empty result here means nothing is implementing it:

```
grep -l '^specs:.*\b002\b' tasks/*.md
```

And for each intent id, an empty result here means no spec was ever written:

```
grep -l '^intents:.*\b003\b' docs/specs/*.md
```

Either way, decide out loud: plan it, or say why not.

**5. Pick.** The highest-priority `todo` — the lowest `priority:` number — not the newest one.

**6. File.** Anything you learned this session that must outlive it becomes a task file before
the session ends.

## Tooling

There is none, deliberately. The greps above are the whole interface, and plain files can be read
and fixed by anyone without running anything.

Write `tools/tasks.py` — `list`, `show`, `next`, `check` — when one of these happens, and not
before: more than 15 open tasks, or the first time a file is malformed, a task is marked done
that is not, or a status drifts from reality without anyone noticing.

**If the tool and the files ever disagree, the files win.**
