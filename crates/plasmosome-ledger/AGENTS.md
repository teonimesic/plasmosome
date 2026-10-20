# plasmosome-ledger — working notes

## What this crate is

The typed reversibility ledger: per-plasmid stacks of effects, each carrying its inverse.
Detach replays them LIFO and the result is verified against observed system state.

## Hard rules

- **An effect without an inverse is not an effect this ledger accepts** — it is an `External`
  entry, and the type system must force the caller to say so deliberately.
- **Replay order is LIFO, and reversing it corrupts the undo.** A mount created inside a
  directory must be removed before the directory. Do not make replay order configurable.
- **Replay must be idempotent and resumable.** Detach can be interrupted; running it again must
  converge, not double-undo.
- **The log is the truth, not the in-memory state.** The ledger is replayable from its written
  record — that property is what lets a controller restart and still finish a teardown. A closed
  ledger's pending cursor and the cell its first detach bound are the deliberate exception: they
  live only in memory, because spec 017 limits the cursor to one ledger instance (line 321) and
  keeps the cell out of single-plugin logs (lines 583-584). Durable cell identity belongs to the
  per-cell journal of spec 008.

## Conventions

- Property tests over hand-written cases where the invariant is universal (replay converges,
  order is preserved).

## Testing

`cargo test -p plasmosome-ledger`. The proptests are not decoration: they cover attach/detach
sequences no one would think to write by hand.
