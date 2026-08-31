---
id: 019
title: ci.yml runs actions from tags anyone upstream can move
status: in_review
priority: 2
specs: []
intents: []
refs: [.github/workflows/ci.yml, .github/workflows/audit.yml]
done_when: >-
  every action in every workflow is pinned, GitHub-authored ones included, to a full commit SHA, and
  a documented way exists to update them together.
pr: https://github.com/teonimesic/plasmosome/pull/20
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

**Deliverable:** every `uses:` in every workflow names a full commit SHA, with the version it
corresponds to beside it, and a documented way to move them forward. Out of scope: changing what
any workflow does, and adding a new one.

Pin the three in `.github/workflows/ci.yml` — `actions/checkout`, `dtolnay/rust-toolchain`,
`Swatinem/rust-cache` — to the SHAs their current tags point at today. `audit.yml` is already
pinned; check it still matches. Put the human-readable version in a trailing comment on each line
(`# v5`), because a SHA alone tells a reader nothing about what they are running.

`dtolnay/rust-toolchain` needs `toolchain: stable` stated explicitly once pinned: the SHA no
longer carries what the `@stable` ref implied.

**Then answer the cost, which is the part that makes this stick.** A pinned action never updates
itself, so without a mechanism this trades a moving target for a stale one. Add
`.github/dependabot.yml` with the `github-actions` ecosystem on a weekly schedule. Say in the PR
that this is the tradeoff being accepted: bumps arrive as reviewable pull requests instead of
silently.

**Verify rather than assume.** Resolve each SHA with `gh api` and show the command output in the
PR. A SHA copied from memory or a search result is exactly the supply-chain hole this closes.

**Done when:** `done_when` holds for every workflow, both files parse, and the gate in root
`AGENTS.md` is green.

## Notes
