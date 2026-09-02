# plasmosome-membrane

The per-cell supervisor. One instance per cell, running on the host, outside the cell it guards.

A cell is a hardware-isolated virtual machine. Everything crossing its boundary — the VM process
itself, the network path, the broker daemons that police egress and DNS — is a child of this
supervisor. It is the membrane: the selective barrier that decides what enters and leaves.

Its central obligation is unglamorous and absolute: **nothing it starts may outlive it
unnoticed.** An orphaned child in this architecture is a process still holding capabilities that
were supposed to be revoked, so every spawn path here is paired with a reap.

## What's inside

| Module | Responsibility |
| --- | --- |
| `vmm` | The VM process lifecycle: fork, observe, kill, and always reap |
| `readiness` | Is a broker actually serving? Readiness is an *answered query*, never a running pid or an existing file — a process can be alive and useless |
| `brokers` | A cell's broker set, each one a `vmm::VmmChild`, asked again on every call |
| `daemon` | `membraned`: spawns the configured brokers and answers `membrane.status` on a private control socket |

## Use

```rust
use plasmosome_membrane::vmm::{VmmChild, VmmState};

let mut child = VmmChild::spawn(launcher)?;
assert_eq!(child.state(), VmmState::Running);
child.kill()?;
```

Tests: `cargo test -p plasmosome-membrane`

## Readiness is an answered query

A supervisor is ready when its control socket **answers** a control-`status` request — not when
the process is alive, and not when a listener accepts. This is measured, not assumed: a
confinement-profile bug leaves a broker half-alive, its control socket dead while its data
listener still answers, and every weaker test calls that broker ready. So `readiness::probe`
treats accept-without-answer as not ready.

The broker set inherits the same rule. It answers ready only once every broker answers its own
control socket, and it asks again on every call rather than caching a past yes. One `status` call
spends a single deadline across the whole set: brokers are asked in turn, each given whatever time
is left, so one unresponsive broker cannot multiply the wait by the size of the set. That bounds
the worst case, not the happy path — a healthy answer still costs the sum of the probes, so it
grows with the number of brokers.

## Nothing it starts outlives it

`vmm::VmmChild` owns its forked child end to end — fork, non-blocking state poll, kill, and reap
on drop — so a dropped handle never leaves an orphaned hypervisor behind. A cell's brokers are one
`VmmChild` each, which is how they inherit that guarantee rather than restating it.

The division of labour is a design rule held in review and written down in `AGENTS.md`, not by a
test: VMs, shims and brokers belong here, and the controller (`plasmosome-core`) must never own
them.
