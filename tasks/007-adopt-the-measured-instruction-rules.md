---
id: 007
title: Adopt the instruction rules that passed the A/B test
status: todo
priority: 2
specs: []
intents: [001]
refs: [docs/decisions/001-instruction-rules-measured.md, AGENTS.md, crates/plasmosome-membrane/AGENTS.md, crates/plasmosome-backend/AGENTS.md]
done_when: >-
  crates/plasmosome-backend/AGENTS.md states the dependency-seam rule and its
  two-adapter brake; the pid rule reads identically wherever it appears, with one
  file authoritative and any other mention a pointer; no AGENTS.md and no
  .agents/skills/**/SKILL.md tells anyone to retry EINTR; and the gate is green.
pr:
evidence:
---

## Why

See [`docs/decisions/001-instruction-rules-measured.md`](../docs/decisions/001-instruction-rules-measured.md).
Six rules were tested over 112 runs. One is new and measurably works, one is dead weight and is
being removed before someone re-proposes it, and the pid rule currently lives only in the membrane
crate although it governs any crate touching pids.

Deliberately not in scope: the comment collision. Every appended rule raises comment output and
nothing in the experiment identifies why, so there is no change to make yet — only an open
question recorded in the decision.

## Plan

## Notes
