# plasmosome-tools — working notes

## What this crate is

The repository's CI and maintainer tools, one module and one binary per tool, and modules the tools
share, such as `bounded`. Read `README.md` for what each tool does and how it runs.

## Hard rules

- **No product crate depends on it, and it depends on no product crate.** Another member may
  reach it only through `[dev-dependencies]`; the guard `tools_are_dev_only` in
  `plasmosome-guards` refuses a normal or build dependency, an optional one behind a feature
  included, because tooling code would then ship inside a product binary.
- **No `unsafe`.** The crate inherits the workspace's `forbid(unsafe_code)` and is not in spec
  026's register. Every system call a tool needs has a safe route, such as running `/bin/kill`.
- **No early exit.** Every target root, the library, each binary and each test file, carries
  `#![deny(clippy::exit)]`. A test that calls `std::process::exit(0)` ends its binary with status
  0 before libtest prints `test result:`, so `cargo test` alone would report success.
- **One shape per tool.** A module exposes `pub fn run(args, seams..., out, err) -> u8`. A seam is
  something the tool needs from outside, such as a command runner or a clock. The binary passes
  the real ones and the rows pass fakes; it does nothing else.
- **Bounds live in the module that reads.** Each bound is a named `pub const`, checked while
  reading through a limit of the bound plus one byte (`Read::take(LIMIT + 1)`), never by a length
  check before an unbounded read. Past the bound the tool refuses; it never truncates to a
  success.
- **A port changes no behaviour.** A tool moved from Python keeps its outputs and exit codes,
  except for the drifts plasmosome-rt2i names, and its PR shows both versions on the same inputs.
- **A fake program on `PATH` is an `sh` script**, written once per test binary and run once to
  warm it, because on macOS a freshly written executable can take seconds to start. Prefer an
  in-process fake.

## Testing

`cargo test -p plasmosome-tools`. Each tool's rows are `tests/<tool>.rs`.
