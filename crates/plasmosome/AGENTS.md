# plasmosome — working notes

## What this crate is

The crates.io name `plasmosome`, held for this project. A library with no exports, no
dependencies, and no binary target. It ships so that the word belongs to the kernel described in
the root `README.md` rather than to whoever publishes it first.

## Hard rules

- **Export nothing.** Anything public here becomes a compatibility promise made by a crate that
  has not designed its interface yet. The value of publishing at `0.0.0` is that it promises
  nothing; an export spends that.
- **No binary target until there is a tool to install.** `cargo install plasmosome` refusing with
  `no packages found with binaries or examples` is the honest answer for a name with nothing
  behind it. A placeholder executable would install, run, and do nothing, which reads as a broken
  tool rather than an unfinished one. See
  [`docs/decisions/010-claiming-the-crates-io-names.md`](../../docs/decisions/010-claiming-the-crates-io-names.md).
- **The manifest's `exclude` is not decoration.** `AGENTS.md` and `CLAUDE.md` are instructions for
  whoever edits this repository. A published tarball carrying them ships guidance about a codebase
  the reader does not have.
- **`README.md` is the crate's page on the registry.** Every sentence in it has to be true for
  someone who found the crate and has never seen this repository.

## Testing

`cargo test -p plasmosome`. There is nothing here to test; what guards this crate is
`only_the_held_names_are_publishable_to_a_registry` in `plasmosome-freeze-checks`, which is where
its publishing metadata is checked.
