# plasmosome-core — working notes

## What this crate is

The controller brain: plasmid registry, desired-state reconciler, manifest grammar, credential
gatekeeper, session log. It decides capability grants. It does not enforce them.

## Hard rules

- **No virtualization dependencies, ever.** No libkrun, no netstack, no VMM crate, not even
  transitively. A controller that links a hypervisor cannot outlive a crashing cell, which is the
  whole point of splitting it out. Nothing fails the build over it: this is a rule review holds,
  and the reviewer to convince is whoever reads the diff.
- **State crossing the seam is serde data**, never shared handles. No `Arc`/`Mutex` in wire types.
  The controller and the supervisor are separate processes; anything else is a lie about the
  boundary.
- **Secrets never leave the gatekeeper.** Plasmids receive handles. If you find yourself returning
  a credential value from this crate, the design took a wrong turn.

## Conventions

- Manifest grammar is frozen. Adding a field is a compatibility decision, not a refactor.

## Testing

`cargo test -p plasmosome-core`. Manifest parsing tests are the specification of the grammar —
if you change parsing behavior, the test names should tell a reader what the rule is.
