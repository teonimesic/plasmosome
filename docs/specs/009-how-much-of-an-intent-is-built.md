---
id: 009
title: How much of an intent has been built, recorded in the intent and checkable against the tree
status: draft
intents: [008]
---

## Behavior

The work chain records what each piece of work points at: a task names a spec, a spec names an
intent. Read upward it answers "was this asked for". Read downward it answers nothing. An intent
says what is wanted and never says whether any of it exists, so `docs/intents/` cannot tell a goal
that is nearly met from one nobody has started, and the only way to find out is to read every spec
and every task and form the judgement again.

This spec puts that answer in the intent, split along the line that decides whether it stays true.
**The judgement is written down; everything mechanical is derived.** Whether a goal is
substantially met, and what is left of it, is a reading of the goal against what exists — it does
not go stale when a task merges. Which specs name the intent, what their statuses are, which tasks
name those specs and how many are still open all change every day and are a `grep` away, so no
intent file carries them.

A typed list of specs and tasks would be wrong within a week and would read as authoritative while
it was. That is not hypothetical. A pull request on this repository carried a count of the
repository's own pull requests and was wrong twice: once from a sample that missed rows, and once
because the true number moved from 43 to 49 while the branch sat open. Nothing about that count was
careless. It was a derived fact typed into prose, which is the failure this shape exists to avoid.

**No intent is ever expected to be finished.** An intent is a direction, and asking when one is
exhausted has no answer for most of them. The vocabulary below therefore has no terminal value: the
top of the scale is "substantially", not "complete", and an intent may sit there permanently with
open work beneath it. Anything that treats leftover work under a well-served goal as a defect has
misread the field.

### Three fields, three different questions

`served:` is a new frontmatter field on an intent. It sits beside two others, and the whole risk in
adding it is that a reader takes it for one of them. Neither neighbour is on `main` yet: `status:`
and `outcome:` arrive together with the change that introduces intent approval, and the table below
is the shape of an intent's frontmatter once they have. `docs/templates/intent.md` on `main` today
carries neither field and defines only a `## Outcome` section.

| Field | The question it answers | Values |
| --- | --- | --- |
| `status:` | Does the owner want this? | `draft`, `approved` |
| `served:` | How much of it exists? | `none`, `partly`, `substantially` |
| `outcome:` | Is this still open? | blank while open, non-blank once settled |

**`status:` and `served:` vary independently, and every combination of the two is legal.**
`outcome:` is not a free third axis and does not join that claim: it is blank while the intent is
open and takes a word once it settles, exactly as the table says, and nothing here changes it. The
independence is a design commitment rather than an observation, and the combination worth stating
outright is the one that looks wrong and is not: a `draft` intent may carry any of the three values,
`substantially` included. **Existence does not wait for approval.** An intent drafted over an area
where code already exists is exactly the backfill `docs/intents/README.md` invites — anyone may
draft the intent that closes a gap under work already finished — and `partly` is the truthful value
for it. Nothing may be *committed to* under a draft goal; that gate is `status:`, and it is not this
field's business. A check that refused `draft` with work beneath it would be reading the approval
axis off the existence axis, which is the confusion this whole design exists to prevent.

No value of one field is a value of another, so a reader who confuses two of them is contradicted by
the words themselves rather than by a rule they have to know.

The three values are the whole vocabulary and there is deliberately no fourth. `none` is nothing
built. `partly` is some of it built. `substantially` is most of what was asked for built, with what
remains named in the prose. The boundaries are judgements, which is what the field is for; a scale
fine enough to argue about would be a scale nobody keeps current.

### Where the prose goes, and why not `## Outcome`

The intent template ships a `## Outcome` section, and it is the obvious home until you look at what
it is paired with. `outcome:` is a terminal marker — blank while the intent is open, non-blank once
it is settled — and the section carries the same name and the same tense: what was built, or why
nothing was, written at the end. Coverage is the opposite tense. It is the running answer, most
useful precisely while the intent is open, and it is revised many times before anything settles.

Putting a running record under a heading whose frontmatter twin means "finished" gives one word two
tenses. A reader finding prose under `## Outcome` could no longer tell whether they were reading a
progress note or an epitaph. So `## Outcome` and `outcome:` keep the meaning they have, and the
judgement gets a section of its own:

