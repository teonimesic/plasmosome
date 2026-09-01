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

## Use

```rust
use plasmosome_core::manifest::PlasmidManifest;

let manifest = PlasmidManifest::from_toml(source)?;
```

Tests: `cargo test -p plasmosome-core`
