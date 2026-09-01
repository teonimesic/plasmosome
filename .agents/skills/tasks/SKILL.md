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
| Anything no existing spec and no existing intent reaches | a new one — anyone drafts it, the owner approves it or the work does not happen | a new one | yes |

**The line count that decided whether work needed a spec is gone; the one in the first row is not
the same rule.** It bounds the trivial exemption from above and does nothing else — it says when a
change is too small to be worth writing down, never when one is big enough to need a spec. Read the
other way it would be a bypass, and an unbounded "trivial" is a worse one: without a ceiling, a
500-line change touching no contract and finished in an afternoon would qualify. Size stopped being
the question everywhere above that row: a 400-line change to code spec 003 already governs needs no
new document, and a 30-line change to something nothing describes needs the same two links as a
large one. The old
threshold also could not be cited in review: four merged pull requests crossed it and none had a
spec — two of them (126 and 218 lines) had no task either, and two (103 and 167) had a task and no
spec. Every change that did name a spec was one the plan had already routed through one, so the
number never decided anything.

## The chain, and the two gates

The links are the `specs:` and `intents:` fields. They are how anyone can tell, without asking,
whether a piece of work is wanted.

**Mapping is the normal case; writing is the exception.** A bug that exists because a spec did not
get something right maps to the spec it violates: the spec is already there and so is the intent
behind it, so that fix needs neither a new document nor an owner decision — it needs the two id
fields filled in. Most work should be mappable this way. When most of it is not, what has drifted
is the queue, not the paperwork.

Both gates are on what may be **started**, not on what may be written down:

- **A task may not enter `in_progress` until it has been planned**, and it cannot reach `planned`
  until `specs:` names an accepted spec.
- **A spec may not become `accepted` until `intents:` names an intent whose `status:` is
  `approved`.** Generating that spec earlier is allowed and is meant to happen: a `draft` spec may
  name a `draft` intent, so a human reading does not idle the queue. What waits on approval is
  commitment, not thought.

The two layers work the same way because their statuses mean the same thing. `draft` is a proposal
on the record; `approved` and `accepted` are what may be built on. Who may move an intent to
`approved` — the owner, relayed or direct, never an agent on its own judgement — is stated in
`AGENTS.md` and in `docs/intents/README.md`, and not restated here.

**Why the owner's gate sits at approval and not somewhere cheaper.** An approved intent may spin up
a great many specs, and each of those spawns tasks. Approval is the point where a single decision
multiplies into a lot of work, so it is the point a person has to hold. Drafting costs one document;
approving commits a queue. Apply that reasoning to a case not written down here by asking whether
the step turns one yes into work nobody has counted.

**What working ahead costs.** A draft spec whose intent is later refused does not survive. That is
the price of not idling, not a failure by whoever wrote it, and it is why a speculative spec stays
`draft`, where it is cheap to throw away.

**The second gate binds a spec being accepted, not one already accepted.** A spec that is already
`accepted` stays usable, and a task naming it may be planned, whether or not anything above it is
filled in. Otherwise this rule would strand finished work behind a document only the owner can
approve. Backfilling an empty `intents:` is worth doing: anyone may draft the intent that would
close the gap, and only the owner can approve it. Nothing waits on either. What the gate stops is a
*new* spec being committed to on the strength of an intent the owner has not read, which is the
direction the drift actually runs.

A task's `intents:` is copied from the spec it names, `[]` included. It is there so a search over
tasks and a search over specs return the same answer, not as a second gate to clear.

**The owner approves intents. Nobody approves specs.** The planner writes a spec and accepts it,
once the intent above it is approved. The gate sits where the question is "is this wanted", which
only the owner can answer, and not where the question is "is this right", which a reviewer answers
on the pull request.

### What predates the rule

`main` already holds tasks and specs that name nothing above them. They were filed correctly under
the rule in force at the time, and **this one does not reach back**: they stay valid and they stay
merged. **Editing such a file is not gated on backfilling it** — a correction, a rewritten `## Why`,
a `pr:` or `evidence:` field, a flip to `in_review` or `done`, and any change elsewhere that happens
to touch an unmapped task file. None of that waits for a mapping.

