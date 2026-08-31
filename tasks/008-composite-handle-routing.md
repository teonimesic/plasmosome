---
id: 008
title: Route a composite handle back to the leaf that issued it
status: in_review
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
pr: https://github.com/teonimesic/plasmosome/pull/9
evidence:
---

## Why

`CompositeBackend` loses the handle its leaf issued. `grant` asks the leaf for the grant, then
overwrites the returned handle with a number from the composite's own counter. `revoke` looks up
which leaf owns that composite handle, then forwards the composite handle down to the leaf — a
number the leaf never issued. The leaf answers `UnknownHandle`.

The two counters agree only while each leaf has issued exactly one grant, which is why the
existing unit tests in `composite.rs` pass: each grants once per leaf. Give any leaf a second
grant and revocation stops working.

The failure is loud but misleading, which is worse than silence. `revoke` returns
`BackendError::UnknownHandle` for a handle the caller is holding and the composite did issue,
while the capability stays granted. A caller reading that error concludes there is nothing to
revoke, and stops. That is the bug class this project exists to prevent: nothing outlives its
owner unnoticed.

The conformance suite found it the first time it was pointed at a second implementation. Task 004
wired `CompositeBackend` into `crates/plasmosome-testkit/tests/composite_backend_conformance.rs`
and three of the five clauses failed on the spot. They are `#[ignore]`d there, naming this defect,
so they become the regression test for the fix rather than a red build.

## Plan

Delete the three `#[ignore]` attributes first and watch the clauses fail, then make `routes` map a
composite handle to the leaf **and the handle that leaf issued**, and forward the leaf's handle on
revoke while reporting the composite's handle back to the caller.

## Notes

**2026-08-31.** The first regression test written for this was vacuous and nearly shipped. It
granted twice to the same leaf, so the composite counter and the leaf counter stayed in lockstep
and forwarding the wrong handle worked by coincidence — it passed against the unfixed code. The
bug only appears once a grant to a *different* leaf has advanced the composite counter past that
leaf's own. The test now grants to the filesystem leaf first. Verified failing against the
unfixed revoke and passing against the fix.

That is also why the original unit tests missed this: each granted once per leaf.

The independent review then found the other half: `revoke` translated the handle on the `Ok` arm
and left the `Err` arm alone, so a leaf's error travelled to the caller carrying the leaf's
handle — a number that could name a different live grant of theirs. `rename_handle` now rewrites
the handle-bearing variants on the way out.

One honest limit on the new tests. `a_revoked_handle_is_forgotten_and_not_reissued` locks the
double-revoke contract, but it does **not** catch a composite that never drops the route: the
leaf rejects the second revoke by itself, and `rename_handle` gives that rejection the caller's
handle regardless. With the rename in place that mutation has no effect observable through the
public API — only unbounded `routes` growth, which no test can see without exposing internals.
