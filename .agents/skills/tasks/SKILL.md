---
name: tasks
description: How work is written down, found, and closed — the six layers, the task queue, and the heartbeat. Use when picking up work, filing new work, closing finished work, or at the start of any working session.
---

# Finding, filing and closing work

Work in this repository is written down in files, not left in a chat window. There are six
layers, from the sentence that would survive a total rewrite down to the unit of work you do this
afternoon: **vision**, **architecture**, **decisions**, **intents**, **specs**, **tasks**. The
bottom four have folders — `docs/decisions/`, `docs/intents/`, `docs/specs/` and `tasks/`. The
top two already live in `README.md` and in each crate's `AGENTS.md`, and stay there.

**There is no task without a spec, and no spec without an intent.** A trivial change earns no
task at all, so the chain never forces a spec for a typo. Everything above that line earns a task,
that task names the spec it serves, and that spec names the intent it came from. Mapping to a spec
and an intent that already exist is the normal case; writing new ones is the exception.

To pick work up: run the **heartbeat** (`.agents/skills/heartbeat`) top to bottom — it ends by
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
| Trivial fix finished in the same session — under ~20 lines, no contract touched | no | no | no — the PR is the record |
| Code an existing spec already governs, including a bug that spec did not get right | the one that spec names | that spec | yes |
| Behavior an existing intent wants and no spec yet describes | that intent | a new one | yes |
| Anything no existing spec and no existing intent reaches | the owner writes one, or the work does not happen | a new one | yes |

There is no line count in that table any more. Size stopped being the question the moment every
task needed a spec: a 400-line change to code spec 003 already governs needs no new document, and
a 30-line change to something nothing describes needs the same two links as a large one. The old
threshold said 100 lines and no merged change had ever obeyed it, which made it useless as a
review standard.

## The chain, and the two gates

The links are the `specs:` and `intents:` fields. They are how anyone can tell, without asking,
whether a piece of work is wanted.

**Mapping is the normal case; writing is the exception.** A bug that exists because a spec did not
get something right maps to the spec it violates: the spec is already there and so is the intent
behind it, so that fix needs neither a new document nor an owner decision — it needs the two id
fields filled in. Most work should be mappable this way. When most of it is not, what has drifted
is the queue, not the paperwork.

Both gates are on what may be **started**, not on what has been written down:

- **A task may not enter `in_progress` until it has been planned**, and it cannot reach `planned`
  until `specs:` names an accepted spec.
- **A spec may not be planned until `intents:` names an approved intent.** An intent on `main` is
  approved: only the owner writes one, `main` is protected, so a merged intent is one the owner
  asked for. Nothing else goes in `docs/intents/` — a proposal there would be indistinguishable
  from an approval.

**The owner approves intents. Nobody approves specs.** The planner writes a spec and accepts it.
The gate sits where the question is "is this wanted", which only the owner can answer, and not
where the question is "is this right", which a reviewer answers on the pull request.

### What the gates refuse

**A task that maps to no spec and to no plausible intent is evidence the work is not wanted.** It
is not a prompt to write an intent that would make it wanted. An intent written to justify work
already filed turns the owner's gate into a rubber stamp, and that gate is the only thing between
the queue and everything anybody has ever noticed.

There are two honest endings for such a task. Put the question to the owner in your own words and
let them say whether an intent covers it. Or drop it, and write down why. Filing it and letting it
wait is neither.

### A review finding that maps to nothing

A finding is fixed in the pull request that raised it. That rule is unchanged and it is still the
default.

What changes is the fallback. **Filing a finding as a task is only available when the finding maps
to a spec.** A finding against behavior some spec requires is an ordinary task: it names that spec
and joins the queue. A finding against something no spec covers has exactly two endings — fixed
here, or dropped with the reasoning written in the thread. There is no third one, and "file it and
move on" was the third one.

This half is what makes the chain worth having. Without it the rule adds bookkeeping to a queue
that keeps growing at the same rate, because every change produces a review, every review produces
findings, and every finding used to produce a task.

**The failure this prevents, concretely.** The queue stopped being fed by the plan and started
being fed by the review process. On the day this was written `main` held twenty tasks, eight
naming no spec and sixteen naming no intent, and seven more had been filed in a single day, none
of them tracing to either. That generates work in proportion to how much reviewing happens rather
than to what the product needs, and it compounds. You can tell whether this rule is working by
whether the count of tasks naming no spec falls; if it climbs, it is not.

## Who writes what

- **Intent** — the owner. An agent may transcribe it word for word. Never summarize it, and
  never write one on the owner's behalf because a task needed something to point at.
- **Spec** — the planner, using the strongest model available. It becomes `status: accepted` when
  its pull request merges, and the planner is who accepts it. The owner's approval is spent on the
  intent above it.
- **Task, and its `## Plan`** — the planner.
- **Execution** — the next model down, in its own worktree, reading the task and the files the
  task names, and nothing else.

**The `## Plan` belongs in the task, never in the spec.** A spec says what must be true and
outlives many tasks. A plan is tied to one branch and is stale the day it merges.

## Numbering, and the fields that link the layers