```markdown
## What is served

(filled in as work lands) How much of this goal exists today, and what is left of it.
Name what is missing, not which specs or tasks exist — those are derived below.
```

It holds two things and nothing else: what of the goal exists, and what is left. It names no spec
id, no task id, and no counts. An intent whose `## What is served` lists the specs beneath it has
reintroduced the stale copy this spec exists to prevent, and a reviewer should refuse it on sight.

**When an intent settles, `## What is served` stops being updated and stays where it is.** It
becomes the record of what existed at the end, and `## Outcome` says why the goal stopped. The two
are not one story told twice: the first says what was built, the second says why nothing more will
be. A settled intent whose `## Outcome` merely restates `## What is served` should lose one of them,
and the one to lose is `## Outcome` — the frontmatter `outcome:` already carries the fact that it
settled.

### Who moves it

`served:` and `## What is served` are the owner's judgement, and a pull request that changes either
stays a draft until the owner takes it out, exactly as one proposing an intent does. An agent that
notices a merged task moved a goal forward raises it that way: opens the edit as a draft, says what
landed, and lets the owner decide the value.

**The reason is not the one that puts approval with the owner, and borrowing that reasoning would be
wrong.** `docs/decisions/008-approving-an-intent-is-an-instruction.md` puts approval there because
an approved intent multiplies into specs and tasks — one decision committing a queue. `served:`
commits nothing; no gate reads it and nothing downstream waits on it. It is the owner's for a
different reason: **the question is not answerable from the tree.** "How much of this goal exists"
compares what was built against what was wanted, and only the person who wanted it holds the second
half. An agent can read every spec and task beneath an intent and still not know whether they add up
to the thing that was asked for. That is why the derived half below can be computed by anyone and
this half cannot.

The cost is stated rather than hidden. A judgement only the owner can revise goes stale between his
readings, and the check catches only the loud ways. Letting an agent set the value from what it can
see in the tree was rejected for collapsing the two halves back together: the derived facts would be
laundered into a judgement, and the file would claim an authority it did not have.

### The check

The point of the field is that it can be contradicted. An intent claiming a coverage the tree does
not support is a defect a script can find, and a convention nobody can notice drifting is the shape
this repository has already decided not to add more of. It belongs beside the cross-layer loop in
`.agents/skills/heartbeat` step 4, and reads the same way: it prints faults, and silence is the only
passing answer.

**Work has landed** under an intent when a task with `status: done` reaches it either way:

- **through a spec** — the task's `specs:` names a spec whose `intents:` names the intent; or
- **directly** — the task's own `intents:` names the intent.

**Both legs are required, and the second is not defensive.** `.agents/skills/tasks` keeps the
task-level `intents:` copy so that a search over tasks and a search over specs return the same
answer; a check walking only the spec leg makes them differ. On the tree today four `done` tasks
carry `specs: []`, and `tasks/007-adopt-the-measured-instruction-rules.md` names `intents: [001]`
directly — finished work under intent 001 that a spec-only walk cannot see, which would let intent
001 sit at `served: none` forever without a word.

The three per-intent faults, and nothing else:

- **`served: none` with work landed.** Something was built under this goal and the file says nothing
  was. This is the staleness case, and it is the common one: it fires the first time a task closes
  under a goal nobody has revisited.
- **`served: substantially` with no work landed.** The file claims most of the goal exists and
  nothing under it has shipped.
- **A file that does not carry exactly one well-formed `served:` line.** Malformation rather than
  contradiction, reported the same way. Absent counts because the field is required and a file that
  never received it is exactly what needs finding — an intent merging from a branch written before
  this spec is the shape that will hit it. Empty counts, and so does a value outside the three. So
  does a *second* `served:` line: the check reads the field by matching a line, a file carrying two
  has whichever one sorts first silently chosen for it, and a file that says two things says
  nothing.

**The check never reads an *intent's* `status:`.** It does read a *task's* `status:`, and only to
tell landed work from open work — that is the `done` test above, and it is the only `status:` the
check has any use for. An intent's approval is invisible to it: approval and existence are
independent, so no combination of the two is a fault, and a `draft` intent carrying `substantially`
is not the check's business. An earlier draft of this spec had that fault and it was wrong twice
over: it fired on the value alone with no landed-work condition, and it left a truthful shape — a
draft intent over work that already exists — with no legal value at all.

