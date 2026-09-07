---
id: 049
title: Define the typed heartbeat observe and apply contract
status: in_review
priority: 1
specs: [015]
intents: [015]
refs:
  [
    docs/intents/015-local-first-shared-work-state.md,
    docs/specs/014-local-first-work-state.md,
    docs/specs/015-typed-heartbeat-observe-and-apply.md,
    tasks/048-heartbeat-dispatch-driver.md,
    .agents/skills/heartbeat/SKILL.md,
    .agents/skills/tasks/SKILL.md,
    docs/templates/spec.md,
    docs/templates/task.md,
  ]
done_when:
  - Spec 015 is implemented as a typed library boundary for bounded heartbeat observation and guarded application, with deterministic operation/action/request ids, receipts, cancellation, lifecycle, freshness, stale and conflict refusals.
  - RED-first contract tests fail before implementation and then pass for valid observation/application, malformed status and links, every freshness refusal, version/generation/authority conflicts, duplicate actions, idempotent receipts, planner non-authority, and cancellation before and after an effect boundary.
  - The implementation delegates mutation and external effects to Spec 014 seams, makes no implicit approval/acceptance/claim/start/dispatch decision, and leaves Task 048 able to consume the typed boundary without inventing semantics.
  - Focused coverage, lint/format checks, and the repository quality gate pass; evidence records commands, elapsed time, result and any explicitly deferred integration boundary.
pr:
evidence:
---

## Why

Task 048 cannot safely implement the recurring driver because Spec 014 names heartbeat commands but
the current work-state crate exposes no typed reconciliation result, proposed action, planner
request, receipt, or cancellation seam. This task defines that smallest missing contract before a
driver is allowed to schedule or dispatch anything.

## Plan

1. Read the accepted Spec 014 contract and freeze the typed input/result/error/receipt shapes in
   the Spec 015 boundary.
2. Add RED tests for deterministic identity, explicit refusal states, guarded revalidation,
   idempotency, planner non-authority, and cancellation.
3. Implement only the library contract and injected writer/effect seams; do not add a scheduler,
   network sync, provider, host operation, VMM, or GUI behavior.
4. Run focused tests, coverage, formatting, lint and the full repository gate; record deferred
   integration work for Task 048 if the existing Spec 014 writer is not yet callable.

## Notes

2026-09-06 — Planned as the bounded prerequisite identified by Task 048. Spec 014 remains the
authority for freshness, lifecycle, writer lease, ownership, publication and receipts; this task
adds only the missing typed heartbeat boundary. No intent approval is inferred.

2026-09-06 — Implemented `plasmosome-work-state::heartbeat` as an injected library seam. Observation
is read-only and returns explicit `refused`, `idle`, or `proposed` states with deterministic
observation/action identifiers, freshness refusal codes, and checked keys. Application requires the
exact observation/action and generation, revalidates through the injected Spec 014 seam, checks
cooperative cancellation before the effect boundary, and records idempotent receipts. The recording
seam exists only for contract tests; it is not an authority implementation. RED-first tests are in
`crates/plasmosome-work-state/tests/heartbeat.rs` (5 passing). Focused tests, crate tests, clippy with
`-D warnings`, and format checks pass. The scheduler, Beads writer, lifecycle mutations, planner
authority, and post-effect cancellation adapter remain Task 048/integration work.

2026-09-06 — Lifecycle handoff: implementation commit `aaa3885` is ready for review. No pull
request exists yet, so this task remains `in_review` and must not be marked `done` until the reviewed
change is merged. Verification: `cargo test -p plasmosome-work-state --test heartbeat` (5 passed),
`cargo test -p plasmosome-work-state` (all passed), `cargo clippy -p plasmosome-work-state
--all-targets -- -D warnings` (passed), and `cargo fmt --all -- --check` (passed).

2026-09-07 — PR #85 exposed an intermittent Linux `ExecutableFileBusy` while launching the
bootstrap fixture. The test now materializes both executable scripts through a fully written and
chmod'd staging path followed by an atomic rename; it still executes the real launcher path and
asserts the exact Cargo argv. The focused test passed 30 consecutive runs, plus focused clippy and
format checks. Fix is pending PR CI rerun.
