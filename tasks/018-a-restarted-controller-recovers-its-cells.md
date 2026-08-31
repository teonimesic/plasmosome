---
id: 018
title: A restarted controller recovers the cells it left running
status: todo
priority: 1
specs: [001]
intents: []
refs:
  [
    docs/decisions/002-a-log-per-cell-not-a-database.md,
    crates/plasmosome-ledger/src/lib.rs,
    crates/plasmosome-core/src/reconciler.rs,
    crates/plasmosome-core/src/state.rs,
  ]
done_when: >-
  a controller started over an instance directory holding live cells rebuilds its
  ControllerState from their ledgers, diffs it against what the backend reports,
  and names every difference rather than trusting either side.
pr:
evidence:
---

## Why

Restart the controller today and it comes up empty while the operating system still holds every
process, socket and mount its cells were granted. Those capabilities are then owned by nothing:
no ledger in memory to replay, no handle to revoke through. A crash turns every running cell into
residue, which is the failure named in the first invariant of the root `AGENTS.md`.

`docs/decisions/002` settles the shape — one append-only ledger per cell, and recovery by
replaying them and reconciling against reality rather than trusting the replay alone. The ledger
records what was asked for; the operating system holds what is true; a recovery that reads only
the first will confidently report capabilities that were revoked out from under it.

This is the piece that makes `plasmosomed` restartable, and nothing above it can be trusted until
it exists — including `ledger_generation`, which spec 001 §3.3 reports and which is a constructor
argument today because nothing tracks it.

## Plan

## Notes
