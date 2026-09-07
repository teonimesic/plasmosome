---
id: 050
title: Refresh the installed Markdown shadow after new commits
status: todo
priority: 1
specs: [014]
intents: [015]
refs:
  [
    docs/specs/014-local-first-work-state.md,
    docs/intents/015-local-first-shared-work-state.md,
    crates/plasmosome-work-state/AGENTS.md,
    crates/plasmosome-work-state/README.md,
    crates/plasmosome-work-state/src/store.rs,
    crates/plasmosome-work-state/src/document.rs,
    crates/plasmosome-work-state/src/read.rs,
    crates/plasmosome-work-state/tests/store.rs,
    crates/plasmosome-work-state/tests/read.rs,
    crates/plasmosome-work-state/tests/cli.rs,
    tools/work-state,
  ]
done_when: >-
  An explicit supported operation refreshes an installed markdown-shadow generation from a later
  committed Markdown snapshot; list, show, ready and blocked in every linked worktree report that
  snapshot and its source commit, including added records and changed task states. Repeating the
  operation is unchanged. Invalid input or interruption leaves the previous complete generation
  readable. Ordinary reads remain local-only and never refresh implicitly, and ledger authority
  cannot be overwritten from Markdown. The gate is green.
pr:
evidence:
---

## Why

The installed task queue remains at its first imported Git commit while tasks on main change.
Spec 014 requires later one-way imports while Markdown is authoritative, but bootstrap refuses a
changed source commit. Without an explicit refresh, a successful local read can keep returning
finished work as open, and newly filed tasks never appear.

## Plan

## Notes

**2026-09-07.** Bootstrap from main at `c5aa18db2f9fa62065ac181e78962712420e8140`
installed 14 intents, 13 specs and 43 tasks. All four read commands passed, including a read from
a linked worktree, and repeating bootstrap returned `unchanged`. Task 003 was then closed on a
separate committed branch after its scheduled audit passed. The linked-worktree read still returned
its imported `in_review` state and the original source commit, rather than the checkout's edit.
That isolation is correct for ordinary reads; the missing operation is an explicit source refresh.

`bootstrap` in `store.rs` refuses a source commit that differs from the installed manifest with
`source_commit_mismatch`. Task 047's online remote-shadow observation is a separate operation:
this task concerns importing changed authoritative Markdown, not enabling remote publication,
leases, lifecycle writes, heartbeat dispatch, or cutover. The planner must preserve those boundaries
and the existing immutable-generation activation protocol rather than delete the installed store
as a refresh mechanism.
