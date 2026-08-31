---
id: 008
title: Route a composite handle back to the leaf that issued it
status: todo
priority: 1
specs: []
intents: []
refs:
  [
    crates/plasmosome-backend/src/composite.rs,
    crates/plasmosome-backend/src/fake.rs,
    crates/plasmosome-backend/src/backend.rs,
    crates/plasmosome-testkit/src/conformance.rs,
    crates/plasmosome-testkit/tests/composite_backend_conformance.rs,
  ]
done_when: >-
  the three tests in crates/plasmosome-testkit/tests/composite_backend_conformance.rs
  that carry #[ignore] today pass with the #[ignore] attribute removed, and no
  conformance clause in crates/plasmosome-testkit/src/conformance.rs changed.
pr:
evidence:
---

## Why

`CompositeBackend` loses the handle its leaf issued. `grant` asks the leaf for the grant, then
overwrites the returned handle with a number from the composite's own counter. `revoke` looks up
which leaf owns that composite handle, then forwards the composite handle down to the leaf — a
number the leaf never issued. The leaf answers `UnknownHandle`.

The two counters agree only while each leaf has issued exactly one grant, which is why the
existing unit tests in `composite.rs` pass: each grants once per leaf. Give any leaf a second
grant and revocation stops working. So revocation silently fails for any composite with more
than one leaf in use — capabilities stay granted while the caller is told nothing, or is told a
handle it holds does not exist. That is the bug class this project exists to prevent: nothing
outlives its owner unnoticed.

The conformance suite found it the first time it was pointed at a second implementation. Task 004
wired `CompositeBackend` into `crates/plasmosome-testkit/tests/composite_backend_conformance.rs`
and three of the five clauses failed on the spot. They are `#[ignore]`d there, naming this defect,
so they become the regression test for the fix rather than a red build.

## Plan

Written by the planner; blank while the task is `todo`. See `.agents/skills/tasks`.

## Notes

Blank until there is something to add.
