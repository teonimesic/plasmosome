# plasmid — working notes

## What this crate is

The plasmid author's command line, and a client of the kernel's control socket. A binary crate
with no library target: nothing in this workspace, and nothing a third party writes, may depend
on it.

## Hard rules

- **Nothing depends on this crate.** It is an endpoint. A `[dependencies]` entry pointing here
  from any other crate means client code has grown into something a second consumer wants, and it
  belongs in a library instead.
- **Never move this binary back into `plasmid-sdk`.** Cargo gives a package one `[dependencies]`
  table, shared by its library and its binaries, so an argument parser or a socket client added
  here would become a dependency of every plasmid crate built against the SDK. The two have
  different audiences and different release cadences.
- **The package name and the binary name are the same word on purpose.** A binary target named
  after a package somebody else publishes makes `cargo install` fail for whoever installs both.
  `plasmid` is the only package that may ship a binary called `plasmid`.
- **The scaffold refuses rather than guesses.** `plasmid new` stays a named refusal until the SDK
  interface is frozen.

## Testing

`cargo test -p plasmid`. The verbs are exercised by running the built binary through
`CARGO_BIN_EXE_plasmid` and reading its exit status, because the exit status is the contract.
