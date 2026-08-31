# plasmosome-ledger — working notes

## What this crate is

The typed reversibility ledger: per-plasmid stacks of effects, each carrying its inverse.
Detach replays them LIFO and the result is verified against observed system state.

## Hard rules

- **An effect without an inverse is not an effect this ledger accepts** — it is an `External`
  entry, and the type system must force the caller to say so deliberately.
- **Replay order is LIFO and that is load-bearing.** A mount created inside a directory must be
  removed before the directory. Do not make replay order configurable.
- **Replay must be idempotent and resumable.** Detach can be interrupted; running it again must
  converge, not double-undo.
- **The log is the truth, not the in-memory state.** The ledger is replayable from its written
  record — that property is what lets a controller restart and still finish a teardown.

## Conventions

- Property tests over hand-written cases where the invariant is universal (replay converges,
  order is preserved).

## Testing

`cargo test -p plasmosome-ledger`. The proptests are not decoration: they cover attach/detach
sequences no one would think to write by hand.