**Two status values are outside that exception, because they are not bookkeeping — they are the work
starting.** A flip to `planned` or to `in_progress` clears the gates above or it does not happen,
and being a legacy file buys no discount on them. Naming the two is deliberate: stated in general
terms this keeps coming out ambiguous between the file and the work, and "a status flip is fine"
is exactly the sentence that lets `status: planned` through the gate this section exists to hold.
The greps under "Finding things" are the waiting list that gate creates, not a list of faults.

**The amnesty is a closed set, and it is one file.** `docs/specs/001-control-protocol.md` is the
only spec that is `accepted` and names no intent, so any *other* accepted spec with an empty
`intents:` is a spec that skipped the gate rather than one that predates it. Naming the set is what
makes that difference visible: an unbounded "it predates the rule" is a permanent excuse, because
nothing distinguishes an old file from a new one claiming to be old. The heartbeat's cross-layer
loop hardcodes that one name for the same reason.

Two shapes that look like breakage and are not. An **accepted spec with an empty `intents:`** keeps
its place if it is that one file; only a spec being written has to name one. And a **task whose
chain closes one layer up** is mapped even while its own `intents:` is blank — if the task names a
spec and that spec names an intent, the link is sound and the blank field is a missing copy, not a
missing link. The copy exists
so a search over tasks and a search over specs return the same answer; filling it in is bookkeeping,
and the greps print those tasks until someone does.

### What the gates refuse

**A task that maps to no spec and to no plausible intent is evidence the work is not wanted.** It
is not a prompt to approve an intent that would make it wanted. That gate is the only thing between
the queue and everything anybody has ever noticed.

There are two honest endings for a task you are filing **now**. Put the question to the owner — a
`draft` intent in `docs/intents/` is how to do that on the record, where the next agent finds it
instead of working it out again — and let them approve it or refuse it. Or drop it, and write down
why. Filing it and letting it wait is neither, and it is how a queue fills with work nobody chose.

The legacy files above are not this. They are already filed, and mapping or dropping them is the
backfill rather than a fresh filing — which is why they may sit on the waiting list while a new one
may not be put there.

**Drafting is not approving, and that is what makes drafting safe.** A draft cannot rubber-stamp the
work under it, because nothing may be committed to until the owner approves it. So write the
proposal down rather than leaving it somewhere it will be lost — but write it as the question it is,
and never as an answer that unblocks what you already filed.

### A review finding that maps to nothing

A finding is fixed in the pull request that raised it. That rule is unchanged and it is still the
default.

What changes is the fallback. **Filing a finding as a task is only available when the finding maps
to a spec.** A finding against behavior some spec requires is an ordinary task: it names that spec
and joins the queue. A finding against something no spec covers has exactly two endings — fixed
here, or dropped with the reasoning written in the thread. There is no third one, and "file it and
move on" was the third one.

**Drafting an intent is not a third ending.** Where the finding is not a defect at all but a goal
nobody has written down, the draft *is* the "dropped with the reasoning written down" ending, put
where the owner will see it rather than only in a thread. It starts nothing: the finding is still
not a task, and does not become one until the owner approves and a spec names it.

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

- **Intent** — drafted by anyone; approved by the owner, and recorded by whoever is carrying that
  approval, usually an agent it was relayed to. Who wrote it does not matter. An agent's own draft
  stays `status: draft` until the owner's approval actually arrives — relayed or direct, but
  arriving, never assumed. A draft written only because a filed task needed something to point at
  is a proposal the owner should refuse, and writing it does not make it less refusable.
- **Spec** — the planner, using the strongest model available. It becomes `status: accepted` when
  its pull request merges, and the planner is who accepts it. The owner's approval is spent on the
  intent above it.
- **Task, and its `## Plan`** — the planner.
- **Execution** — the next model down, in its own worktree, reading the task and the files the
  task names, and nothing else.

