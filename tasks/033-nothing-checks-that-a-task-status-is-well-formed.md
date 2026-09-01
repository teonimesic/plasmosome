---
id: 033
title: Nothing checks that a task status is well formed
status: todo
priority: 2
specs: [012]
intents: [008]
refs:
  [
    docs/specs/012-how-work-enters-the-tree.md,
    .agents/skills/heartbeat/SKILL.md,
    .agents/skills/tasks/SKILL.md,
  ]
done_when: >-
  `.agents/skills/heartbeat` step 4 sweeps `tasks/` for a well-formed `status:` line, alongside
  the two folders it already sweeps, over a glob that cannot pick up a future `README.md`, as the
  merged sweeps' `[0-9]*.md` already cannot. The set it accepts is exactly the five statuses the
  `## Lifecycle` table in `.agents/skills/tasks` lists, `in_progress` among them, and the sweep
  neither adds to that set nor drops from it. **The sweep derives its accepted set from the `##
  Lifecycle` table rather than restating it.** A list hardcoded in the sweep is the copy spec 012
  refuses, and declaring the sweep canonical does not rescue it: the table has to name each status
  to state that status's entry condition, so it stays a written copy whatever a sentence calls it.
  Two lists then drift, and the one every human reads is the stale one — a sweep built on it
  clears the wrong files and reports the right ones. The sweep prints one line per fault and
  prints nothing when run over `tasks/` as it stands on `main`. It prints the offending file for a
  `status:` line that is absent, empty, duplicated, not anchored at the start of its line, outside
  the set, or malformed, with `status:todo` for the no-space case, a trailing space, and CRLF
  endings as the three malformed shapes to test — each of those faults injected on its own into a
  scratch copy. The three malformed shapes are the ones spec 012 names as what a selector stops
  seeing; none has ever appeared on this tree, and the sweep exists so that the first one does not
  pass silently. On an empty input set it prints a refusal in its own words and exits non-zero, in
  both shapes of empty: `tasks/` missing, and `tasks/` present but holding nothing the sweep
  reads. **Neither shell hands back an empty list for free, and that is the trap the refusal has
  to survive**: measured, `bash` leaves an unmatched pattern standing as a literal, so the loop
  body runs once on a path that does not exist and the sweep reports a phantom file while exiting
  0; `zsh` aborts on `NOMATCH` before the loop runs at all, exiting non-zero with its own message
  rather than the sweep's. Exiting non-zero therefore does not by itself mean the sweep refused.
  So the sweep turns no-matches into an empty set explicitly — `shopt -s nullglob` and `setopt
  NULL_GLOB`, or a match count taken without globbing — and both shells then reach the same
  refusal, in the sweep's own words. Verified by running it under each shell rather than by
  reasoning about them. The same change retires the two now-false statements that `tasks/` is
  unswept — `.agents/skills/heartbeat` step 4 ("It covers `docs/intents/` and `docs/specs/`, and
  nothing else", through the sentence making the status vocabulary a precondition) and
  `.agents/skills/tasks` ("the sweep asks it of `docs/intents/` and `docs/specs/` only").
pr:
evidence:
---

## Why

Every list the heartbeat builds over `tasks/*.md` finds records by matching a `status:`, `specs:`
or `intents:` line. A selector like that fails open: a task written `status:todo`, or with a
trailing space, or saved with CRLF endings, leaves its queue silently instead of being reported.
It is not late, it is invisible, and the count that is supposed to notice is the thing that stops
seeing it.

The sweep added for `docs/intents/` and `docs/specs/` closes the malformed-record half of this
hole for those two folders, and was scoped to them deliberately.
`docs/specs/012-how-work-enters-the-tree.md` makes the general form a requirement: each of the
three folders holding records is swept for whether its state lines are well formed at all, and a
sweep prints faults, passes only in silence, and treats an empty input set as a refusal. `tasks/`
is the folder no part of that reaches.

**The empty-input half reaches none of the three today, including the two already swept**, which is
worth stating because it looks done. Run the merged sweep over an empty `docs/intents/` and `bash`
prints a fault for the literal pattern and exits 0, while `zsh` dies in the shell's own words —
neither is the sweep refusing. This task adds the refusal for `tasks/`; the same gap on the other
two folders is its own filing, and maps to the same clause of spec 012.

**The vocabulary question this filing waited on is answered, and only the sweep is left.** A sweep
over `tasks/` has to name the statuses it accepts, and at filing time the repository did not agree
on what they were: the `## Lifecycle` table listed five and no task file on the tree carried
`in_progress`. Writing the sweep then would have settled the vocabulary by freezing it into a
check, which is the one thing spec 012 says a sweep never does — where the written set and the tree
disagree, that is resolved before the sweep is written and not by it.

**`docs/specs/012-how-work-enters-the-tree.md` records the owner's ruling that `in_progress`
stays.** So the accepted set is the five the table lists, the question is closed, and the answer
came from a ruling a reviewer can go and read rather than from a check that asserted it — which is
how a person's judgement reaches this tree at all.

| Status | In the table | On the tree |
| --- | --- | --- |
| `todo` | yes | 8 |
| `planned` | yes | 2 |
| `in_progress` | yes | **0** |
| `in_review` | yes | 6 |
| `done` | yes | 12 |

Counts are from commit `17ace62`, read off `tasks/*.md` on `main` rather than carried from an
earlier session. They sum to 28 against 28 files in `tasks/`, every one carrying exactly one
`status:` line whose value is in the set — which is the reconciliation this table needs and not a
decoration. A count produced by the selector this task exists to distrust cannot otherwise tell a
real absence from a record that stopped being seen, and the bolded zero is precisely the number
that ambiguity would land on.

Re-running it later means not reaching for `grep -c '^status:'`, which is the selector rather than
a check: it prints a line per file instead of a total, and it accepts `status:todo` — the first
malformation named above. What reconciles is a count that rejects every shape the sweep must
reject, aggregates, and is compared against the number of records:

```shell
statuses=$(sed -n '/^## Lifecycle/,/^## /p' .agents/skills/tasks/SKILL.md |
  sed -n 's/^| `\([a-z_]*\)` |.*/\1/p')
ok=0; n=0
for f in $(find tasks -maxdepth 1 -name '[0-9]*.md' 2>/dev/null | sort); do
  n=$((n+1))
  [ "$(grep -c '^status:' "$f")" -eq 1 ] || continue
  printf '%s\n' "$statuses" | grep -qxF "$(sed -n 's/^status: //p' "$f")" && ok=$((ok+1))
done
[ "$n" -gt 0 ] || { printf 'no records read; refusing\n'; exit 1; }
printf '%s well-formed of %s records\n' "$ok" "$n"
```

**It reads the five values out of the `## Lifecycle` table rather than restating them**, on the same
rule `done_when` sets for the sweep — a command that hardcoded the set would be the second copy that
requirement exists to prevent, sitting in the file that asks for it. **And the value is compared as
a whole fixed line, `grep -qxF`, never as an anchored regex**: `grep -qE '^status: todo$'` still
passes a CRLF record, because `$` matches before the carriage return, so the obvious spelling of
this check silently admits the third malformation named above.

Run against a scratch copy holding one file of each shape — good, `status:todo`, a duplicated line,
a value outside the set, a trailing space, and CRLF — it prints `1 well-formed of 6 records`. On
`main` at `17ace62` it prints `28 well-formed of 28 records`, identical under `bash` and `zsh`; on
this branch, `29 well-formed of 29 records`, since the task file below is the twenty-ninth. Adding a
row to the `## Lifecycle` table in that scratch copy moves the first figure to `2 of 6`, and
removing it moves it back, which is what shows the set is derived and not transcribed.

**It enumerates with `find` rather than a glob, and refuses an empty result**, for the reason the
paragraph above gives and this command has to obey as much as the sweep does: `for f in
tasks/[0-9]*.md` reports `0 well-formed of 1 records` under `bash` when nothing matches — a false
failure, which is worse than the false pass, because it sends a reader hunting a malformation that
does not exist — and aborts under `zsh` before the loop. `find` yields nothing in both shells, so
the count falls to zero and the refusal is the command's own. Both shapes of empty were run: with
`tasks/` present and empty, and with `tasks/` missing, each under `bash` and `zsh`, all four
printing `no records read; refusing` at exit 1.

The zero is therefore a real absence, and still not evidence against the status. A count of files
sitting in `in_progress` at one instant measures how long work pauses there, not whether the state
exists: a status nobody rests in long enough to commit the line reads as zero on every snapshot,
and the entry condition the table gives it — branch `task-NNN-slug`, in the executor's own worktree
— is one a task can meet and leave inside a single session.

## Plan

## Notes

Filed from the round-8 review of PR #43, which asked whether that PR's sweep should have covered
`tasks/` too. It should, and at filing time it could not.

**Two merged files tell a reader that `tasks/` is unswept, and both go in the same change as the
sweep.** `.agents/skills/heartbeat` step 4 bounds the sweep, names the gap, and then names the
status vocabulary as the precondition for closing it — that last sentence is the one the ruling
above has discharged, and left alone it would go on telling every reader this work is blocked.
`.agents/skills/tasks` carries its own copy of the same gap. Retiring one and leaving the other is
the concrete way this lands half-done, which is why `done_when` names both rather than leaving it
to whoever picks the task up.

**Half of the reason spec 012 gives for the two-shell check survives measurement and half does
not, and `done_when` above carries the measured behaviour.** The spec says a glob matching nothing
"is fatal in one and empty in the other". **Fatal in one is right**: `zsh` raises `NOMATCH` and
aborts the whole script before the loop runs, so a line after the sweep never executes and the exit
is non-zero. **Empty in the other is wrong**: `bash` does not hand back an empty list, it leaves the
pattern standing as a literal, runs the loop once on a path that does not exist, and exits 0. The
requirement the spec sets — verify under both shells — is right and is kept. The correction for
whoever next edits that file is to the "empty" half only, and getting that scoped right matters,
because striking the sentence whole would delete a true statement about `zsh`.
