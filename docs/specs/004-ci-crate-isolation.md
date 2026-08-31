---
id: 004
title: CI proves each crate alone, the workspace together, and the MSRV
status: accepted
intents: [002]
---

## Behavior

CI answers three questions the current single job cannot: does each crate still compile and pass
its tests on its own, does the workspace still hold together, and is the declared minimum Rust
version (1.96) still true. A green run means all three, and the list of crates CI checks cannot
drift from the workspace without a test failing.

Today `ci.yml` runs one `gates` job: fmt, clippy, workspace tests, freeze rules, provenance
guard. That proves the workspace together. It does not prove a crate stands on its own — a crate
can lean on a sibling's dev-dependencies or on workspace build order and stay green for months.

Publishing is deliberately not here. The strongest per-crate check, `cargo package`, needs
version fields on path dependencies and a decision to publish at all; that is spec 007 and it is
blocked on owner decisions. This spec ships the strongest check available short of that.

## Design

### Jobs

- **`gates`** — unchanged. It remains the integration gate; when spec 003's testkit lands, its
  cross-crate tests run here automatically under `cargo test --workspace`.
- **`crate`** — a matrix job, one entry per workspace member, hardcoded in `ci.yml`. Two steps
  per entry, one command per step (the existing `ci.yml` rule: a multi-command step can pass
  while a command inside it failed):
  1. `cargo build -p <crate> --all-targets`
  2. `cargo test -p <crate>`
- **`msrv`** — `dtolnay/rust-toolchain@1.96` (the pinned toolchain action, not `@stable`), then
  one step: `cargo check --workspace --all-targets`.

### What the matrix does and does not prove

`cargo -p` inside a workspace still shares the lockfile, and features unify per invocation. With
no feature flags declared anywhere today, the matrix catches the real current risks: a crate
whose tests only compile because a sibling's dev-dependency happens to be in the graph, and a
crate that breaks alone but not in a workspace-wide build. It is not full standalone packaging —
that is `cargo package`, deferred to spec 007. This limit is stated here so nobody mistakes the
matrix for the packaging check.

One entry proves less than the rest: a green `crate (plasmosome-freeze-checks)` does not show
that crate standing on its own. Its tests shell out to `cargo` and run it against the workspace
root, so they read the whole workspace by construction and cannot do otherwise.

### Keeping the matrix honest

A hardcoded matrix rots when a crate is added or renamed. A new rule in
`plasmosome-freeze-checks` reads `Cargo.toml`'s `members` and `.github/workflows/ci.yml`, and
fails when the matrix and the member list differ in either direction. Per that crate's own
rules the check is mutation-tested: remove a matrix entry, watch it fail, restore it.

### Cost

The matrix multiplies CI minutes by roughly the member count, minus cache hits.
`Swatinem/rust-cache` is reused per matrix entry with the crate name in the cache key. Accepted:
this repository is small and correctness of the per-crate promise is worth more than the
minutes. Revisit if a run ever exceeds fifteen minutes.

## Contract

- Job names: `gates` (unchanged), `crate (<member>)` one per workspace member, `msrv`.
- A green `crate (<member>)` run promises: the crate and all its targets build via `-p`, and its
  tests pass, without any sibling named on the command line.
- A green `msrv` run promises: the workspace type-checks on Rust 1.96 exactly.
- The freeze rule name states its clause, e.g. `ci_matrix_matches_workspace_members`.

## Acceptance

- `ci.yml` has the `crate` matrix job covering every workspace member at merge time, two steps
  per entry, one command per step.
- `ci.yml` has the `msrv` job pinned to 1.96 and it is green.
- The freeze-checks rule fails when a workspace member is missing from the matrix or the matrix
  names a non-member, and the mutation test (entry removed, failure observed, entry restored)
  is recorded in the PR.
- All new jobs are green on the PR that introduces them.
- The gate in the root `AGENTS.md` is green.
