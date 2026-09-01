---
id: 035
title: Cut the freeze checks down to the guards, and name the crate for what is left
status: done
priority: 2
specs: [013]
intents: [008]
refs:
  [
    docs/specs/013-what-earns-a-guard.md,
    crates/plasmosome-guards/AGENTS.md,
    crates/plasmosome-guards/tests/workspace_guards.rs,
    .coderabbit.yaml,
    .github/workflows/ci.yml,
  ]
done_when: >-
  `crates/plasmosome-freeze-checks` is gone and `crates/plasmosome-guards` holds
  exactly six guards — the publish allowlist, the binary-name collision, the
  testkit dev-only rule, the attribution guard, the provenance guard and skill
  discovery; no test in the workspace asserts the controller's dependency set,
  the shape of a wire type, or shared memory across the seam; `cargo test
  --workspace` is green and reports 235 tests against 261 before; each of the
  six guards is shown to fail on the violation it names, by an in-crate refusal
  test or by a live mutation recorded in the PR; and nothing in the tree claims
  the old crate name enforces anything, the dated records under
  `docs/decisions/` and `tasks/` and spec 013's own account of the change aside.
pr: 64
evidence: squash commit db3cea6 on main; plasmosome-freeze-checks is gone and plasmosome-guards holds the six guards that refuse permanent harm, each shown to fail on the violation it names
---

## Why

Five of the eight rules in the crate pinned a controller/supervisor seam nobody has built. The
owner's ruling: "Let's not build a ton of pointless ceremony over all things. No idea why there's
an entire freeze-checks for work in progress. Design is bound to evolve here." Spec 013 turns that
into a bar the surviving guards clear and the removed ones do not.

## Plan

1. Delete the five rules that assert a design: the two controller-dependency rules, the
   shared-memory scan over wire files, the serde round-trip list, and the no-executable rule on
   `plasmid-sdk`. The scanner behind the third — `src/shared_memory.rs` and its 25 tests — goes
   with it.
2. Keep the six that name a permanent or public consequence, unchanged in logic.
3. Rename the crate to `plasmosome-guards`; carry the name through the workspace manifest, CI,
   `.coderabbit.yaml`, the root `README.md`, and the crate docs that named it. Leave
   `docs/decisions/` and `tasks/` alone: they are dated records of what was true when written.
4. Rewrite the crate's `README.md` and `AGENTS.md` around the permanent-or-public bar, and correct
   every doc that claimed an enforcement which no longer exists.
5. Show each of the six guards refusing the violation it names — a test in the crate that feeds
   it the violation and asserts refusal, or a live mutation observed to fail and reverted — and
   record the evidence in the PR.

## Notes

**What went with the five rules, and what it was.** `controller_wire_state_shares_no_memory_across_the_seam`
was enforced by a `syn`-based scanner that resolved type aliases within a file, followed
alias-of-alias chains, read `thread_local!` bodies and raw borrow expressions, and refused rather
than guessed when a `mod name;` put the code it needed in another file. Its 25 tests documented its
own blind spots by name — an alias declared in a sibling module is missed because that needs name
resolution, and a construct word meaning something else is over-reported on purpose. That honesty
is rarer than the scanner. It is removed because it held one architectural preference in place at a
moment when neither `membraned` nor the controller daemon exists: if the seam moves, the scanner is
wrong, and nobody finds out until it blocks a change that is right.

`docs/decisions/004-a-rule-about-code-parses-code.md` still holds — a guard that makes a claim
about what code does must parse it rather than grep it. What is gone is its only instance, so
nothing in the crate parses Rust today, and the decision governs the next one that has to.

2026-09-01 — GitHub reports PR 64 merged at
`db3cea6a54a298b83c894422853b372b13f646b2`; the heartbeat reconciled the stale task record.
