---
id: 007
title: Publishing the crates — packagability in CI, releases by tag
status: draft
intents: [002]
---

## Behavior

Every crate meant for third parties can reach crates.io from a tagged release, and CI proves
continuously that each one still packages on its own — `cargo package` builds the crate outside
the workspace, which is the standalone-compilation check spec 004's matrix deliberately stops
short of.

**This spec is blocked and stays `draft`.** Publishing is a promise, and three decisions that
make the promise honest have not been made. They are the owner's, not a planner's:

1. **`plasmid-sdk`'s interface is deliberately undesigned.** Its own working notes forbid
   freezing the interface by accident — and publishing is exactly that freeze. The SDK is the
   crate third parties exist for; publishing the kernel crates without it serves nobody.
2. **No versioning policy exists.** What a pre-1.0 bump promises to a plasmid author has not
   been decided.
3. **Whether and when to claim the crates.io names at all** is an owner decision with a
   squatting risk on one side and a premature-commitment risk on the other.

## Design

Decided now, so the spec is buildable the day the decisions land:

- **Publish:** `plasmid-sdk`, `plasmosome-core`, `plasmosome-ledger`, `plasmosome-backend`,
  `plasmosome-membrane`. **Never publish:** `plasmosome-freeze-checks` and
  `plasmosome-testkit`, which get `publish = false`.
- Path dependencies gain `version =` fields, which `cargo package` requires.
- CI gains a `package` job: `cargo package -p <crate>` per publishable crate, in dependency
  order. This subsumes nothing in spec 004 — the matrix stays; packaging proves a stronger,
  different thing.
- Release flow: pushing a version tag runs a workflow that publishes in dependency order with
  `--locked`, using a repository-secret token, and stops at the first failure.
- Open question: one workspace-wide version or per-crate versions. Leaning workspace-wide for
  simplicity pre-1.0, but this rides the versioning-policy decision above.

## Contract

- `publish = false` on the two internal crates is permanent.
- A green `package` job promises every publishable crate builds outside the workspace.
- A tag `vX.Y.Z` is the only trigger for publishing; no workflow publishes from a branch.

## Acceptance

- The three blocking decisions are recorded in `docs/decisions/` before any of the below lands.
- The `package` CI job is green for every publishable crate.
- One tagged release has published every publishable crate to crates.io in dependency order.
- The gate is green: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo fmt --all -- --check`, `./.githooks/provenance-guard`.

## Blocked on

The three owner decisions above. Until they are made this spec stays `draft` and files no task.
