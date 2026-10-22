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
- **The scaffold refuses rather than guesses.** `plasmid new` writes the declaration with only the
  id, the version and an empty description, and grants no capability. It writes no component, and
  exits 2 for that refusal, until the plasmid interface is frozen.

## The envelopes

The binary writes spec 025's envelopes, `not_served`, `invalid` and `local_io`, with its own
writer in `src/envelope.rs`. It does not use `plasmosome-cli`'s writer: `plasmosome-cli` is not
published, and a normal dependency on an unpublished package stops this crate from packaging,
which spec 010 requires it to keep doing. Whether plasmid moves onto `plasmosome-cli` later is an
open question (plasmosome-4t24). Both writers escape the terminal controls spec 025 section 3
names.

## Testing

`cargo test -p plasmid`. The verbs are exercised by running the built binary through
`CARGO_BIN_EXE_plasmid` and reading its exit status, because the exit status is the contract.

One unit test, in `src/envelope.rs`, reads spec 025 section 3's list of terminal controls and
checks both writers against it for every Unicode scalar value, except the quote and the backslash
for `write_string`, which always escapes them. The binary cannot be given every character in one
argument.

The tests read the envelopes with `serde_json`, which is a dev-dependency only, through the
helpers in `tests/support/mod.rs`. They parse what `plasmid new` writes with `plasmosome-core`'s
parser, so a printed section that drifts from the refusals the parser gives fails a test.
`plasmosome-core` is a path-only dev-dependency, which `cargo package` drops. As a normal
dependency it would stop this crate from packaging, because `plasmosome-core` is not published.

The tests that write to a closed pipe live in `tests/closed_streams.rs`, a test binary with one
`#[test]`, so no other thread spawns a child while one of those pipes exists. When they shared a
binary with other tests, a closed pipe's read end was still open somewhere in 14 of 400 runs
under load, most likely in a child spawned at that moment. Each pipe is probed with a write that
must fail before a child gets it, so a leak fails the test instead of passing unseen. Keep any new
closed-pipe case inside that one `#[test]`.
