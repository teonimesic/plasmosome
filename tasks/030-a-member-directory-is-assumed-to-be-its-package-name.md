---
id: 030
title: A member's directory name is assumed to be its package name
status: todo
priority: 2
specs: [003]
intents: [002]
refs:
  [
    crates/plasmosome-freeze-checks/tests/freeze_rules.rs,
    tasks/024-the-dependency-freeze-reads-text-not-toml.md,
  ]
done_when: >-
  `workspace_members` returns each member's declared `[package].name` rather than the last
  component of its path, while the member list itself stays derived from the workspace manifest,
  so that the count `only_the_held_names_are_publishable_to_a_registry` checks against
  `cargo metadata` is still a reading taken from a second source. A test builds a member whose
  directory name differs from its package name — a crate named `plasmid` living at
  `crates/plasmid-placeholder`, say — and asserts on it that `workspace_members` yields `plasmid`
  and not `plasmid-placeholder`, that `cargo tree -p` resolves that name, and that every test in
  `crates/plasmosome-freeze-checks/tests/freeze_rules.rs` passes, `testkit_is_dev_only` among them.
pr:
evidence:
---

## Why

`workspace_members` reads the member *paths* out of the workspace manifest and takes each one's
last path component as the crate name. Cargo does not require a member's directory to be named
after its package, so the two can differ, and then the name is simply wrong.

`testkit_is_dev_only` is where that becomes a defect rather than a cosmetic slip. It feeds those
path-derived names straight to `cargo tree -p <name>`, which takes a *package* name. Give a member
a directory name that differs from its package name and the command cannot resolve it, so the test
fails at its command-success assertion — `cargo tree -p plasmid-placeholder failed: error: package
ID specification plasmid-placeholder did not match any packages` — before the dependency graph it
exists to read is inspected at all. It is a command failure, not the layering violation the rule is
there to catch, and there is nothing wrong with the crate that provoked it.

Reproduced on `086f7aa`: a member crate named `plasmid` placed at `crates/plasmid-placeholder`,
with nothing else wrong with it, fails `testkit_is_dev_only` at `freeze_rules.rs:176` while the
other seven rules in that file stay green, and moving it back restores all eight.

The consequence is that it stays invisible until somebody names a directory differently, and then
it looks like their change broke the suite rather than like a latent bug they walked into.

**Overlaps task 024, and should land in the same pass.** That task already asks for
`workspace_members` to stop parsing TOML by hand. But its `done_when` is about the *parser*, and a
fix that swaps in `toml` or `cargo metadata` for reading the `members` array while keeping
`rsplit('/')` would satisfy it and leave this defect exactly where it is. Reading the array
correctly still yields paths; resolving each path to the package it declares is the separate step,
and it is the one this task is about. Whoever takes 024 should take this with it.

The member list stays manifest-derived when they do.
`only_the_held_names_are_publishable_to_a_registry` asserts that the number of members the
manifest lists equals the number `cargo metadata` reports, so a `workspace_members` sourced from
`cargo metadata` would leave that check comparing one reading against itself, where nothing it is
meant to catch could fail it.

**Not the shape of the rule it was found next to.** PR #45 added a publish rule that briefly
compared those same path-derived names against the packages `cargo metadata` reports, and dropped
the comparison once it turned out to be asserting an unstated directory-naming convention rather
than a coverage guarantee. That rule now counts members instead of naming them, so it is not
affected by this. This task is not an argument for enforcing the convention — it is the opposite:
the code should stop depending on it.

## Plan

## Notes
