---
id: 018
title: A restarted controller recovers the cells it left running
status: todo
priority: 1
specs: [001, 008]
intents: []
refs:
  [
    docs/decisions/002-a-log-per-cell-not-a-database.md,
    docs/specs/008-cell-recovery-contract.md,
    crates/plasmosome-ledger/src/lib.rs,
    crates/plasmosome-core/src/reconciler.rs,
    crates/plasmosome-core/src/state.rs,
  ]
done_when: >-
  an accepted spec names the per-cell ledger path, the durable home for
  ledger_generation, and the quarantine report; a controller started over an
  instance directory holding live cells rebuilds desired state from their ledgers
  and diffs it against the backend's observed state, naming every difference; a
  cell whose ledger does not parse is quarantined and named rather than adopted
  from its valid prefix; and each of those is shown failing against a controller
  that skips it.
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

## Notes

A spec comes first. `docs/decisions/002` settles the shape and deliberately leaves three contract
details open: where a cell's ledger lives and how both the writer and recovery derive that path
from a cell's identity, where `ledger_generation` is durably kept — `Ledger::open_file` restores
only the plugin and its effects today, and `ControllerState` has no generation field at all — and
what the quarantine report actually says. None of those is a design choice with rejected
alternatives, so none belongs in the decision; they are the contract a stranger would need to
build against.

That spec is written:
[`docs/specs/008-cell-recovery-contract.md`](../docs/specs/008-cell-recovery-contract.md). It
decides all three, and it is `draft` until the owner reads it; this task may not be claimed
before it is `accepted`.

## Plan

## Notes
