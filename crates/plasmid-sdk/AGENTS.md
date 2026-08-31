# plasmid-sdk — working notes

## What this crate is

The published contract plasmid authors build against. Reserved; the interface is not designed yet.

## Hard rules

- **Do not freeze the interface by accident.** Anything exported here becomes a compatibility
  promise to third-party crates. Until the design lands, exports stay minimal and the scaffold
  refuses to generate.
- **The kernel must never depend on a specific plasmid.** Discovery is by manifest at runtime.
  If kernel code names a plasmid, the layering broke.
- **No binary target in this package.** Cargo gives a package one `[dependencies]` table,
  shared by its library and its binaries, so anything a command line wants — an argument
  parser, a client for the control socket — would become a dependency of every plasmid crate
  built against this one. The `plasmid` binary is its own crate for that reason.
- **A plasmid's capabilities come from its manifest, not from its code.** Nothing here should let
  a plasmid acquire authority by importing something.

## Open design questions

How plasmids are declared, what the minimal attachable contract is, and what an authoring
workflow looks like end to end. These are being decided deliberately, not inferred from the
first implementation.

## Testing

`cargo test -p plasmid-sdk`.
