# plasmosome-backend — working notes

## What this crate is

The trait between capability decisions and capability enforcement, plus the fake backend that
makes the kernel testable without a VM.

## Hard rules

- **A grant returns a ledger entry.** If a backend can create something it cannot describe how to
  undo, the seam is wrong — fix the backend, do not widen the trait.
- **A removal names its owner.** More than one plasmid can hold a key at once, so a revoke says
  whose object it is taking. It takes that plugin's object, and takes nothing when that plugin
  holds nothing there — never another plugin's. See
  [`docs/decisions/006-a-removal-names-its-owner.md`](../../docs/decisions/006-a-removal-names-its-owner.md).
- **Grants declare hot vs generation-bound.** Never let a caller assume a capability can change
  on a running cell; the backend knows and must say.
- **The fake backend is not a stub — it is a model.** Its observable behavior must match the real
  backends' contract, or every test above it is measuring fiction.
- **Wire types stay serde-serializable.** This seam crosses a process boundary.

## Conventions

- New capability classes extend the universe enumeration deliberately: residue verification can
  only see classes it knows about, so an unlisted class is an invisible leak.

## Testing

`cargo test -p plasmosome-backend`.
