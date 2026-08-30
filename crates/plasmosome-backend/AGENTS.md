# plasmosome-backend — working notes

## What this crate is

The trait between capability decisions and capability enforcement, plus the fake backend that
makes the kernel testable without a VM.

## Hard rules

- **A grant returns a ledger entry.** If a backend can create something it cannot describe how to
  undo, the seam is wrong — fix the backend, do not widen the trait.
- **Grants declare hot vs generation-bound.** Never let a caller assume a capability can change
  on a running cell; the backend knows and must say.
- **The fake backend is not a stub — it is a model.** Its observable behavior must match the real
  backends' contract, or every test above it is measuring fiction.
- **Wire types stay serde-serializable.** This seam crosses a process boundary.

## Conventions

- No inline `//` comments. `///` contract docs on public items only.
- New capability classes extend the universe enumeration deliberately: residue verification can
  only see classes it knows about, so an unlisted class is an invisible leak.

## Testing

`cargo test -p plasmosome-backend`.
