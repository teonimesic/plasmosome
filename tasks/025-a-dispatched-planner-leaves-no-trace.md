---
id: 025
title: A dispatched planner leaves no trace until it pushes
status: todo
priority: 2
specs: []
intents: []
refs:
  [
    .agents/skills/heartbeat/SKILL.md,
    .agents/skills/planning-work/SKILL.md,
    .agents/skills/tasks/SKILL.md,
  ]
done_when: >-
  a planner dispatched by step 4 is recorded before it has pushed anything,
  somewhere an agent in a different checkout can read — an unpushed branch does
  not count, which is the whole defect. Two heartbeats run back to back over the
  same draft spec or unspecced intent dispatch one planner between them, not two,
  and the second says which record told it to skip. A record whose planner died
  before it pushed does not silence that item for ever: the written procedure says
  how such a record is recognised and retried, and says it in a way a third
  heartbeat can follow. The same record, or another named in the same place, is
  what stops two agents claiming one `NNN` — the numbering check in
  `.agents/skills/tasks` reads the remote and cannot see a claim that has not been
  pushed.
pr:
evidence:
---

## Why

The heartbeat opens by saying that nothing which matters is remembered — everything that matters
is a file. Step 4 then breaks its own rule. It scans for a draft spec no task implements and an
intent with no spec, and for each says: "dispatch a planner for it, or say why not." Dispatching
writes nothing down. The planner is real work, in flight, and the only place it exists is the
memory of the orchestrator that started it.

So the next heartbeat scans the same two loops, finds the same spec still unimplemented — the
planner has not finished — and dispatches a second planner onto it. Two agents plan one thing, and
either two task files arrive for the same work or two branches collide over the same number.

Step 5 does not cover it. It classifies worktrees by what GitHub knows about their branch, and a
planner that has pushed nothing is a `NONE` row, which step 5 correctly refuses to interpret:
"only you can tell which." Telling requires the knowledge that vanished with the previous session.
The row also carries no link back to the spec or intent it came from, so even a settled `NONE`
does not answer the question step 4 is asking.

**This is not hypothetical, and the cheaper symptom bites first.** Auditing the merged PRs on
2026-08-31, I filed two tasks as 021 and 022, taking the numbers from every branch on the remote
exactly as `.agents/skills/tasks` prescribes. Another agent, working at the same time, had already
chosen 021 and 022 and had not yet pushed. Both of us followed the written procedure and both were
right; the procedure reads the remote, and a claim that has not been pushed is not on the remote.
I renumbered to 023 and 024. Nothing detected it — I found it by re-running the duplicate check
after pushing, which the skill does not ask anyone to do.

The two are the same shape: a claim that exists only in an agent's head until a push makes it
public, and every check the repository has looks at what has been pushed.

**What this is not.** PR #31 is reworking steps 5 and 6, so that liveness is decided once and a
row nobody could classify stops the count. That makes the rows that exist trustworthy; it does not
create a row for a planner that has produced none. Whoever takes this rebases on it, and should
read it first — the two changes land in the same file and the answer here may well be a bucket or
a step that one already has a place for.

**Where it came from.** CodeRabbit raised it on PR #26, as an "outside diff range" comment in the
review body. Those never become review threads, so the PR merged with zero unresolved threads and
nothing to answer.

## Plan

## Notes
