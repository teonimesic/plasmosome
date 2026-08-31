---
id: 003
title: Weekly cargo audit workflow
status: in_review
priority: 3
specs: []
intents: []
refs: []
done_when: >-
  A workflow runs cargo audit on a weekly schedule and on any pull request that
  changes Cargo.lock, fails the job when a dependency has an open RustSec
  advisory, and has completed one green scheduled run.
pr: https://github.com/teonimesic/plasmosome/pull/17
evidence:
---

## Why

Nothing checks the dependency tree for known vulnerabilities. `Cargo.lock` is committed and
versions are pinned, so an advisory published after a dependency was added stays invisible until
somebody happens to look.

A weekly run catches it when nobody is touching the code, which is when a new advisory usually
lands. Running on `Cargo.lock` changes catches it at the moment a dependency arrives.

## Plan

**Deliverable:** `.github/workflows/audit.yml` runs `cargo audit` weekly and on any pull request
that changes `Cargo.lock`, and fails the job on an open RustSec advisory. Out of scope: changing
`ci.yml`, adding a dependency, and fixing whatever the first run reports.

A separate workflow file, not a job in `ci.yml`: this one runs on a schedule and must not gate a
normal PR, and the two have different triggers and different failure meanings.

Pin `cargo-audit` to a version rather than installing latest, so a new release cannot change what
the job does without a commit saying so.

`done_when` requires one green scheduled run, which cannot happen inside the PR — say so in the
PR body and leave the task `in_review` until a real scheduled run has passed.

**Done when:** the workflow exists, its syntax is valid, a `Cargo.lock`-touching PR triggers it,
the gate in root `AGENTS.md` is green, **and one scheduled run has completed green on `main`**.
That last condition cannot be met inside a pull request — a `schedule:` trigger only fires from
the default branch — so merging does not close this task.

## Notes

**2026-08-31.** Merged as `25a4cb8` but deliberately left `in_review`. `done_when` requires one
green scheduled run, and a `schedule:` trigger only fires from the default branch — so it could
not be met inside the pull request and cannot be met by merging. Close it when a Monday run has
passed.
