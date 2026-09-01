---
id: 038
title: Spec 012 says an unmatched glob comes back empty in bash, and it does not
status: in_progress
priority: 2
specs: [012]
intents: [008]
refs:
  [
    docs/specs/012-how-work-enters-the-tree.md,
    tasks/033-nothing-checks-that-a-task-status-is-well-formed.md,
  ]
done_when: >-
  The `## Acceptance` bullet in `docs/specs/012-how-work-enters-the-tree.md` that requires both
  shapes of empty to be tested still requires that verification under both `bash` and `zsh`, and
  the reason it gives names what each shell measurably does: `zsh` aborts on an unmatched glob
  before the loop runs, and `bash` leaves the pattern standing as a literal, so the loop runs once
  over a path that does not exist and exits zero. **Deleting the sentence does not satisfy this,
  and neither does striking the half about `bash`**: a reader of that bullet who has never used
  either shell must still be able to say, from the bullet alone, both that a sweep is run under
  two shells and why a sweep that finds nothing can come back silent and zero rather than empty.
  Nothing in the bullet, or anywhere else in the file, still describes `bash` as returning an
  empty list, an empty set, or nothing. The two commands below reproduce the measurement on the
  machine of whoever checks this, so the claim in the file is one they can refute rather than one
  they take on trust. `git diff origin/main` touches exactly two files, this one and that spec,
  and within the spec exactly the one sentence naming the two shells — no other line of 012 is
  reworded, and the requirement itself is neither weakened, strengthened, nor restructured.
pr:
evidence:
---

## Why

`docs/specs/012-how-work-enters-the-tree.md` requires every sweep over the record folders to
refuse an empty input set rather than pass, and requires that refusal to be verified under both
`bash` and `zsh`. The requirement is right. The reason it gave was half false, and the false half
is the one the requirement rests on.

Measured, in a directory holding no `.md` file:

```shell
bash -c   'for f in empty/*.md; do echo "GOT:[$f]"; done'   # GOT:[empty/*.md]        exit 0
zsh -f -c 'for f in empty/*.md; do echo "GOT:[$f]"; done'   # no matches found        exit 1
```

"Fatal in one" is `zsh`, and it is true. "Empty in the other" is `bash`, and it is not: `bash`
leaves the unmatched pattern standing as a literal, so the loop body runs exactly once over a path
that does not exist, and the script exits zero.

That is not a wording preference. A sweep written by someone reading the old sentence would expect
`bash` to hand it nothing to iterate, so it would expect its own refusal to fire. What actually
happens is that the sweep reads one phantom file, finds no fault in a file it cannot open, prints
nothing, and exits zero — a pass produced by reading nothing at all. Spec 012 names that shape by
its own name two sentences earlier: it is the same as a green that reviewed nothing, which is the
failure the empty-input refusal exists to catch. The spec was arguing for the right check out of a
premise that hands the check its own failure mode back.

`tasks/033-nothing-checks-that-a-task-status-is-well-formed.md` had already measured this while
being written, carries the corrected behaviour in its `done_when`, and its `## Notes` scope the
correction to the `bash` half and warn that striking the sentence whole would delete a true
statement about `zsh`. That task is still `todo`, so the sweep it describes is unbuilt and no
reader has been misled into writing one yet. This is the correction landing in the file the next
reader actually opens, before the sweep is written against it.

## Plan

The deliverable is one sentence in `docs/specs/012-how-work-enters-the-tree.md`, in the
`## Acceptance` bullet beginning "Each sweep prints nothing on the tree as it stands".

Out of scope: the requirement itself, which is correct and does not move; every other line of 012;
the sweep of `tasks/`, which is task 033's; the same "empty in the other" claim wherever it may
appear in a file this change does not otherwise touch; and building or fixing any sweep.

Files to read, and nothing beyond them:

- `docs/specs/012-how-work-enters-the-tree.md` — the `## Acceptance` section.
- `tasks/033-nothing-checks-that-a-task-status-is-well-formed.md` — `## Notes`, which measured
  this first and scopes the correction.

There is no test table. The change is a claim about two shells, and what stands in for a test is
the pair of commands in `## Why` above, run on the machine of whoever is checking, against a
directory holding no `.md` file. A reader who gets different output has refuted the sentence, which
is the whole of what makes it checkable.

Done is `done_when` above, plus the gate in the root `AGENTS.md`: `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`,
`./.githooks/provenance-guard`, `./.githooks/attribution-guard`, all exiting zero.

STOP when done — do not start the next piece of work.

## Notes
