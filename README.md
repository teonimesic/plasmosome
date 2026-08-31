# Plasmosome

**A composable, OS-enforced capability kernel for AI agents.**

An agent gets a **cell**: a hardware-isolated microVM that starts with *nothing* — no network,
no filesystem beyond its workspace, no credentials, no model access. Capabilities arrive as
**plasmids**: modules you attach and detach while the agent is running. Removing one is enforced
by the operating system, not by asking the agent nicely — the socket closes, the mount goes away,
the credential handle dies.

The name is biology's: a *plasmosome* organizes the cell; *plasmids* are the mobile modules that
confer abilities on it and can be lost again without altering the organism.

## Why

Agent sandboxes today grant capabilities for a whole session and enforce them inside the harness.
Plasmosome moves enforcement below the harness — into the VM boundary, the network topology, and
the kernel's own access controls — so it holds for *any* workload in the cell, including agent
software that has never heard of Plasmosome. And because every capability is a revocable object,
it can be granted late, revoked mid-turn, and swapped for a mock.

## Properties

- **Deny by default.** A cell begins with no capabilities. Everything is an explicit grant.
- **Hot attach / detach.** Capabilities change while the agent runs; revocation is enforced at
  the OS layer within milliseconds, not at the next restart.
- **Verified reversibility.** Detaching a plasmid returns the system to its pre-attach state, and
  the kernel *proves* it: OS state is diffed across the lifecycle and any residue is named.
- **Mockable worlds.** Any plasmid can serve a fake backend (`simulate`), record a real one
  (`capture`), or pass through — so agents can be evaluated against production-shaped worlds
  without touching production.
- **Harness-agnostic.** Enforcement does not depend on the agent cooperating.

## Status

Early. The architecture and its properties were established through a measured research program;
this repository is the product build. The kernel controller, the per-cell supervisor, the typed
reversibility ledger, and the enforcement-backend seam are landing here first.

## Architecture

| Component | Role |
| --- | --- |
| `plasmosome-core` | The controller: plasmid registry, desired-state reconciler, manifest grammar, session log, credential gatekeeper |
| `plasmosome-membrane` | The per-cell supervisor: owns the cell's VMM, network path, and broker processes — the selective barrier around a cell |
| `plasmosome-ledger` | Typed reversibility: every effect records its inverse; detach replays them, and the result is verified |
| `plasmosome-backend` | The enforcement seam: one interface, with a fake in-memory backend for tests and real OS backends behind it |
| `plasmid-sdk` | The stability boundary for plasmid authors — build against this, not against the kernel |
| `plasmosome-freeze-checks` | Architectural rules as tests: the controller may never acquire a dependency on virtualization code |
| `plasmosome-testkit` | Test support: builders, the backend conformance suite, and the cross-crate scenarios — never shipped |

## Build

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Rust stable, edition 2024. macOS (Apple Silicon) is the first target; Linux follows.

## License

MIT — see [LICENSE](LICENSE).

## Author

Written by Stefano Benatti ([@teonimesic](https://github.com/teonimesic)).
