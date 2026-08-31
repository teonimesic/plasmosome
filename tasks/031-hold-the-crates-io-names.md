---
id: 031
title: Hold the crates.io names plasmosome and plasmid
status: in_progress
priority: 2
specs: [010]
intents: [013]
refs:
  [
    docs/specs/010-holding-the-crates-io-names.md,
    docs/decisions/010-claiming-the-crates-io-names.md,
    crates/plasmosome-freeze-checks/tests/freeze_rules.rs,
    crates/plasmid/Cargo.toml,
    crates/plasmid/README.md,
    Cargo.toml,
  ]
done_when: >-
  `crates/plasmosome` is a workspace member with a library target and no binary
  target; `plasmosome` and `plasmid` both sit at version `0.0.0` carrying
  `publish = ["crates-io"]`, a description, a repository, a readme and
  `exclude = ["AGENTS.md", "CLAUDE.md"]`, and neither emits cargo's
  `manifest has no ...` warning; each crate's README has a Status section naming
  `0.0.0` and contains neither `publish = false` nor `cargo install --path`; the
  freeze rule `only_the_held_names_are_publishable_to_a_registry` passes and no
  test named `no_workspace_crate_is_publishable_to_a_registry` remains; every
  mutation in spec 010's `## Acceptance` list makes that rule fail and name the
  package or entry it is about; and the gate in the root `AGENTS.md` is green.
pr:
evidence:
---

## Why

crates.io has no reservation mechanism and never releases a claimed name, and this repository is
public, so the two words this project needs can be taken by anyone who reads it. Today the
workspace forbids publishing anything at all, so nothing can be claimed until that blanket refusal
becomes a refusal with two named exceptions.

## Plan

The deliverable, in one sentence: two manifests crates.io will accept, and a freeze rule that lets
exactly those two through and nothing else.

Out of scope: publishing. Nothing here runs `cargo publish` without `--dry-run`, and no CI job or
release workflow is added — spec 007 stays `draft`. Also out of scope: keeping agent notes out of
the other seven crates' tarballs, which is filed separately.

Read `docs/specs/010-holding-the-crates-io-names.md`,
`docs/decisions/010-claiming-the-crates-io-names.md`,
`crates/plasmosome-freeze-checks/tests/freeze_rules.rs`, `crates/plasmid/` and the root
`Cargo.toml`. Do not explore beyond them.

1. Add `crates/plasmosome`: a package with `src/lib.rs` and no binary target, at version `0.0.0`,
   carrying `description`, `repository`, `readme`, workspace-inherited `license`,
   `publish = ["crates-io"]` and `exclude = ["AGENTS.md", "CLAUDE.md"]`. It gets an `AGENTS.md`, a
   `CLAUDE.md` containing `@AGENTS.md`, and a `README.md` whose Status section names `0.0.0`.
2. Add the member to the root `Cargo.toml`.
3. Move `crates/plasmid` to version `0.0.0` and give it the same publishing metadata.
4. Rewrite the Status section of `crates/plasmid/README.md`. That file becomes the crate's page on
   the registry, so it may not say the package is unpublished or offer a checkout as the way to
   get the binary.
5. Replace `no_workspace_crate_is_publishable_to_a_registry` with an allowlist rule naming exactly
   `plasmosome` and `plasmid`. An allowlisted member must carry `publish = ["crates-io"]`; every
   other member must carry `publish = false`; a member leaving `publish` unset still panics; and
   every name on the allowlist must be a real workspace member.

### Tests

| Test | What it proves |
| --- | --- |
| `only_the_held_names_are_publishable_to_a_registry` | Exactly `plasmosome` and `plasmid` may reach a registry, and only `crates-io`; every other member is refused; an unset `publish` is fatal for any member; and an allowlist entry naming no member fails |

`cargo metadata` cannot tell an unset `publish` from `publish = true` — it reports `null` for both
— so a held name carries an explicit registry list rather than an absent field, and the unset case
stays fatal for every member, on the list or off it.

Run each mutation in spec 010's `## Acceptance` list in a copy of the tree outside this
repository, and confirm the rule fails and names the package or entry it is about.

### Done

The gate in the root `AGENTS.md` — `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`,
`./.githooks/provenance-guard`, `./.githooks/attribution-guard` — is green, and every line of spec
010's `## Acceptance` has been run and observed.

STOP when done — do not start the next piece of work, and do not publish anything.

## Notes
