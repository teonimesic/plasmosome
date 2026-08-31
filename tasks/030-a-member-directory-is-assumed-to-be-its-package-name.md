---
id: 030
title: A member's directory name is assumed to be its package name
status: todo
priority: 2
specs: [003]
intents: []
refs:
  [
    crates/plasmosome-freeze-checks/tests/freeze_rules.rs,
    tasks/024-the-dependency-freeze-reads-text-not-toml.md,
  ]
done_when: >-
  `workspace_members` returns each member's declared `[package].name` rather than the last
  component of its path, and a member whose directory name differs from its package name — a
  crate named `plasmid` living at `crates/plasmid-placeholder`, say — leaves the whole freeze
  suite green. A test covers that case.
pr:
evidence:
---

## Why

`workspace_members` reads the member *paths* out of the workspace manifest and takes each one's
last path component as the crate name. Cargo does not require a member's directory to be named
after its package, so the two can differ, and then the name is simply wrong.

`testkit_is_dev_only` is where that becomes a defect rather than a cosmetic slip. It feeds those
path-derived names straight to `cargo tree -p <name>`, which takes a *package* name. Give a member
a directory name that differs from its package name and the command fails to resolve, so the test
fails — reporting a dependency-layering violation that does not exist.

Confirmed against the branch of PR #45: a member crate named `plasmid` placed at
`crates/plasmid-placeholder`, with nothing else wrong with it, fails `testkit_is_dev_only`.

The consequence is that it stays invisible until somebody names a directory differently, and then
it looks like their change broke the suite rather than like a latent bug they walked into.

**Overlaps task 024, and should land in the same pass.** That task already asks for
`workspace_members` to stop parsing TOML by hand. But its `done_when` is about the *parser*, and a
fix that swaps in `toml` or `cargo metadata` for reading the `members` array while keeping
`rsplit('/')` would satisfy it and leave this defect exactly where it is. Reading the array
correctly still yields paths; resolving each path to the package it declares is the separate step,
and it is the one this task is about. Whoever takes 024 should take this with it.

**Not the shape of the rule it was found next to.** PR #45 added a publish rule that briefly
compared those same path-derived names against the packages `cargo metadata` reports, and dropped
the comparison once it turned out to be asserting an unstated directory-naming convention rather
than a coverage guarantee. That rule now counts members instead of naming them, so it is not
affected by this. This task is not an argument for enforcing the convention — it is the opposite:
the code should stop depending on it.

## Plan

## Notes
