---
id: 022
title: One plasmid granting the same capability twice materializes one object
status: todo
priority: 2
specs: []
intents: []
refs:
  [
    crates/plasmosome-backend/src/universe.rs,
    crates/plasmosome-backend/src/fake.rs,
    crates/plasmosome-testkit/src/conformance.rs,
    crates/plasmosome-testkit/tests/clauses_discriminate.rs,
    docs/decisions/006-a-removal-names-its-owner.md,
  ]
done_when: >-
  a plasmid that mounts two sources at one target, or maps one host down two routes, holds two
  objects in the universe and can detach both; and the conformance suite holds every backend to a
  revoke that takes its own plugin's object and no other holder's.
pr:
evidence:
---

## Why

Two things task 021 found and did not fix.

**A second grant of the same capability disappears.** `UniverseOp::AddMount` drops its `source`
when it forms the key, so one plasmid mounting `/secrets` and then `/code` at `/workspace` holds
one object, not two. The operating system keeps both mounts. The first detach takes the object,
the second answers `UnknownObject`, and the residue diff cannot see the mount still standing.
`SetProxyMap` does the same with `route`. The classes that discard nothing behave the same way for
two identical grants, so widening those two keys closes part of it and not all of it — what to do
about identical grants is the open question, and it is a modelling question, not a bug fix.

**The revoke-takes-its-own-object rule has no conformance clause.** Task 021 proved it over
`FakeBackend` and, through it, `CompositeBackend`. A real backend can still take another plugin's
object and pass the suite. The clause belongs beside the ones in
`crates/plasmosome-testkit/src/conformance.rs`, with a matching `Defect` in
`tests/clauses_discriminate.rs` that it discriminates against. It was left out of task 021 to stay
clear of task 012, which was rewriting both files at the time.

## Plan

## Notes
