---
id: 007
title: Adopt the instruction rules that passed the A/B test
status: done
priority: 2
specs: []
intents: [001]
refs: [docs/decisions/001-instruction-rules-measured.md, AGENTS.md, crates/plasmosome-membrane/AGENTS.md, crates/plasmosome-backend/AGENTS.md]
done_when: >-
  exactly one instruction file states the dependency-seam rule and its two-adapter
  brake; exactly one states the pid rule; no AGENTS.md and no
  .agents/skills/**/SKILL.md tells anyone to retry EINTR; and the gate is green.
pr: https://github.com/teonimesic/plasmosome/pull/6
evidence: >-
  squash commit 2fb281b on main adds the seam rule to AGENTS.md Style; the pid rule
  and the EINTR absence were verified unchanged and recorded in Notes.
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

**2026-08-31.** Two of the three items needed no change, and the `done_when` was rewritten to say
what is actually wanted — one copy of each rule — rather than to name a file.

The seam rule went to root `AGENTS.md` under Style, not to
`crates/plasmosome-backend/AGENTS.md`. It is a general rule about where a test seam belongs, and
the task that measured it was supervisor code, not backend code. Root is the only file every
agent loads, and a copy in a crate would be the duplicate `AGENTS.md` itself forbids two sections
earlier. The backend crate already carries the rule's backend-shaped consequence — the fake
backend is a model, not a stub — and that stays where it is.

The pid rule was left in `crates/plasmosome-membrane/AGENTS.md`, unchanged. It already appears
exactly once. Promoting it to root would either duplicate it or take it away from the only crate
that touches pids today. When a second crate handles pids, move it then.

No instruction file mentions retrying `EINTR`; verified rather than assumed.
