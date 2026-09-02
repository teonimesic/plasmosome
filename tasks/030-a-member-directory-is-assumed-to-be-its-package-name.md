---
id: 030
title: A member's directory name is assumed to be its package name
status: in_review
priority: 2
specs: [003]
intents: [002]
refs:
  [
    crates/plasmosome-guards/tests/workspace_guards.rs,
    crates/plasmosome-guards/AGENTS.md,
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
  `crates/plasmosome-guards/tests/workspace_guards.rs` passes, `testkit_is_dev_only` among them.
pr: 79
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

Split `workspace_members()` in `crates/plasmosome-guards/tests/workspace_guards.rs` into a
root-parameterised worker `workspace_members_in(root: &Path)` and the existing zero-argument
wrapper, which keeps the anti-vacuity assert that the production list contains `plasmosome-testkit`.
The worker keeps the line scan of the root manifest's `members = [` array exactly as it is; where it
took the last component of each member path, it now reads that member's own `Cargo.toml` and parses
`[package].name` with the `toml` crate, per
[`docs/decisions/004-a-rule-about-code-parses-code.md`](../docs/decisions/004-a-rule-about-code-parses-code.md).
A member whose manifest cannot be read or parsed, or which declares no `[package].name`, is a
panic naming that member path, never a silent skip — dropping one would leave the member count and
every name-based check reporting a workspace smaller than the one on disk. Split `cargo()` the same
way into `cargo_in(root: &Path)` plus a wrapper, so a fixture can run Cargo in its own root. Both
production call sites keep calling the zero-argument wrappers, so the real-root default is
unchanged. Add `toml = { workspace = true }` to the crate's `[dev-dependencies]`.

Three tests, on temp-directory fixture workspaces built with `tempfile`:

- `a_member_directory_that_differs_from_its_package_name_yields_the_declared_name` — a workspace
  holding `plasmid` at `crates/plasmid-placeholder` and `straight` at `crates/straight`, asserting
  the exact list `["plasmid", "straight"]`. The mismatched member reaches the resolution branch; the
  matched one proves the ordinary case is undisturbed; exact-list equality is what stops an empty or
  shortened list from passing.
- `cargo_tree_resolves_every_name_workspace_members_reports` — the same fixture, the same pinned
  list first, then `cargo tree -p <name>` in the fixture root for each name, asserting the command
  succeeds and roots its graph at that name.
- `a_member_that_declares_no_package_name_is_refused_not_skipped` — a fixture whose second member
  holds parseable TOML with no `[package]` table, pinned with `#[should_panic(expected = ...)]` to
  the refusal message.

`only_the_held_names_are_publishable_to_a_registry` and `testkit_is_dev_only` are neither broken nor
touched: every member directory in this workspace currently matches its declared package name, so
the list is the same ten names today and becomes correct rather than lucky the first time one
differs. In `crates/plasmosome-guards/AGENTS.md`, the second paragraph of "Why the publish guard
counts members instead of naming them" states a premise this task makes false, and is rewritten to
give the reason that survives: the refusal to compare names one by one is no longer about those
names being path-derived, it is that widening what a guard asserts is its own decision with its own
task.

## Notes

2026-09-02 — **Re-scoped off a crate that no longer exists.** `refs:` and `done_when:` both named
`crates/plasmosome-freeze-checks/tests/freeze_rules.rs`, which `db3cea6` (task 035, PR 64) deleted
along with the rest of that crate. `workspace_members`, `testkit_is_dev_only` and the publish guard
survived the deletion and now live in `crates/plasmosome-guards/tests/workspace_guards.rs`; the two
fields were repointed there and nothing else in `done_when:` was changed.

**Landing without task 024, and what 024 still owes.** These two were filed to land together, and
this one lands first because it is the defect with a reproduction. After it, the root manifest's
`members = [` array is still found by scanning lines and trimming quotes and commas, while each
member's own manifest is parsed with `toml`. So 024's subject is unchanged and narrowed to one
place: the array read in `workspace_members_in`, which still assumes the members are listed one per
line and is held honest only by the wrapper's assert that the list contains `plasmosome-testkit`.

**Mutation evidence.** With the worker reverted to `path.rsplit('/').next()` and everything else
left in place, `cargo test -p plasmosome-guards` reports 6 passed and 3 failed:
`a_member_directory_that_differs_from_its_package_name_yields_the_declared_name` fails
`assertion left == right failed`, left `["plasmid-placeholder", "straight"]` against the pinned
right `["plasmid", "straight"]`; `cargo_tree_resolves_every_name_workspace_members_reports` fails on
the same pinned list, which it checks before running Cargo; and
`a_member_that_declares_no_package_name_is_refused_not_skipped` reports `test did not panic as
expected`. Running the arm the second test guards directly against that fixture shows what the
reversion costs downstream: `cargo tree -p plasmid-placeholder --prefix none` exits 101 with the
message below, which is the failure `testkit_is_dev_only` would report on a real crate that had
done nothing wrong. Restoring the resolution returns all 9 to green.

```text
error: package ID specification `plasmid-placeholder` did not match any packages
```

Those counts were taken when the binary held 9 tests; it holds 11 now, because the refusal arms
are covered too. Skipping any member whose directory holds no `Cargo.toml` — the mutant
`if !root.join(path).join("Cargo.toml").exists() { continue; }` inserted in `workspace_members_in`,
which is exactly the silent drop the refusal prose names — leaves
`a_member_whose_manifest_is_missing_is_refused_not_skipped` reporting `test did not panic as
expected` and the other 10 green. The parse arm was mutated twice. Returning an empty table instead
of panicking carries the member on to the nameless arm, and
`a_member_whose_manifest_is_not_valid_toml_is_refused_not_skipped` fails with `panic did not contain
expected string`, `expected substring: "is not valid TOML"` against a panic message that begins
with the nameless arm instead — which is also the reading that the three pinned substrings name
three different arms rather than the long tail all three messages share, since they turn on
`could not be read`, `is not valid TOML` and `declares no` respectively.
Skipping any member whose manifest does not parse gives the same `test did not panic as expected`.
Every mutation was restored from a copy taken beforehand, each restore confirmed exact with `diff`,
and the binary is back to 11 green.
