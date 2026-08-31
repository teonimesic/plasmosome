---
id: 010
title: Holding the crates.io names plasmosome and plasmid
status: draft
intents: [013]
---

## Behavior

Two names on crates.io — `plasmosome` and `plasmid` — belong to this project, and nothing usable
is published under either. Someone who reaches for them gets version `0.0.0`: a crate that does
nothing, with a description saying so and a link back to this repository. The names are held
before anyone else takes them, and no interface is frozen by holding them.

Nothing else in this workspace can reach a registry, including a crate added tomorrow. That
property exists today as a blanket refusal — the freeze rule
`no_workspace_crate_is_publishable_to_a_registry` fails if any member allows a registry, and
panics if any member leaves `publish` unset. This spec turns the blanket refusal into a refusal
with two named exceptions, and keeps the default for everything else exactly as it was.

Publishing is not part of this. What this spec covers is the state of the repository that makes
the name claim correct and safe to make: two manifests that carry what crates.io asks for, ship
no agent working notes, and are the only two the freeze rule lets through. Running `cargo publish`
is a separate deliberate act by the owner, and no line below waits on it.

## Design

**`plasmosome` is a new workspace member at `crates/plasmosome`, library only.** It has a library
target and no binary target. `cargo install plasmosome` then refuses cleanly, because there is
nothing to install. A placeholder binary would be worse: it would install, run, and do nothing,
which looks like a broken tool rather than an unfinished one. The crate needs a `README.md` of its
own, because the `readme` field must point at a file that exists.

**`plasmid` is the crate that is already there.** `crates/plasmid` owns the name and ships the
`plasmid` binary. It moves to version `0.0.0` and drops `publish = false`. A second package
holding the name is not an option in either direction. Two packages called `plasmid` in one
workspace is an error cargo refuses outright. And a second package that held the name while
`crates/plasmid` kept shipping the `plasmid` binary is something the registry would accept without
complaint — it breaks later, on a user's machine, after the name is permanently claimed.

**Version `0.0.0` on both** says the same thing the crates say: nothing here is promised yet. The
first real release picks its own number and is not bound by this one.

**Both manifests carry what crates.io requires**: `name`, `version`, `description`, `license`
inherited from the workspace, `repository`, and `readme`. Cargo warns `manifest has no
description, license, ...` when any of those is missing, and that warning is the mechanical check
— it names every field it wants.

**Both carry `exclude = ["AGENTS.md", "CLAUDE.md"]`.** Agent working notes are instructions for
whoever edits this repository. They are not part of what a user installs, and a published crate
that carries them ships guidance about a codebase the reader does not have.

**The freeze rule becomes an allowlist, not a weakened assertion.** A constant names exactly
`plasmosome` and `plasmid`. The rule then checks three things, one per member:

- A member on the list must be publishable.
- A member off the list must be `publish = false`.
- A member that leaves `publish` unset still panics, exactly as it does today.

The default is unchanged, which is the whole reason for the shape. Relaxing the assertion instead
— allowing any package to be publishable, or checking only that most are not — would let the next
crate through as a side effect of being new.

**One check in that rule is not optional, and reads as if it were.** The rule cross-checks itself
by comparing counts: `reported.len()` against `workspace_members().len()`. It never compares
names. So an allowlist entry naming a package that no longer exists is invisible to everything the
rule does — the counts still agree, and every remaining member still passes. An existence check
that ties each allowlisted name to a real workspace member is the only thing keeping the list
honest. Without it the list outlives the crates it names, and a later crate could inherit a hold
that was never meant for it.

**The reverse direction fails too.** A name-hold that is made unpublishable is a failure, not a
quiet return to safety. Closing the carve-out is then a deliberate edit of the allowlist, which is
what removing a public name claim should be.

## Contract

- Exactly two packages in this workspace may be publishable: `plasmosome` and `plasmid`. They are
  named in the freeze rule's allowlist.
- Every other member carries `publish = false`. A member added later with `publish` unset fails
  the rule.
- Every name on the allowlist is a member of this workspace. An entry naming no member fails.
- An allowlisted package that is not publishable fails.
- `crates/plasmosome` has a library target and no binary target.
- Both manifests carry `name`, `version`, `description`, `license` (workspace-inherited),
  `repository` and `readme`. Both are at version `0.0.0`.
- Both carry `exclude = ["AGENTS.md", "CLAUDE.md"]`, and neither packaged crate contains either
  file.
- Neither manifest emits cargo's `manifest has no description, license, ...` warning.

## Acceptance

- `no_workspace_crate_is_publishable_to_a_registry` passes on a clean tree.
- In a copy of the tree with `publish = false` restored on `plasmosome`, and again on `plasmid`,
  the rule fails and names the package.
- In a copy of the tree with a new member added whose manifest leaves `publish` unset, the rule
  fails.
- In a copy of the tree whose allowlist names a package that is not a workspace member, the rule
  fails and names the entry.
- `cargo metadata` reports no binary target for `plasmosome`.
- `cargo package -p plasmosome --list` and `cargo package -p plasmid --list` each name no
  `AGENTS.md` and no `CLAUDE.md`.
- `cargo package -p plasmosome` and `cargo package -p plasmid` each emit no `manifest has no ...`
  warning.
- The gate in the root `AGENTS.md` is green.

## Out of scope

- **The release pipeline.** Spec 007 covers the CI packaging job, the tag-triggered publish
  workflow, the version policy, and publishing in dependency order. It stays `draft` and this spec
  does not unblock it. Nothing here adds a workflow or a CI job.
- **Publishing itself.** No acceptance line requires a crate to be on crates.io. This spec makes
  two manifests publishable; whether and when they are published is the owner's to do.
- **Every other crate's name.** `plasmid-sdk`, `plasmosome-core`, `plasmosome-backend`,
  `plasmosome-ledger`, `plasmosome-membrane`, `plasmosome-freeze-checks` and `plasmosome-testkit`
  stay unpublishable and unclaimed.
- **Agent notes in the other crates.** Every crate here carries `AGENTS.md` and `CLAUDE.md`, so
  any future publish of any of them would ship those files. The `exclude` above fixes it for the
  two crates being claimed and nowhere else. The workspace-wide fix is filed as its own task.
