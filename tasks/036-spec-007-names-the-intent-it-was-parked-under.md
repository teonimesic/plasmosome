---
id: 036
title: Spec 007 names the intent it was parked under, not the one that wants it
status: todo
priority: 2
specs: [012]
intents: [008]
refs:
  [
    docs/specs/007-publishing-pipeline.md,
    docs/intents/013-widely-used-and-successful.md,
  ]
done_when: >-
  `docs/specs/007-publishing-pipeline.md` names in `intents:` the approved intent that asks for
  the crates to be distributed, rather than the testing intent it names today, and every task
  naming spec 007 carries the same value.
pr:
evidence:
---

## Why

Spec 007 describes how the crates reach crates.io, and its `intents:` names 002, which asks for a
test system. That is where it came from: 007 was written in the split of intent 002 into
test-system specs, when nothing in the repository asked for the crates to be distributed and there
was nowhere else to attach it. `docs/intents/013-widely-used-and-successful.md` now asks for
exactly that, is `approved`, and names 007 by number as the spec that should descend from it.

The link is not decorative. It is what the gate on a spec becoming `accepted` reads, and it is
what anyone walking upward from 007 follows to find out whether the work is wanted. Both give the
wrong answer today, and they give it in the direction that is hard to notice: 002 is `approved`,
so the gate passes. On the day someone accepts 007, nothing stops them, and what they have cleared
is a spec whose stated parent does not ask for what it builds. A reader gets the same silence — the
walk ends in the test system and reads as though publishing were a testing concern.

The edit is one line. `002` appears nowhere else in the file, checked at `17ace62`. Nothing on the
tree copies the value yet — at filing, no task names spec 007 — but task 037 does, so the second
half of the line above is what keeps the two from disagreeing.

## What this does not change

007 stays `draft`, and re-parenting clears none of the four blockers it lists — those are the
owner's decisions and this task does not touch them. One of the four has since been answered
elsewhere, which is a separate edit to the same file and deliberately not carried here: a task that
re-parents a spec and also rewrites what it is blocked on is two units of work sharing a diff.

## Plan

## Notes
