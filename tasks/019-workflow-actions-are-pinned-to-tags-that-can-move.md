---
id: 019
title: ci.yml runs actions from tags anyone upstream can move
status: todo
priority: 2
specs: []
intents: []
refs: [.github/workflows/ci.yml, .github/workflows/audit.yml]
done_when: >-
  every third-party action in every workflow is pinned to a full commit SHA, and
  a documented way exists to update them together.
pr:
evidence:
---

## Why

`.github/workflows/ci.yml` runs `actions/checkout@v5`, `dtolnay/rust-toolchain@stable` and
`Swatinem/rust-cache@v2`. All three are mutable references: whoever controls those repositories
can move the tag, and the next run of the workflow that gates every pull request executes
different code with no commit here saying so.

`audit.yml` is pinned to full SHAs as of task 003, which makes this worse rather than better until
the rest follows. Pinning one workflow and leaving the other on tags is the appearance of
supply-chain hygiene without the substance — and the unpinned one is the more valuable target,
because it runs on every pull request rather than once a week.

The reason this was not fixed alongside `audit.yml` is scope: task 003 was explicitly told not to
open `ci.yml`, since another agent was working in parallel and the two would have collided. That
was the right call at the time and it is why this is a task rather than a line in that PR.

Pinning costs something real and the plan should say how it is paid: a SHA does not tell a reader
what version it is, and nothing updates them. Dependabot handles pinned actions and is the obvious
answer, but adopting it is a decision, not a detail.

## Plan

## Notes