**Silence needs a floor, because silence is the passing answer.** The check reads relative paths in
the working tree, so a run started from the wrong directory finds no intent files, prints nothing,
and is indistinguishable from a clean tree. That is the same shape as a review status that reads
green because no review ran. So **an empty input set refuses, never passes**: finding no intent file
carrying an `id:` is reported and the run refuses, rather than passing quietly. One condition buys
the difference between "nothing is wrong" and "nothing was read", and without it every other
guarantee here is conditional on a fact nobody checked.

**That refusal is not a fourth fault, and the distinction is what makes it implementable.** The
three above are per-intent: each names the intent file it fires on, and finding them is what a run
does. This one is about the run itself, has no intent file to name, and is the answer to whether
there was a run at all. It is reported in the check's own words rather than in the one-line-per-fault
shape, and it ends the run.

This is the whole of the provenance question the check can answer on its own. It names its inputs,
and it refuses when they are absent. It cannot tell which *repository* it is standing in, and it
does not try — that is a property of how it is invoked, not of what it reads.

**What it must not flag, stated because it is the tempting check and it is wrong.** Open tasks
beneath an intent marked `served: substantially` are not a contradiction. Substantial is not
exhausted, no intent is expected to be exhausted, and a check that treated leftover work as a fault
would fire on every healthy goal in the folder and be switched off within a week.

**No state of the work beneath an intent can contradict `partly`.** It is compatible with landed
work and with none, so only the malformation fault can ever name a `partly` intent, and only by
being unreadable rather than by being wrong. That is a real limit and the reason this is a floor
rather than a proof: it catches the two extremes going stale and catches nothing in the middle. An
intent drifting from `partly` to `substantially` is caught by the owner reading, and by nothing
else.

### Reading the derived half

Nothing above replaces reading the tree, and the command that reads it is what an intent file
points at instead of listing anything. For one intent: its specs with the tasks under each, then any
task that reaches the intent directly.

**It walks both legs, for the same reason the check does.** A reader forming this judgement is
reading to answer "how much of this exists", and a spec-only walk answers it wrong in exactly the
place the check was built to catch: no spec on the tree names intent 001, so a spec-only command
prints nothing for it while `tasks/007-adopt-the-measured-instruction-rules.md` sits `done` beneath
it. A command that hides finished work from the person setting `served:` is worse than no command.

```shell
n=008
shown=""
for f in docs/specs/*.md; do
  grep -q "^intents:.*\b$n\b" "$f" || continue
  sid=$(sed -n 's/^id: *//p' "$f" | head -1)
  printf '%s\t%s\n' "$(sed -n 's/^status: *//p' "$f" | head -1)" "$f"
  for t in tasks/*.md; do
    grep -q "^specs:.*\b$sid\b" "$t" || continue
    printf '  %s\t%s\n' "$(sed -n 's/^status: *//p' "$t" | head -1)" "$t"
    shown="$shown|$t|"
  done
done
for t in tasks/*.md; do
  grep -q "^intents:.*\b$n\b" "$t" || continue
  case "$shown" in *"|$t|"*) continue ;; esac
  printf '%s\t%s\n' "$(sed -n 's/^status: *//p' "$t" | head -1)" "$t"
done
```

A task that carries both links is printed once, under its spec: `.agents/skills/tasks` has every
task copy its spec's `intents:` down, so the common case is a task matching both legs, and a
command that listed it twice would read as two pieces of work. The second loop prints only what the
first did not — `shown` is why it exists, and it is a string rather than an array because array
syntax and unquoted word splitting differ between `bash` and `zsh`.

Ids are read from each file's `id:` rather than from its filename, for the reason the heartbeat's
loop already gives: a glob over a dangling id aborts the loop it was meant to report on.

## Contract

- **`served:`** is a required frontmatter field on every intent file, appearing **exactly once**, on
  its own line, anchored at the start of the line, inside the frontmatter and directly after
  `status:`. Its value is exactly one of `none`, `partly`, `substantially`.
  `docs/templates/intent.md` ships `served: none`, so a copied and unfilled file claims nothing.
