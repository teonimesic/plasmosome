---
id: 005
title: Per-crate CI matrix, MSRV job, and the matrix-drift freeze rule
status: planned
priority: 2
specs: [004]
intents: [002]
refs:
  [
    docs/specs/004-ci-crate-isolation.md,
    .github/workflows/ci.yml,
    Cargo.toml,
    crates/plasmosome-freeze-checks/AGENTS.md,
    crates/plasmosome-freeze-checks/tests/freeze_rules.rs,
  ]
done_when: >-
  ci.yml runs a per-crate build+test matrix over every workspace member and an
  MSRV check pinned to 1.96, a mutation-tested freeze rule fails when the matrix
  and the workspace member list differ in either direction, and all jobs are
  green on the PR.
pr:
evidence:
---

## Why

Spec 004: the current single CI job proves the workspace together but not any crate alone, and
nothing checks that the declared minimum Rust version is still true.

## Plan

Do not claim this task until spec 004 is `accepted`. Claim it in either order against task 004.

**A task that adds a workspace member adds that member's `ci.yml` matrix entry in the same PR.**
Task 005's freeze rule `ci_matrix_matches_workspace_members` fails when a member is missing from
the matrix, so a new member and its matrix entry cannot land in separate PRs in either order.
Task 004 adds `plasmosome-testkit`; whichever of the two lands second carries the entry. So if
`plasmosome-testkit` is already a workspace member when you start, it is one more matrix entry
here. If it is not, leave it out — task 004 adds it along with the crate.

**Deliverable:** the `crate` matrix job, the `msrv` job, and the matrix-drift freeze rule,
exactly as spec 004's Design lays them out.

**Out of scope:** `cargo package` or anything publishing-shaped (spec 007), benchmarks jobs
(spec 005), touching the existing `gates` job beyond leaving it as it is.

**Read only the files in `refs:` and this task.** The spec decides the job names, the
one-command-per-step rule, the toolchain pins, and the cache-key scheme. If the spec contradicts
what you find, stop and report.

Steps:

1. Add the `crate` matrix job: one entry per workspace member listed in `Cargo.toml` at the
   time you write it, steps
   `cargo build -p <crate> --all-targets` and `cargo test -p <crate>`, one command per step,
   `Swatinem/rust-cache` keyed by crate name.
2. Add the `msrv` job: `dtolnay/rust-toolchain@1.96`, one step `cargo check --workspace
   --all-targets`.
3. Add the freeze rule `ci_matrix_matches_workspace_members` in `plasmosome-freeze-checks`,
   comparing the matrix entries in `ci.yml` against `Cargo.toml` members, failing on a
   difference in either direction. Write it failing first (before the matrix exists, or against
   a deliberately wrong entry), then green.
4. Mutation-test the rule: remove one matrix entry, observe the failure, restore it, record the
   observation in the PR description.

| Test | Proves |
| --- | --- |
| `ci_matrix_matches_workspace_members` | a member missing from the matrix, or a matrix entry naming a non-member, fails the build |
| its mutation run (recorded in PR) | the rule can actually fail |
| green `crate (<member>)` for every member | each crate builds and tests alone via `-p` |
| green `msrv` | the workspace type-checks on Rust 1.96 exactly |

**Done when:** `done_when:` above holds and the gate in the root `AGENTS.md` passes. Append
surprises to `## Notes`.

STOP when done — do not start the next piece of work.

## Notes
