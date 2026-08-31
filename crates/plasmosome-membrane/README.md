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

## Use

```rust
use plasmosome_membrane::vmm::{VmmChild, VmmState};

let mut child = VmmChild::spawn(launcher)?;
assert_eq!(child.state(), VmmState::Running);
child.kill()?;
```

Tests: `cargo test -p plasmosome-membrane`
