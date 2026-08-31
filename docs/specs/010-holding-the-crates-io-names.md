---
id: 010
title: Holding the crates.io names plasmosome and plasmid
status: accepted
intents: [013]
---

## Behavior

Two names on crates.io — `plasmosome` and `plasmid` — belong to this project, and nothing usable
is published under either. Both sit at version `0.0.0`, each with a description saying what it is
and a link back to this repository. `plasmosome` is an empty library. `plasmid` is the command
line that already exists, whose one verb refuses and exits 2, so it installs and then says plainly
that it is unfinished. The names are held before anyone else takes them, and no interface is
frozen by holding them.

Nothing else in this workspace can reach a registry, including a crate added tomorrow. That
property exists today as a blanket refusal — the freeze rule
`no_workspace_crate_is_publishable_to_a_registry` fails if any member allows a registry, and
panics if any member leaves `publish` unset. This spec turns the blanket refusal into a refusal
with two named exceptions, and keeps the default for everything else exactly as it was.

Publishing is not part of this. What this spec covers is the state of the repository that makes
the name claim correct and safe to make: two manifests that carry what crates.io asks for, ship
no agent working notes, describe themselves honestly, and are the only two the freeze rule lets
through. Running `cargo publish` is a separate deliberate act by the owner, and no line below
waits on it.

## Design

**`plasmosome` is a new workspace member at `crates/plasmosome`, library only.** It has a library
target and no binary target. `cargo install plasmosome` then refuses with `no packages found with
binaries or examples` and exits non-zero, because there is nothing to install. A placeholder
binary would be worse: it would install, run, and do nothing, which looks like a broken tool
rather than an unfinished one. The crate needs a `README.md` of its own, because the `readme`
field must point at a file that exists.

**`plasmid` is the crate that is already there.** `crates/plasmid` owns the name and ships the
`plasmid` binary. It moves to version `0.0.0` and becomes publishable. A second package holding
the name is not an option in either direction. Two packages called `plasmid` in one workspace is
an error cargo refuses outright. And a second package that held the name while `crates/plasmid`
kept shipping the `plasmid` binary is something the registry would accept without complaint — it
breaks later, on a user's machine, after the name is permanently claimed.

**Version `0.0.0` on both** says the same thing the crates say: nothing here is promised yet. The
first real release picks its own number and is not bound by this one.

**Both manifests carry what crates.io asks for**: `name`, `version`, `description`, `license`
inherited from the workspace, `repository`, and `readme`. Cargo warns `manifest has no
description, license or license-file` and names the fields it wants, which covers most of them —
but it never mentions `readme`, and a manifest with no `readme` at all packages without a word.
So `readme` needs a check of its own rather than riding on the warning.

**Both carry `exclude = ["AGENTS.md", "CLAUDE.md"]`.** Agent working notes are instructions for
whoever edits this repository. They are not part of what a user installs, and a published crate
that carries them ships guidance about a codebase the reader does not have.

**`crates/plasmid/README.md` stops describing the crate as unpublished.** Its Status section
currently says the package carries `publish = false` and that a checkout is the only way to get
the binary. That is what the `readme` field points crates.io at, so it becomes the crate's page —
and the sentence would be false the moment it is published. The reasoning that keeps agent notes
out of the tarball applies here too: what ships must be true for the person reading it.

**The freeze rule becomes an allowlist, not a weakened assertion.** It is renamed to
`only_the_held_names_are_publishable_to_a_registry`, because the old name would then describe the
opposite of what it checks. A constant names exactly `plasmosome` and `plasmid`. The rule checks
three things, one per member:

- A member on the list carries `publish = ["crates-io"]` — publishable, and to that registry only.
- A member off the list carries `publish = false`.
- A member that leaves `publish` unset still panics, exactly as it does today.

Both allowed states are explicit, which is what keeps the third check intact. `cargo metadata`
cannot tell an unset `publish` from `publish = true` — it reports `null` for both — so a name-hold
that simply dropped `publish = false` would be indistinguishable from a crate whose author forgot
the field. Naming the registry makes the two allowed states readable and leaves the unset default
failing for every member, on the list or off it.

Relaxing the assertion instead — allowing any package to be publishable, or checking only that
most are not — would let the next crate through as a side effect of being new.

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
  named in the freeze rule's allowlist and carry `publish = ["crates-io"]`.
- Every other member carries `publish = false`. A member added later with `publish` unset fails
  the rule.
- Every name on the allowlist is a member of this workspace. An entry naming no member fails.
- An allowlisted package that is not publishable fails.
- `crates/plasmosome` has a library target and no binary target.
- Both manifests carry `name`, `version`, `description`, `license` (workspace-inherited),
  `repository` and `readme`. Both are at version `0.0.0`.
- Both carry `exclude = ["AGENTS.md", "CLAUDE.md"]`, and neither packaged crate contains either
  file.
- Neither manifest emits cargo's `manifest has no ...` warning.
- Each crate's README has a Status section naming version `0.0.0`. Neither claims the crate is
  unpublished, and neither offers a checkout as the way to get it.

## Acceptance

- `only_the_held_names_are_publishable_to_a_registry` passes on a clean tree, and no test named
  `no_workspace_crate_is_publishable_to_a_registry` remains.
- In a copy of the tree, each of these mutations makes the rule fail and name the package or
  entry it is about. Every one of them is a state some weaker version of the rule would accept, so
  each is checked separately:
  - `publish = false` restored on `plasmosome`, and again on `plasmid`.
  - `publish` removed altogether from `plasmosome`, and again from `plasmid`.
  - `publish = ["crates-io", "some-other"]` on `plasmosome`, and again on `plasmid`.
  - a registry list given to an off-list member, `plasmosome-core`.
  - a new member added whose manifest leaves `publish` unset.
  - the allowlist naming a package that is not a workspace member.
- `cargo metadata` reports, for `plasmosome` and `plasmid` both: version `0.0.0`, a non-null
  `readme`, and a non-null `repository`. For `plasmosome` it reports a target of kind `lib` and no
  target of kind `bin`. For `plasmid` it reports a target of kind `bin` named `plasmid`.
- `cargo package -p plasmosome --list` and `cargo package -p plasmid --list` each name no
  `AGENTS.md` and no `CLAUDE.md`.
- `cargo package -p plasmosome` and `cargo package -p plasmid` each emit no `manifest has no ...`
  warning.
- `crates/plasmosome/README.md` and `crates/plasmid/README.md` each contain a Status section
  naming version `0.0.0`, and neither file contains the strings `publish = false` or
  `cargo install --path`.
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
