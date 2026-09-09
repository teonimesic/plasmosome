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
| `vmm` | The VM process lifecycle: fork, observe, kill, and reap on drop |
| `readiness` | Is a broker actually serving? Readiness is an *answered query*, never a running pid or an existing file — a process can be alive and useless |
| `brokers` | A cell's broker set, each one a `vmm::VmmChild`, asked again on every call |
| `daemon` | `membraned`: spawns the configured brokers and answers `membrane.status` on a private control socket |
| `control` | The ndjson control-protocol envelope the daemon serves |
| `exec` | Resolving and preparing broker commands for `vmm::VmmChild` to run |

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
shares one budget across the whole set: brokers are asked in turn, each given whatever time
is left, and no further probe starts once that budget is spent. This avoids giving every broker
a fresh full deadline; it is not a strict bound on elapsed time. The set cannot interrupt a probe
that overshoots its allowance, and the real probe connects with a blocking socket call before
setting read/write timeouts. A healthy answer still costs the sum of the sequential probes.

## Dropping a handle cleans up its child

`vmm::VmmChild` owns its forked child end to end — fork, non-blocking state poll, kill, and reap
on drop. Each broker in a `BrokerSet` has one `VmmChild`, so dropping the set drops those handles.
The handle must be its child's only reaper: a competing reaper can leave the child's process
group running. Cleanup also requires the handle to be dropped; `mem::forget` leaks the child.

Killing the daemon with `SIGKILL` runs no destructors, so its brokers can keep running and its
socket path remains. The `membrane.residue.snapshot` verb intended to observe that residue is
still reserved in spec 001 §4, not an implemented recovery mechanism.

The division of labour is a design rule held in review, not by a test: VMs, shims and brokers
belong here, and the controller (`plasmosome-core`) must never own them. Both halves are written
down per crate — `crates/plasmosome-membrane/AGENTS.md` and `crates/plasmosome-core/AGENTS.md` —
not in the root file.

## Not here yet

The netstack shim and the vsock bridges belong to this crate by that rule, and none of it is
built: the modules present are `brokers`, `control`, `daemon`, `exec`, `readiness` and `vmm`.
They arrive in the next P1 step.
