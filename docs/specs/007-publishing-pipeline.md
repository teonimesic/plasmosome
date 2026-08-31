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
- CI gains a `package` job: `cargo package -p <crate>` per publishable **leaf** crate — the ones
  that depend on no other crate in this workspace. This subsumes nothing in spec 004 — the matrix
  stays; packaging proves a stronger, different thing.
- Why leaf crates only: `cargo package` strips a dependency's local `path` and resolves the
  declared `version` from the registry, so it cannot verify a crate that depends on another crate
  in this workspace until that dependency is actually published. `plasmosome-core` and
  `plasmosome-ledger` both depend on `plasmosome-backend`, so neither can be package-verified
  until `plasmosome-backend` is on crates.io. Running the job in dependency order does not fix
  this: dependency order orders the commands, it does not stage anything for them to resolve
  against.
- Release flow: pushing a version tag runs a workflow that publishes in dependency order with
  `--locked`, using a repository-secret token, and stops at the first failure.
- Before publishing each crate the workflow queries the registry for that exact name and version
  and skips it if it is already there. A published version can never be overwritten, so a release
  that failed halfway cannot be recovered by republishing from the start. Skipping what is
  already out makes a rerun resume instead of fail. That leaves the flow re-runnable but never
  re-publishable, which is the correct direction: a rerun can finish an interrupted release, and
  can never quietly replace one that shipped.
- Open question: one workspace-wide version or per-crate versions. Leaning workspace-wide for
  simplicity pre-1.0, but this rides the versioning-policy decision above.

## Contract

- `publish = false` on the two internal crates is permanent.
- The `package` job covers publishable leaf crates only: `plasmid-sdk`, `plasmosome-backend`,
  `plasmosome-membrane`. A green run promises each of those builds outside the workspace.
  `plasmosome-core` and `plasmosome-ledger` join the job once the versions they depend on exist
  on the registry, and until then no job claims they package.
- A tag `vX.Y.Z` is the only trigger for publishing; no workflow publishes from a branch.
- The publish workflow skips any crate whose exact name and version is already on the registry,
  and never attempts to overwrite one.

## Acceptance

- The `package` CI job is green for every publishable leaf crate.
- One tagged release has published every publishable crate to crates.io in dependency order, and
  rerunning that release skips the crates already on the registry instead of failing on them.
- The gate in the root `AGENTS.md` is green.

## Blocked on

Three decision records must exist in `docs/decisions/` before this spec can be accepted, and a
fourth blocker follows from the first three. Making the decisions is the owner's work, not this
spec's, so recording them is not one of its acceptance criteria — it is the precondition for
having any:

1. **The `plasmid-sdk` interface freeze.** Whether the SDK's interface is stable enough to
   publish, given that its own working notes forbid freezing it by accident.
2. **The pre-1.0 versioning policy.** What a version bump promises a plasmid author.
3. **Claiming the crates.io names.** Whether and when, weighed against squatting risk on one
   side and premature commitment on the other.
4. **`plasmosome-backend` published, before the crates depending on it can be package-verified.**
   `plasmosome-core` and `plasmosome-ledger` reach it through a workspace path, and `cargo
   package` resolves that dependency from the registry instead of from the path. Neither crate
   can be verified until `plasmosome-backend` is actually on crates.io, which cannot happen
   before the three decisions above. Until then the `package` job covers leaf crates only, and
   says so.

Until all four are settled this spec stays `draft` and files no task.
