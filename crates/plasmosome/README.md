# plasmosome

**A composable, OS-enforced capability kernel for AI agents.**

An agent gets a **cell**: a hardware-isolated microVM that starts with nothing — no network, no
filesystem beyond its workspace, no credentials, no model access. Capabilities arrive as
**plasmids**: modules attached and detached while the agent is running. Removing one is enforced
by the operating system rather than by asking the agent nicely — the socket closes, the mount goes
away, the credential handle dies.

This crate is the name, and nothing else yet. The kernel is being built in the open at
[teonimesic/plasmosome](https://github.com/teonimesic/plasmosome), where the controller, the
per-cell supervisor, the reversibility ledger and the enforcement-backend seam live as separate
crates. When there is a single command a person runs to drive all of that, it will ship here.

## Status

Version `0.0.0`: an empty library with no API, so nothing about the interface is promised and
nothing later is constrained by it. There is no binary either, which is why
`cargo install plasmosome` refuses rather than putting a command on your PATH that does nothing.
The first release with something in it picks its own version number.

To follow the work, read the repository. To build a capability module today, the contract is
[`plasmid-sdk`](https://github.com/teonimesic/plasmosome/tree/main/crates/plasmid-sdk) and the
authoring tool is [`plasmid`](https://github.com/teonimesic/plasmosome/tree/main/crates/plasmid).
