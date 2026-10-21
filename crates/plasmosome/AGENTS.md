# plasmosome — working notes

## What this crate is

The crates.io name `plasmosome`, held for this project, and the package of the `plasmosome`
command. The library exports nothing. The binary is a thin `main` over `plasmosome-cli`, where
decision 009 and spec 025 §1 put the command line's logic. The `0.0.0` package already on
crates.io is the empty placeholder, with no binary, and it does not change: nothing publishes this
package now (spec 010, decision 010).

## Hard rules

- **Export nothing from the library.** Anything public here becomes a compatibility promise made
  by a crate that has not designed its interface yet. The value of publishing at `0.0.0` is that
  it promises nothing; an export spends that.
- **The binary is a thin `main`.** `src/main.rs` passes the arguments and `CARGO_PKG_VERSION` to
  `plasmosome_cli::main_with` and exits with the code it returns. Every rule of the command line
  lives in `plasmosome-cli`, where tests reach it without a process. A rule written here is one
  those tests cannot reach.
- **This package cannot be published as it stands.** It depends on `plasmosome-cli`, a path
  package with `publish = false`, and `cargo publish` refuses a dependency with no version. A
  release is the owner's decision under spec 010, and needs a plan for that dependency first.
- **The manifest's `exclude` is not decoration.** `AGENTS.md` and `CLAUDE.md` are instructions for
  whoever edits this repository. A published tarball carrying them ships guidance about a codebase
  the reader does not have.
- **`README.md` is the crate's page on the registry.** Every sentence in it has to be true for
  someone who found the crate and has never seen this repository. It keeps the published `0.0.0`
  placeholder apart from what a checkout builds, and never says the placeholder has a command.

## Testing

`cargo test -p plasmosome` runs `tests/binary.rs` against the built binary: help, the version,
`not_served`, usage errors and a closed stdout, through a real process. A live socket under a test
`HOME` checks that neither help nor an unserved command connects. The rules themselves are
tested in `plasmosome-cli`. `plasmosome-guards` checks this package's targets in
`the_plasmosome_binary_is_in_the_held_package` and its publishing metadata in
`only_the_held_names_are_publishable_to_a_registry`.
