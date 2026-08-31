---
id: 009
title: Close the three gaps a broken backend can walk through the conformance suite
status: done
priority: 2
specs: [003]
intents: []
refs:
  [
    docs/specs/003-test-architecture.md,
    crates/plasmosome-testkit/src/conformance.rs,
    crates/plasmosome-testkit/tests/fake_backend_conformance.rs,
    crates/plasmosome-testkit/tests/composite_backend_conformance.rs,
    crates/plasmosome-backend/src/backend.rs,
    crates/plasmosome-backend/src/fake.rs,
    crates/plasmosome-ledger/src/lib.rs,
  ]
done_when: >-
  each of the three gaps below has a clause in
  crates/plasmosome-testkit/src/conformance.rs, and each clause is shown failing
  against a backend carrying that defect before it is shown passing against
  FakeBackend.
pr: https://github.com/teonimesic/plasmosome/pull/10
evidence: >-
  squash commit f42c797 on main; the conformance suite carries eight clauses. The
  failing-first runs behind them were scratch and are not in that commit — task 010
  commits them as tests/clauses_discriminate.rs, which is where that claim becomes
  checkable
---

## Why

Task 004's review built backends that pass all five clauses while being broken. Three gaps let
them through.

**Handle uniqueness is not a clause.** A backend that returns the same `Handle(7)` for every
grant passes all five. Clauses 1, 3 and 4 grant one capability and revoke it before granting the
next, so no two live grants ever share a handle inside a clause, and clause 5 never revokes at
all. Two live grants sharing a handle break `plasmosome-ledger`'s replay: the first revoke consumes
the handle, the second gets `UnknownHandle`, and `replay` propagates it with `?` — so detach
aborts and every effect below the failure stays granted and pending, not just the duplicate.

**`apply` and `apply_removal` are never called.** Two of the six trait methods have no clause. A
backend that returns `Unimplemented` from both is certified conformant. `apply_removal` is the
path the ledger takes for every compensating effect and for `InverseVia::Universe`, and `replay`
propagates its error too — so such a backend passes the suite and then fails every detach that
reaches one of those effects, leaving the rest of the ledger unreverted.

**Revoking an already-revoked handle is not a clause.** Clause 2 probes only
`live.handle + 1_000_000` — a handle no grant ever issued. It never probes a handle that was
granted and then killed, which is a different code path in any backend that keeps a table. A
partially replayed ledger does exactly that: it resumes and revokes handles some earlier pass
already withdrew.

## Plan

Written after the fact, which is the wrong order and is recorded here rather than hidden: the
planner dispatched this in a message instead of writing it into the file, and
`.agents/skills/planning-work` says outright that a plan is a file, not a message. The executor
noticed the placeholder still sat here at `in_review`.

Add three clauses to `crates/plasmosome-testkit/src/conformance.rs`, each
`pub fn <name><B: EnforcementBackend>(make: impl Fn() -> B)` matching the five already there, and
call each from both `tests/fake_backend_conformance.rs` and
`tests/composite_backend_conformance.rs`.

| Clause | Proves |
| --- | --- |
| handles are distinct across live grants | a backend reusing one handle cannot pass; the ledger's replay revokes each recorded handle in turn, so a shared handle aborts detach and strands every effect below it |
| `apply` and `apply_removal` reach the universe | neither method had a clause, so a backend refusing both was conformant while the ledger's compensating path went nowhere |
| revoking a revoked handle errors, naming the caller's handle | clause 2 only probed a handle no grant issued, so nothing specified the spent-handle path a partially replayed ledger walks on every retry |

Each clause is written against a scratch backend carrying exactly its defect, watched failing, and
only then shown green against `FakeBackend` and `CompositeBackend`. A clause that fails against
either of those is a real bug: stop and report it, never weaken the clause and never fix the
backend inside this task.

## Notes

The `## Plan` above stayed empty: the plan reached the executor as a brief rather than through
this file.

**2026-08-31.** Each of the five scratch backends written to watch a clause fail was first run
against the original five clauses, and every one of them passed — the gap this task names is
reproduced, not assumed. Two of the five cover clause B alone, one refusing both apply methods
and one refusing only `apply_removal`, so each half of that clause is shown failing on its own.
Two cover clause C: one answering a second revoke out of a cache with `Ok`, and one reporting
`UnknownHandle` carrying a handle of its own rather than the caller's.

**Clause C does not cover the composite defect task 008 fixed, and an earlier draft of this file
claimed it did.** The independent review reverted that fix — `rename_handle` on the `Err` arm of
`CompositeBackend::revoke` — and all eight clauses stayed green. The reason is structural: on a
second revoke the composite finds no route and returns `UnknownHandle` naming the caller's handle
without ever reaching the leaf, so the `Err` arm the fix guards is unreachable from this clause.
Clause C catches that error *shape*, in a backend built to have it. It does not catch it in the
composite. The claim is withdrawn rather than softened.

Neither `FakeBackend` nor `CompositeBackend` needed a change; both pass all eight as they stand.
