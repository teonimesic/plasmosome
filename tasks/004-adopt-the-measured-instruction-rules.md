---
id: 004
title: Adopt the instruction rules that passed the A/B test
status: todo
priority: 2
specs: [002]
intents: [001]
refs: [docs/specs/002-agent-instruction-rules.md, AGENTS.md, .agents/skills/pr-review/SKILL.md, crates/plasmosome-membrane/AGENTS.md, crates/plasmosome-backend/AGENTS.md]
done_when: >-
  every line in spec 002's Acceptance list is true of the tree, checked one by one,
  and the gate is green.
pr:
evidence:
---

## Why

See `docs/intents/001-rules-that-measurably-improve-agent-code.md`. Six candidate rules were
tested over 112 runs; four passed, one is new, one fails and must be recorded as failing so it is
not proposed again.

Two shipped rules also conflict today: the reap rule explains why it exists, and that makes the
model write the `//` comments the comment ban forbids. Spec 002 resolves it by moving reasoning
out of rule text into crate documents.

## Plan

## Notes
