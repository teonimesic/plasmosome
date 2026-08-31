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

`served:` is a new frontmatter field on an intent. It sits beside two that already exist, and the
whole risk in adding it is that a reader takes it for one of them.

| Field | The question it answers | Values |
| --- | --- | --- |
| `status:` | Does the owner want this? | `draft`, `approved` |
| `served:` | How much of it exists? | `none`, `partly`, `substantially` |
| `outcome:` | Is this still open? | blank while open, non-blank once settled |

They are independent, and every combination of them occurs. A refused draft is `status: draft`,
`served: none`, with `outcome:` filled. A goal half built is `status: approved`, `served: partly`,
`outcome:` blank. A goal that is mostly there and will never be closed is `status: approved`,
`served: substantially`, `outcome:` blank for good. No value of one field is a value of another, so
a reader who confuses two of them is contradicted by the words themselves rather than by a rule
they have to know.

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

### Who moves it

`served:` and `## What is served` are the owner's judgement, and they move the way `status:` moves:
an agent may carry the owner's word, relayed or heard directly, and may never originate it. The
reasoning is the same one `docs/decisions/008-approving-an-intent-is-an-instruction.md` gives for
approval — only the person who wanted the goal can say whether what exists meets it — and it needs
no second rule, only one more trigger for a convention that already exists.

That trigger is the draft-pull-request convention. A pull request that changes `served:` or
`## What is served` stays a draft until the owner has read it and taken it out of draft, exactly as
one proposing an intent does. An agent noticing that a merged task has moved a goal forward raises
it that way: open the edit as a draft, say what landed, and let the owner decide what the new value
is. Nothing mechanical enforces this, for the reasons that record already sets out.

The cost is stated rather than hidden. A judgement only the owner can revise is a judgement that
goes stale between his readings, and the check below catches only the two ways it can go stale
loudly. The alternative — letting an agent set the value from what it can see in the tree — was
rejected because it collapses the two halves back together: the derived facts would be laundered
into a judgement, and the file would claim an authority it did not have.

### The check

The point of the field is that it can be contradicted. An intent claiming a coverage the tree does
not support is a defect a script can find, and a convention nobody can notice drifting is the shape
this repository has already decided not to add more of. It belongs beside the cross-layer loop in
`.agents/skills/heartbeat` step 4, and reads the same way: it prints faults, and silence is the only
passing answer.

Work has **landed** under an intent when some task with `status: done` names a spec whose `intents:`
names that intent. That is the one derived fact the check needs.

The two contradictions, and nothing else:

- **`served: none` with work landed.** Something was built under this goal and the file says nothing
  was. This is the staleness case, and it is the common one: it fires the first time a task closes
  under a goal nobody has revisited.
- **`served: substantially` with no work landed.** The file claims most of the goal exists and
  nothing under it has shipped.

Two more faults are malformation rather than contradiction, and the loop reports them the same way:
a `served:` value outside the three, and a `served:` above `none` on an intent whose `status:` is
still `draft`, which says work landed under a goal the owner has not approved.

**What it must not flag, stated because it is the tempting check and it is wrong.** Open tasks
beneath an intent marked `served: substantially` are not a contradiction. Substantial is not
exhausted, no intent is expected to be exhausted, and a check that treated leftover work as a fault
would fire on every healthy goal in the folder and be switched off within a week.

**`partly` is compatible with every state of the tree**, so the check can never contradict it. That
is a real limit and the reason this is a floor rather than a proof: it catches the two extremes
going stale and catches nothing in the middle. An intent drifting from `partly` to
`substantially` is caught by the owner reading, and by nothing else.

### Reading the derived half

Nothing above replaces reading the tree, and the command that reads it is what an intent file
points at instead of listing anything. For one intent, its specs and the tasks under them:

