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

**A cell whose ledger does not parse cleanly is quarantined, not half-recovered.** Its ledger is
not trusted as a history, the cell is not adopted, and it is reported by name. Its objects are
still found — the backend snapshot does not need the ledger — so an operator is told what exists
without the controller pretending to know how to undo it. Containment is the point of one file per
cell, and silently accepting a shorter history would give that up while keeping the file layout.

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

**Treating a torn ledger's valid prefix as authoritative.** `Ledger::open_file` skips lines it
cannot parse and returns the rest, so a half-written record silently becomes a shorter, plausible
history. Anything granted after the tear would then be owned by nothing, which is the failure this
decision exists to prevent — arrived at quietly instead of loudly.

## Consequences

Recovery becomes a reconciliation, and `Reconciler` and `Diff` already exist for it, in the shape
they were built for: **replay reconstructs the desired state — what was asked for — and the
backend snapshot supplies the observed state.** The difference between them is the work to do.
Keeping those two apart is the whole point; a recovery that let a replayed grant stand in for an
observed one would report intent as fact and miss every drift that happened while the controller
was down. That is a real unit of work and it does not exist yet.

`ledger_generation`, which spec 001 §3.3 reports, has to come from somewhere real. It is a
constructor argument today because nothing tracks it, and this decision is what makes it a
per-cell counter rather than a process-wide one.

Writing a ledger per cell means the instance directory layout has to name them, and both the
writer and the recovery path must resolve the same path from a cell's identity. That layout, the
durable home for `ledger_generation`, and the exact quarantine report are contract details this
decision does not settle — they belong in a spec, which task 018 now requires before its work can
start.

The cost is paid at startup: a controller with many cells replays many files before it can answer
anything. If that ever becomes the reason a restart is slow, the fix is a snapshot alongside the
log, not a different kind of store.
