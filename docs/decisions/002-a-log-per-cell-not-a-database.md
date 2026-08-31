---
id: 002
title: Cell state is an append-only log per cell, recovered by reconciliation, not a database
date: 2026-08-31
status: accepted
---

## Context

The controller holds its view of the world in memory — `ControllerState { instances }` and
nothing else. Restart it and that view is gone, while the operating system still has the
processes, sockets and mounts the cells were granted. The question that prompted this was the
obvious one: does that mean we need a database?

Two things already exist and shape the answer. `plasmosome-ledger` writes an append-only file and
reads it back by replay. `plasmosome-core`'s session log is the same shape: ndjson events,
appended, read whole. Neither is a store you query; both are sequences you replay.

The ledger's defining property decides this. It replays in exact reverse order, and it must be
resumable from a partial replay — three grants undone, the process dies, and the next attempt has
to resume at the fourth without re-undoing the first three. Recovery is the point of the file, not
a feature of it.

## Decision

No database. Cell state is an append-only ledger, **one file per cell**, and a controller that
restarts **recovers its live cells** by replaying those ledgers and reconciling the result against
what the operating system actually shows.

One file per cell is the durability decision: a corrupt or truncated ledger costs the cell it
belongs to and no others. A single file per instance would make one bad write lose every cell the
instance owns, which is the wrong blast radius for a component whose job is to know what is still
granted.

Recovery is required, not optional. A controller that comes up and refuses to adopt the cells it
left running would be a controller that has lost the capabilities it is responsible for revoking —
the failure this project exists to prevent, arrived at by restarting.

## Rejected

**A relational database.** It would store an ordered sequence we always read whole, in reverse,
and never query. We would gain transactions we do not need and lose the property that the file
*is* the recovery record — readable with `cat` by someone debugging a leaked cell at three in the
morning. The one thing a database is for here, querying across many instances, is not a P1
problem and does not exist as a requirement.

**A single ledger per instance.** Simpler to write and simpler to lock, and it fails in the wrong
direction: one torn write and the instance cannot account for any of its cells.

**Refusing to adopt live cells on restart.** Much simpler, and briefly tempting for P1. It means a
crash converts every running cell into residue that nothing owns. Rejected on the same grounds as
the invariant in the root `AGENTS.md`.

**Trusting the ledger alone on recovery.** The ledger records what was *asked for*. The operating
system holds what is *true*. A replay that does not diff against reality would confidently report
capabilities that were revoked out from under it, or miss ones it never recorded.

## Consequences

Recovery becomes a reconciliation, and `Reconciler` and `Diff` already exist for it: replay each
cell's ledger to learn what was granted, snapshot what the OS shows, and treat the difference as
the work to do. That is a real unit of work and it does not exist yet.

`ledger_generation`, which spec 001 §3.3 reports, has to come from somewhere real. It is a
constructor argument today because nothing tracks it, and this decision is what makes it a
per-cell counter rather than a process-wide one.

Writing a ledger per cell means the instance directory layout has to name them, which is a
decision the daemon unit now inherits rather than invents.

The cost is paid at startup: a controller with many cells replays many files before it can answer
anything. If that ever becomes the reason a restart is slow, the fix is a snapshot alongside the
log, not a different kind of store.