```shell
n=008
for f in docs/specs/*.md; do
  grep -q "^intents:.*\b$n\b" "$f" || continue
  sid=$(sed -n 's/^id: *//p' "$f" | head -1)
  printf '%s\t%s\n' "$(sed -n 's/^status: *//p' "$f" | head -1)" "$f"
  for t in tasks/*.md; do
    grep -q "^specs:.*\b$sid\b" "$t" &&
      printf '  %s\t%s\n' "$(sed -n 's/^status: *//p' "$t" | head -1)" "$t"
  done
done
```

Ids are read from each file's `id:` rather than from its filename, for the reason the heartbeat's
loop already gives: a glob over a dangling id aborts the loop it was meant to report on.

## Contract

- **`served:`** is a required frontmatter field on every intent, on its own line, anchored at the
  start of the line, directly after `status:`. Its value is exactly one of `none`, `partly`,
  `substantially`. `docs/templates/intent.md` ships `served: none`, so a copied and unfilled file
  claims nothing.
- **`## What is served`** is a section of `docs/templates/intent.md`, placed before `## Outcome`.
  It holds what of the goal exists and what is left. It contains no spec id, no task id and no
  count.
- **`status:`, `outcome:` and `## Outcome` are unchanged** by this spec, in meaning and in
  placement.
- **Every intent already on `main` gains the field.** The value is the owner's; the mechanical part
  of the backfill is that no intent is left without one.
- **The check** lives in `.agents/skills/heartbeat` step 4 beside the existing cross-layer loop.
  It prints one line per fault naming the intent file and the fault, and prints nothing on a clean
  tree. It reports exactly four faults: `served: none` with a `done` task reaching the intent
  through any spec that names it; `served: substantially` with no such task; a `served:` value outside the
  three; and `served:` above `none` on a `status: draft` intent. It reports nothing about open tasks
  under any value, and nothing about `partly`. It reads only the files in `docs/intents/` that
  carry an `id:`, so the folder's `README.md` is skipped rather than reported as malformed.
- **Callers may rely on** the check being a floor and not a proof: silence means no intent makes a
  claim the tree flatly contradicts, never that any intent's coverage is accurate.
- **A pull request changing `served:` or `## What is served` stays a draft** until the owner takes
  it out of draft, and says where the judgement came from. `.agents/skills/pr-review` step 2 gains
  that trigger; `.agents/skills/tasks` and `docs/intents/README.md` gain the field and its values.
  Nothing mechanical enforces it, per
  `docs/decisions/008-approving-an-intent-is-an-instruction.md`.

## Acceptance

- `docs/templates/intent.md` carries `served: none` directly after `status:`, and a
  `## What is served` section before `## Outcome`.
- Every file in `docs/intents/` carries a `served:` line whose value is one of the three;
  the count of files matching `^served: (none|partly|substantially)$` equals the number of
  intent files.
- `docs/intents/README.md` states the three values and the question the field answers, and states
  that the field is the owner's to move.
- `.agents/skills/tasks` lists intent `served:` beside intent `status:` in its field-value list, and
  neither section restates the other's rule.
- `.agents/skills/pr-review` step 2 names a `served:` change as a pull request that stays a draft.
- No intent file names a spec id, a task id or a count in `## What is served`.
- The check prints nothing on the tree as it stands.
- The check prints the offending file for each of the four faults, injected one at a time into a
  scratch copy of the tree: an intent flipped to `served: none` while a `done` task reaches it
  through a spec that names it; one flipped to `served: substantially` with nothing done beneath it; one
  carrying `served: mostly`; and a `status: draft` intent carrying `served: partly`.
- The check prints nothing for an intent marked `served: substantially` that has open tasks beneath
  it, verified by planting exactly that shape.
- The check prints nothing for an intent marked `served: partly` in any of those shapes.
- The check survives an intent whose `intents:` reference points at no file, and reports rather than
  aborting — verified by planting a spec naming intent `099`.
- The check says nothing about `docs/intents/README.md`, which carries no `id:`.
- The check runs clean under both `bash` and `zsh`.
- The derivation command in this spec, run for an intent with at least one spec, prints that spec
  and the tasks naming it.

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
