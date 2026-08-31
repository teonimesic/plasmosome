---
id: 004
title: Build the testkit crate, the conformance suite, and the seam rules
status: in_review
priority: 1
specs: [003]
intents: [002]
refs:
  [
    docs/specs/003-test-architecture.md,
    AGENTS.md,
    Cargo.toml,
    crates/plasmosome-backend/AGENTS.md,
    crates/plasmosome-backend/Cargo.toml,
    crates/plasmosome-backend/src/backend.rs,
    crates/plasmosome-backend/src/fake.rs,
    crates/plasmosome-ledger/Cargo.toml,
    crates/plasmosome-ledger/src/lib.rs,
    crates/plasmosome-core/Cargo.toml,
    crates/plasmosome-core/src/manifest.rs,
    crates/plasmosome-freeze-checks/AGENTS.md,
    crates/plasmosome-freeze-checks/Cargo.toml,
    crates/plasmosome-freeze-checks/tests/freeze_rules.rs,
    .github/workflows/ci.yml,
  ]
done_when: >-
  crates/plasmosome-testkit exists with builders, a five-clause backend
  conformance suite that FakeBackend passes, one cross-crate integration test
  asserting empty residue after LIFO replay, a mutation-tested freeze rule
  keeping testkit out of non-dev dependencies, a testkit AGENTS.md carrying the
  layer table and seam rule, and the crate's ci.yml matrix entry if that matrix
  already exists — with the gate green.
pr: 8
evidence:
---

## Why

Spec 003: every other testing spec assumes this crate exists. The conformance suite is what
makes the fake backend a model instead of a hope, and the integration tests are the first ones
to cross crates at all.

## Plan

Do not claim this task until spec 003 is `accepted`. Priority 1 because spec 005's
`attach_detach` benchmark and every future real backend depend on this crate.

**A task that adds a workspace member adds that member's `ci.yml` matrix entry in the same PR.**
Task 005's freeze rule `ci_matrix_matches_workspace_members` fails when a member is missing from
the matrix, so a new member and its matrix entry cannot land in separate PRs in either order.
Task 004 adds `plasmosome-testkit`; whichever of the two lands second carries the entry. If
`ci.yml` already has a `crate` matrix when you start, add `plasmosome-testkit` to it and change
nothing else in that file. If it does not, there is nothing to add and task 005 will carry it.

**Deliverable:** the `plasmosome-testkit` crate exactly as spec 003's Design section lays it
out — builders, conformance suite, first integration test, freeze rule, AGENTS.md.

**Out of scope:** benchmarks (spec 005), CI changes except the one matrix entry for the crate
this task adds (spec 004), any end-to-end test, any change to the kernel crates beyond adding
`plasmosome-testkit` to workspace members. If a kernel API seems to need changing to make this
buildable, stop and report; do not change it.

**Read only the files in `refs:` and this task.** Spec 003 makes every design decision: crate
layout, module names, the factory-function shape of conformance clauses, the no-mock-framework
rule. If the spec contradicts what you find in the code, stop and report.

Steps:

1. Add `crates/plasmosome-testkit` (`publish = false`) to workspace members; depend on
   plasmosome-backend, plasmosome-ledger, plasmosome-core.
2. `src/builders.rs`: builders for `PlasmidManifest`, `Grant`/`Effect` sequences,
   `DesiredState`. Only what the conformance suite and the integration test need — no
   speculative helpers.
3. `src/conformance.rs`: the five clauses from spec 003's Design, each
   `pub fn <clause_name><B: EnforcementBackend>(make: impl Fn() -> B)`, failing tests first
   against a deliberately broken local impl, then green against `FakeBackend`.
4. `tests/attach_detach_residue.rs`: the full scenario in this order —
   manifest → register → grant → ledger → detach → replay → empty-residue. Register and detach
   are not optional: spec 003 requires both before LIFO replay, and a scenario that skips them
   can pass without ever exercising the attach and detach path this test exists for.
5. The freeze rule in `plasmosome-freeze-checks` (follow the existing rules' style in
   `freeze_rules.rs`); mutation-test it by adding the violation, observing the failure,
   reverting, and recording that in the PR description.
6. `crates/plasmosome-testkit/AGENTS.md`: the layer table and the seam rule, copied from spec
   003, plus the crate's own testing command.
