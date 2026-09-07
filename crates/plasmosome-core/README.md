# plasmosome-core

The controller. It decides *what* a cell may do; it never touches a virtual machine itself.

A cell's capabilities are declared in a manifest, granted as **plasmids**, and revoked on demand.
This crate owns that decision-making: the registry of available plasmids, the reconciler that
drives a cell toward its declared shape, the credential gatekeeper (secrets live here, never in
the cell), and the append-only session log.

Enforcement happens elsewhere, on purpose. `plasmosome-core` builds and tests without any
virtualization dependency, because a controller that can boot a VM is a controller that dies with
one.

## What's inside

| Module | Responsibility |
| --- | --- |
| `manifest` | The plasmid declaration grammar: capabilities, scopes, credential delivery, mock mode |
| `registry` | Which plasmids exist, which are attached, and the tools they expose |
| `reconciler` | Desired state vs observed state, converging by generation |
| `gatekeeper` | Credential custody — the cell receives handles, never secrets |
| `session_log` | Append-only record of everything that happened in a cell |
| `state` | Wire types: instances, cells, genomes, mock modes |
| `daemon` | Serves the control protocol on a Unix socket; the `plasmosomed` binary |

## Use

```rust
use plasmosome_core::manifest::PlasmidManifest;

let manifest = PlasmidManifest::from_toml(source)?;
```

Tests: `cargo test -p plasmosome-core`

## Socket ownership

`plasmosomed` refuses a control-socket path that already exists, and never unlinks a path it did
not create. The alternative — clearing whatever is there and binding anyway — cannot tell a stale
file from a live daemon's socket, and taking the socket out from under a running controller leaves
it alive but unreachable. Refusing costs an operator one `rm` after a hard kill; the other way
round produces a controller that is running and cannot be talked to.

A daemon that returns removes its socket path, on every route out: a clean shutdown, an error
raised after the bind, or a panic unwinding through. `SIGKILL` is the case no destructor covers,
because a killed process runs none, so the path survives the daemon. That residue is observed
rather than prevented — the next start refuses the path and says why.

Connections are taken one at a time. The shutdown flag is read between accepts and between reads,
and both halves of a connection carry a timeout, so neither an idle client nor one that never
reads its replies can hold the daemon open past shutdown.