Files are `NNN-slug.md`, three digits. Each folder numbers from 001 on its own, so spec 002 and
task 002 are unrelated.

**Take the number from the remote, not from `main`.** Another agent may already have filed one on
an unmerged branch, and `main` cannot see it. Two branches carrying the same number is a conflict
nobody notices until merge.

```shell
git fetch origin
for b in $(git ls-remote --heads origin | awk '{print $2}' | sed 's|refs/heads/||'); do
  git ls-tree -r --name-only "origin/$b" tasks/ 2>/dev/null
done | sort -u > /tmp/tasknums

sed -E 's|tasks/([0-9]{3}).*|\1|' /tmp/tasknums | sort -n | tail -1

sed -E 's|tasks/([0-9]{3}).*|\1|' /tmp/tasknums | sort | uniq -d |
  while read n; do grep "tasks/$n" /tmp/tasknums; done
```

The first command gives the number to take one past. The second prints nothing when the numbering
is sound, and prints both files when it is not. It compares distinct paths, not branches — the
same file on three branches is one file, while one number carrying two different slugs is the
collision. Do the same for `docs/specs/` and `docs/intents/`, which number separately.

Links point upward, and they are always **one-line flow lists of those three-digit ids**:

- a spec carries `intents: [003]`
- a task carries `specs: [001, 004]` and `intents: [003]`

**Both fields are always present.** Write `[]` while the link has not been made yet — a `todo`
task nobody has planned is the one place that legitimately stays empty, and it is what the two
gates above refuse to let past. An absent field would force every search to be written twice. Keeping them on one line, anchored at the start of
the line, is what keeps the two id namespaces apart and stops a search matching body prose.

## Templates

Copy the skeleton, do not retype it:

```shell
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
| `planned` | ready to hand to an executor | `## Plan` written; `specs:` names an accepted spec, and `intents:` names the intent that spec carries |
| `in_progress` | claimed | it was `planned` first; branch `task-NNN-slug`, in the executor's own worktree |
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

```shell
grep -l '^status: todo' tasks/*.md
grep -l '^status: planned' tasks/*.md
grep -l '^priority: 1' tasks/*.md
grep -l '^specs:.*\b001\b' tasks/*.md
grep -l '^intents:.*\b003\b' docs/specs/*.md
grep -l '^intents:.*\b003\b' tasks/*.md
grep -l '^specs: \[\]' tasks/*.md
grep -l '^intents: \[\]' docs/specs/*.md
grep -h '^title:' /dev/null $(grep -l '^status: todo' tasks/*.md)
```

The last two are the gates read backwards: the first prints every task that may not be planned
yet, the second every spec that may not be planned yet. Both should be short lists and both should
be getting shorter.

## Checking whether a task is really done

`gh pr merge --squash` puts a new commit on `main` and does not merge the branch. **Afterwards
the branch tip is not an ancestor of `main` — the squash commit is.** Any check shaped like "is
this branch merged into main" answers no for work that shipped weeks ago. Never verify a task
that way.

Ask GitHub instead:

```shell
gh pr view <number> --json state,mergeCommit
```

`state: MERGED` with a merge commit is the proof. Put that commit hash, or the PR URL, in
`evidence:`.

## Which pull request does each file land in

`main` is protected, so every file here — intent, spec, task — reaches it the same way code does:
on a branch, through a PR. Nothing is written straight to `main`. That has one consequence people
trip over, so it is worth stating plainly.

**A spec lands in its own PR, before the work branch exists.** No task may be claimed until the
spec it names is `accepted`, and a spec is `accepted` once its PR merges. So work that needs a new
spec is two PRs, in order:

1. `docs(spec): NNN <title>` — the spec at `status: draft`, its `intents:` naming an intent that
   is already on `main`. It merges after review; that merge is what makes it `accepted`. Flip the
   status to `accepted` in the last commit before merging, so `main` never holds a spec whose
   status lies.
2. The work branch, `task-NNN-slug` — the code, plus the task's own status flips.

An intent reaches `main` the same way and earlier still, in a PR of its own carrying the owner's
words as the owner wrote them.

Work whose spec already exists skips step 1 and is one PR, which is what most work should look
like. Trivial work skips the task as well.

- Filing a task, and every status flip up to `in_review`, rides the work branch itself.
- `in_review` needs `pr:`, which does not exist until the PR is open. Set it in a second commit
  and push — that costs a CI run, so open the PR as a draft and set it before marking it ready.
- `done` flips ride the next piece of work, or a `chore(tasks): close NNN` PR of their own. They
  cannot ride the PR they describe: it has already merged.

## The heartbeat

Every working session starts with it: reconcile the queue against reality, then pick. It is its
own skill — see `.agents/skills/heartbeat`.

## Tooling

There is none, deliberately. The greps above are the whole interface, and plain files can be read
and fixed by anyone without running anything.

Write `tools/tasks.py` — `list`, `show`, `next`, `check` — when one of these happens, and not
before: more than 15 open tasks, or the first time a file is malformed, a task is marked done
that is not, or a status drifts from reality without anyone noticing.

**If the tool and the files ever disagree, the files win.**