7. If `.github/workflows/ci.yml` already carries a `crate` matrix, add the `plasmosome-testkit`
   entry to it. Nothing else in that file changes.

| Test | Proves |
| --- | --- |
| `grant_is_replayable` (conformance) | a grant's ledger entry round-trips to a revoke |
| `revoke_unknown_handle_is_error` (conformance) | unknown handle yields `UnknownHandle`, never success |
| `drained_revoke_removes_object` (conformance) | after graceful revoke the snapshot no longer holds the object |
| `planted_residue_survives_unrelated_revoke` (conformance) | residue detection cannot be cleaned up by accident |
| `snapshot_never_invents_objects` (conformance) | snapshot contains only granted or planted objects |
| `attach_detach_residue` (integration) | core + backend + ledger together leave no residue after LIFO replay |
| freeze rule `testkit_is_dev_only` | a non-dev dependency on testkit fails the build |

**Done when:** `done_when:` above holds and the gate in the root `AGENTS.md` passes. Append to
`## Notes` anything the next agent would otherwise rediscover.

STOP when done — do not start the next piece of work.

## Notes

- `.github/workflows/ci.yml` has no `crate` matrix — it runs one `gates` job over the whole
  workspace. Step 7 had nothing to add, so task 005 carries the `plasmosome-testkit` entry when
  it builds the matrix.
- Cargo refuses the `testkit_is_dev_only` violation before the rule can see it, but only for the
  three crates the testkit depends on: `plasmosome-core` naming `plasmosome-testkit` in
  `[dependencies]` is a package cycle and `cargo test` stops there. The rule was mutation-tested
  against `plasmosome-membrane`, which the testkit does not depend on and where cargo is happy to
  build the violation.
- `PlasmidManifest` has no constructor other than `parse`/`load`, and every field is public, so
  `ManifestBuilder` fills the struct directly. That skips the grammar's validation on purpose —
  a builder that can only produce valid manifests cannot set up a test about an invalid one.
- The conformance clauses need to know which `OsObject` a `Capability` materializes, and the
  fake's mapping is private. `conformance::materialized` states that mapping independently, over
  the public `UniverseOp::object()`. Two copies is the point: the suite is a statement of the
  contract, the fake is one implementation of it.
- **What proves LIFO replay, exactly.** In `tests/attach_detach_residue.rs` the only thing that
  proves replay ran last effect first is the ordered-string assertion on `report.replayed`.
  Nothing structural depends on the order: `OsState` is a flat `BTreeSet<OsObject>` with no
  containment semantics, so the `Mount` at `/workspace` and the `UdsPath` at
  `/workspace/run/egressd.uds` are unrelated entries in different classes, and FIFO replay leaves
  exactly the same empty state. Reversing replay to FIFO fails that one assertion and nothing
  else; relaxing it to an order-blind comparison makes FIFO pass. The fake does not model path
  nesting, and a reader of this test should not assume it does.
- **The conformance suite was pointed at a second backend and found a real bug.**
  `tests/composite_backend_conformance.rs` runs the same five clauses against `CompositeBackend`
  over three fake leaves. Two pass; three fail, because `CompositeBackend::grant` overwrites the
  handle its leaf issued with the composite's own counter and `revoke` forwards that composite
  handle back down to the leaf. The three are `#[ignore]`d with that defect named. The fix is
  task 008 and is deliberately not in this PR — no clause was weakened to make them pass.
- **The suite has gaps of its own**, recorded as task 009: handle uniqueness is not a clause,
  `apply` and `apply_removal` are never called, and revoking an already-revoked handle is not a
  clause. A backend with any of those three defects is certified conformant today.
- **`testkit_is_dev_only` walks `cargo tree`, not the manifest text.** Parsing `[dependencies]`
  by exact header match missed two forms that produce a genuine non-dev dependency: a
  `[dependencies.plasmosome-testkit]` table, and a `[target.'cfg(unix)'.dependencies]` section.
  Both were confirmed to slip past the old rule and to fire the new one. The rule now matches the
  idiom `controller_crates_have_no_dependency_path_to_a_vmm_or_netstack_crate` already used.
  `declared_in` stays, because the fork/socketpair rule still uses it.
- **`ManifestBuilder::version` and `GrantSequence::plugin` were deleted.** Both had zero callers,
  which the crate's own "builders carry only what a test in this repository uses" rule forbids.