- **`## What is served`** is a section of `docs/templates/intent.md`, placed before `## Outcome`.
  It holds what of the goal exists and what is left. It contains no spec id, no task id and no
  count. It stops being updated once the intent settles and is not deleted.
- **`status:`, `outcome:` and `## Outcome` are unchanged** by this spec, in meaning and in
  placement. No fault in the check reads an intent's `status:`; the only `status:` it reads is a
  task's, to tell landed work from open.
- **Every intent file already on `main` gains the field.** The value is the owner's; the mechanical
  part of the backfill is that no intent file is left without one.
- **The task under this spec runs after `status:` exists.** `served:` is placed relative to
  `status:`, and no intent on `main` carries that field yet — it arrives with the change that
  introduces intent approval. Sequencing the backfill after it keeps one placement rule instead of
  two, and avoids a second pass to move every line. This is an ordering constraint on the task, not
  a second gate: nothing about this contract changes if the two land in the other order, only the
  amount of editing.
- **The check** lives in `.agents/skills/heartbeat` step 4 beside the existing cross-layer loop.
  Its inputs are `docs/intents/`, `docs/specs/` and `tasks/`. It **validates** only the files in
  `docs/intents/` that carry an `id:`, so the folder's `README.md` is neither validated nor
  reported; it reads the other two folders to derive whether work has landed.
- **Per intent**, it prints one line per fault naming the intent file and the fault, and prints
  nothing on a tree where every intent's `served:` is well-formed and unrefuted. There are exactly
  three per-intent faults: `served: none` with work landed; `served: substantially` with no work
  landed; and a file not carrying exactly one well-formed `served:` line — absent, empty,
  duplicated, or outside the three values. It reports nothing about open tasks under any value,
  nothing about an intent's `status:`, and nothing that contradicts `partly`.
- **"Work has landed"** is true when some task with `status: done` either names a spec whose
  `intents:` names the intent, or names the intent directly in its own `intents:`. Either leg
  suffices.
- **A spec naming an intent id that no intent file carries does not abort the check**; the intents
  that do exist are still validated. Ids are resolved by reading each file's `id:`, never by
  globbing a filename.
- **An empty input set is a run-level refusal, outside the three per-intent faults.** If no file in
  `docs/intents/` carries an `id:`, there is no intent file to name and the one-line-per-fault shape
  does not apply: the check prints its own refusal, naming the input set it did not find, and exits
  non-zero without validating anything. It is not counted among the three, and a run that refuses
  reports no per-intent faults at all — it did not read far enough to have any. This is what stops a
  run in the wrong working directory being read as a clean tree.
- **Callers may rely on** the check being a floor and not a proof: silence means no intent makes a
  claim the tree flatly contradicts, never that any intent's coverage is accurate.
- **A pull request changing `served:` or `## What is served` stays a draft** until the owner takes
  it out of draft, and says where the judgement came from. `.agents/skills/pr-review` step 2 gains
  that trigger; `.agents/skills/tasks` and `docs/intents/README.md` gain the field and its values.
  Nothing mechanical enforces it.

## Acceptance

- `docs/templates/intent.md` carries `served: none` directly after `status:`, and a
  `## What is served` section before `## Outcome`.
- Every intent file — every file in `docs/intents/` carrying an `id:`, which excludes `README.md` —
  carries **exactly one** `^served:` line, sitting in the frontmatter on the line directly after
  `status:`, whose value is one of the three. Counting the files that match
  `grep -E '^served: (none|partly|substantially)$'` does **not** establish this and is not the
  criterion: that count passes a file carrying both `served: none` and `served: mostly`, and passes
  one whose only `served:` line sits in the body. Count `^served:` lines per file, assert one, then
  assert its position and its value.
- `docs/intents/README.md` states the three values and the question the field answers, states that
  the field is the owner's to move, and states that it is independent of `status:`.
- `.agents/skills/tasks` lists intent `served:` beside intent `status:` in its field-value list, and
  neither section restates the other's rule.
- `.agents/skills/pr-review` step 2 names a `served:` change as a pull request that stays a draft.
- No intent file names a spec id, a task id or a count in `## What is served`.
- The check prints nothing on the tree as it stands **after the backfill in the same change** — the
  tree this spec merges into has no `served:` field at all, so the clean run is asserted against the
  post-backfill tree.