**A decision is not a link in the chain.** It records why a choice was made, and a task may cite
one in `refs:`, but it never stands in for the spec a task has to name. Where a decision settles
something a task must build against, the buildable half of it belongs in a spec.

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
task nobody has planned is the one place that legitimately stays empty, and it is what the gates
above refuse to let past. That is also why the templates ship `[]` rather than a placeholder:
a copied-and-unfilled `[NNN]` would read as a link and pass every grep below. An absent field
would force every search to be written twice. Keeping them on one line, anchored at the start of
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

- Intent `status:` is `draft` or `approved` — there is no `superseded`, so a withdrawn approval is
  unrepresentable and nothing has needed it. Approval originates with the owner; an agent records
  one it is carrying and never invents one. `outcome:` is blank while the intent is open and
  non-blank once settled, which is what tells a refused draft from a forgotten one.
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
| `planned` | ready to hand to an executor | `## Plan` written; `specs:` names an accepted spec, and `intents:` carries whatever that spec carries |
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

```shell
grep -l '^status: todo' tasks/*.md
grep -l '^status: planned' tasks/*.md
grep -l '^priority: 1' tasks/*.md
grep -l '^specs:.*\b001\b' tasks/*.md
grep -l '^intents:.*\b003\b' docs/specs/*.md
grep -l '^intents:.*\b003\b' tasks/*.md
grep -l '^specs: \[\]' tasks/*.md
grep -l '^intents: \[\]' tasks/*.md
grep -l '^intents: \[\]' docs/specs/*.md
grep -l '^status: draft$' docs/intents/*.md
grep -h '^title:' /dev/null $(grep -l '^status: todo' tasks/*.md)
```

Three of those are the gates read backwards: tasks that may not be planned yet, tasks whose spec
names no intent, and specs that cannot be accepted because they name none. All three should be
getting shorter. The fourth is the queue in front of the owner — every draft intent is a question
somebody asked, and a draft nobody has been shown is the same as one nobody wrote. Drafts already
settled carry a non-blank `outcome:` and are not waiting on anybody:

```shell
grep -l '^status: draft$' docs/intents/*.md | while read f; do
  grep -q '^outcome:[[:space:]]*[^[:space:]]' "$f" || echo "$f"
done
```

**None of these finds a violation.** Each reads one layer and reports what is *missing*, so a file
that is well-formed and wrong matches none of them. Two checks in `.agents/skills/heartbeat` step 4
do more: a cross-layer loop that reads specs against intents, and a sweep that reads whether the
status lines are well formed at all. Both live there rather than here, so this list does not
restate what they catch and cannot fall behind them.

**A selector fails open, and that is why the sweep exists.** A grep that finds records by matching
a line stops seeing one written `status: draft ` with a trailing space, or saved with CRLF endings:
it leaves the queue silently instead of being reported. A gate predicate refuses on a mismatch; an
enumeration just stops seeing you. Catching that means asking whether the record is well formed,
which is a different question — and the sweep asks it of `docs/intents/` and `docs/specs/` only.
The greps over `tasks/*.md` have no such backstop, so a malformed task still opts out of its own
queue.

Nothing catches an intent an agent approved on its own judgement, by decision rather than by
oversight — `docs/decisions/008-approving-an-intent-is-an-instruction.md` says why and what it
costs. Read a clean grep as "nothing is waiting", never as "nothing is wrong".

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
   status lies. **That flip is only available once the intent it names reads `status: approved`.**
   A spec written against a draft intent merges as a draft and waits for the approval; a later
   one-line PR flips it, and step 2 does not start before that.
2. The work branch, `task-NNN-slug` — the code, plus the task's own status flips.

An intent reaches `main` the same way and earlier still, in a PR of its own. Approval is a second
one-line edit, `status: draft` to `status: approved`, and it travels through a PR like everything
else — an agent may carry that edit on the owner's word, whether it heard it directly or had it
relayed, and may never originate it. **That PR says where the approval came from**, which is the
only place the provenance is recorded and the reason no field in the file tries to.

**Both of those PRs stay drafts until the owner approves them, and an agent does not mark them
ready.** That is where the owner does the reading, so it is where the waiting is visible — see
`.agents/skills/pr-review` step 2.

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
