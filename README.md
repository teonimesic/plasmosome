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
| `plasmid` | The plasmid author's command line — a client of the control socket, never something a plasmid depends on |
| `plasmid-sdk` | The stability boundary for plasmid authors — build against this, not against the kernel |
| `plasmosome-guards` | The repository's own guards as tests: what may reach a registry, what may claim a binary name, what may credit a commit |
| `plasmosome-testkit` | Test support: builders, the backend conformance suite, and the cross-crate scenarios — never shipped |

## Build

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Rust stable, edition 2024. macOS (Apple Silicon) is the first target; Linux follows.

## Development work

Tasks live entirely in shared native Beads: their descriptions, plans, acceptance, notes, status
and ownership. Use `./tools/work-state` from any linked worktree; it binds the pinned native
runtime to one store under the Git common directory, without changing your PATH. Worktrees
isolate code only. Intents and specs remain versioned Markdown.

```shell
./tools/work-state install --archive /path/to/pinned-release-archive --bd /path/to/verified-bd
./tools/work-state init
./tools/work-state ready --label planned
./tools/work-state show BEADS_ID
```

The installer verifies the current platform's archive and binary against
`tools/work-state-beads-1.1.2.toml`; Python 3.11+ is required. Do not initialize an empty queue
when you need the project's existing task history: obtain the verified backup or explicitly
pull its configured Dolt remote. There is no tracked task export to reconstruct it from.

Read [AGENTS.md](AGENTS.md) and the [heartbeat](.agents/skills/heartbeat/SKILL.md) before picking
work. The [task skill](.agents/skills/tasks/SKILL.md) covers native filing, planning and unique-actor
claims; [spec016](docs/specs/016-native-beads-task-authority.md) defines the complete API,
installation, migration and backup contract. Ordinary reads and mutations stay local.
`./tools/work-state dolt pull` and `dolt push` are explicit replication, not distributed claim
coordination across independent writer clones. No task Markdown or status-closure PR is needed.

## License

MIT — see [LICENSE](LICENSE).

## Author

Written by Stefano Benatti ([@teonimesic](https://github.com/teonimesic)).