- The check prints the offending file for each of the three faults, injected one at a time into a
  scratch copy: an intent flipped to `served: none` while a `done` task reaches it; one flipped to
  `served: substantially` with nothing done beneath it; and one carrying `served: mostly`.
- The check prints the offending file for an intent whose `served:` line is **deleted**, for one
  whose `served:` line is present but empty, and for one carrying **two** `served:` lines — planted
  as `served: none` and `served: mostly` in the same file, the shape a per-file line count catches
  and a match count does not.
- The check prints nothing for a `status: draft` intent carrying `served: partly`, and nothing for a
  `status: draft` intent carrying `served: substantially` with work landed beneath it.
- The check prints nothing for an intent marked `served: substantially` that has open tasks beneath
  it, verified by planting exactly that shape.
- The check prints nothing for an intent marked `served: partly` in any of those shapes.
- **The second landing leg is exercised:** with `tasks/007-adopt-the-measured-instruction-rules.md`
  `done`, `specs: []` and `intents: [001]`, intent 001 at `served: none` prints the staleness fault
  and at `served: substantially` prints nothing. A check walking only the spec leg gets both
  backwards, which is what this bullet is for.
- The check says nothing about `docs/intents/README.md`, which carries no `id:`.
- Run from a directory with no `docs/intents/`, and from one where `docs/intents/` holds only files
  without an `id:`, the check prints **its own** refusal and exits non-zero — verified in both
  shapes and under both shells, because a silent pass there is indistinguishable from a clean tree.
  Leaving this to the shell is not enough: `zsh` aborts on a glob that matches nothing, so an
  unguarded implementation dies with a shell error under `zsh` and prints the check's refusal only
  under `bash`. Test the directory before globbing it.
- A planted spec carrying `intents: [099]`, which no intent file has, does not abort the check:
  every real intent is still validated in the same run.
- The check runs clean under both `bash` and `zsh`. Two `zsh` traps are already known and neither
  may resurface: `status` is a read-only variable name, and a glob matching nothing is a fatal
  error rather than an empty list.
- The derivation command in this spec, run for an intent with at least one spec, prints that spec
  and the tasks naming it. Run for **intent 001** — which no spec names, and which
  `tasks/007-adopt-the-measured-instruction-rules.md` reaches directly — it prints that task. It
  prints nothing, and exits 0, only for an intent neither leg reaches.
- The derivation command prints a task carrying **both** links exactly once, under its spec:
  verified against `tasks/004-testkit-and-seams.md`, which names `specs: [003]` and `intents: [002]`,
  in a run for intent 002. It runs identically under `bash` and `zsh`, which is why the seen-set is a
  string tested with `case` rather than an array.

## Out of scope

- **Building any of it.** This spec is the contract; the field, the template and README edits, the
  backfill and the check are one task under it, and that task is the first work this spec generates.
  Splitting them is not caution — a spec lands in its own pull request before the work branch
  exists, and the check cannot be written against a field no file carries yet.
- **Deciding any intent's value.** Nothing here says how much of any goal is built. Every value in
  the backfill is the owner's, gathered the way the draft-pull-request convention says.
- **Rolling the field up.** No summary across intents, no dashboard, no count of how many goals are
  substantially served. The folder is small enough to read, and a rollup is a derived fact with
  somewhere to go stale.
- **Making `partly` checkable.** It would need a measure of how much of a goal a spec covers, which
  is the judgement the field exists to hold rather than a fact a script can read.
- **A `superseded` or `abandoned` coverage value.** A goal that stops being wanted is already
  expressible: the intent settles and `outcome:` says so. A second way to say it would be a second
  source of truth.
- **Changing what `status:` or `outcome:` mean.** Both keep the definitions
  `docs/intents/README.md` gives them.
- **A decision record.** The alternatives turned down here — an agent deriving `served:` from the
  tree, folding coverage into `status:`, reusing `## Outcome` — are argued above rather than in
  `docs/decisions/`. The first of them is the one that will be argued for again, because deriving
  the value looks like less work every time someone meets a stale field. **If it is raised a second
  time, that is the trigger to write the record** rather than to re-run the argument from here.
