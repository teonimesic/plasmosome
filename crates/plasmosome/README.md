# plasmosome

**A composable, OS-enforced capability kernel for AI agents.**

An agent gets a **cell**: a hardware-isolated microVM that starts with nothing — no network, no
filesystem beyond its workspace, no credentials, no model access. Capabilities arrive as
**plasmids**: modules attached and detached while the agent is running. Removing one is enforced
by the operating system rather than by asking the agent nicely — the socket closes, the mount goes
away, the credential handle dies.

The kernel is being built in the open at
[teonimesic/plasmosome](https://github.com/teonimesic/plasmosome), where the controller, the
per-cell supervisor, the reversibility ledger and the enforcement-backend seam live as separate
crates. The `plasmosome` command, which a person or an agent runs to drive a kernel instance one
step at a time, is being built in this crate.

## Status

The version on crates.io, `0.0.0`, is an empty placeholder that holds the name: a library with no
API and no binary. Nothing about the interface is promised, and `cargo install plasmosome` refuses
rather than putting a command on your PATH that does nothing. The first release with something in
it picks its own version number.

A checkout of the repository builds a `plasmosome` binary from this crate. It serves one command:
`plasmosome status` asks a controller for its status and prints one JSON line, the controller's
answer inside an envelope, on stdout. It finds the controller's socket by `--socket PATH`, else by
the instance name in `--kernel NAME` or `PLASMOSOME_KERNEL`, at
`$HOME/.plasmosome/instances/NAME/control.uds`. With none of them, it takes the one instance under
`$HOME/.plasmosome/instances/`, and refuses when there are several. Before it connects, it checks
that the socket is a socket you own with mode `0600`, in a directory you own that no other user can
open, and that no user but you or root can replace a directory above it. Before it writes, it checks
that the daemon runs as you. A failure is one JSON line on stderr, with an exit code that says
whether the command can be repeated safely. `plasmosome --help` lists every other command as not
served, and running one exits 2 with one JSON line on stderr whose error code is `not_served`.
`plasmosome --version` prints `plasmosome 0.0.0`. A wrong command line exits 64, also with one JSON
line on stderr.

To follow the work, read the repository. Nothing for building a capability module is ready yet:
[`plasmid-sdk`](https://github.com/teonimesic/plasmosome/tree/main/crates/plasmid-sdk), the
contract a module will build against, is reserved in this repository and unimplemented, and the
authoring tool [`plasmid`](https://github.com/teonimesic/plasmosome/tree/main/crates/plasmid)
writes a module's declaration but not yet its component in this repository's source. The
`plasmid 0.0.0` on crates.io is the name-holding placeholder, whose `plasmid new` only refuses.
