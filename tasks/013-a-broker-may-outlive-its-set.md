---
id: 013
title: A broker's own children outlive the set, and the stale-pid guard is dead code here
status: todo
priority: 2
specs: []
intents: []
refs:
  [
    crates/plasmosome-membrane/src/vmm.rs,
    crates/plasmosome-membrane/src/brokers.rs,
    crates/plasmosome-membrane/AGENTS.md,
  ]
done_when: >-
  a broker's forked worker does not survive the set that owned its parent,
  proven by observing the grandchild's pid rather than by asserting a code path
  ran; and a BrokerSet that has observed a broker's terminal state does not
  signal that pid again.
pr:
evidence:
---

## Why

Two findings from the review of PR #13. Neither blocks that PR; both bite the moment a real
broker binary exists.

**A broker's own children survive the set.** `VmmChild` kills and reaps the broker, and nothing
else. Measured:

```text
GRANDCHILD broker_pid=49892 broker_alive=false grandchild_pid=49893 grandchild_alive=true
49893  1  S  .../plasmosome_membrane-...
```

The broker dies; its worker is reparented to init and keeps running. `VmmChild` neither calls
`setsid` nor kills the process group. Real brokers — egressd, dnsd — are exactly the kind of
daemon that forks workers, so this is a capability outliving its owner, which the first invariant
of the root `AGENTS.md` names as the bug class this project exists to prevent.

**The stale-pid guard cannot fire under `BrokerSet`.** `crates/plasmosome-membrane/AGENTS.md`
carries a hard rule: never signal a pid after its terminal state was observed. `VmmChild::drop`
guards on `self.terminal.is_none()`, but `BrokerSet` never calls `state()` or `wait_terminal()`,
so `terminal` is always `None` and every broker drop takes the kill path unconditionally. Measured
on a broker already reaped by something else, immediately before the drop:

```text
EXTERNALLY-REAPED pid 49400: kill(pid,0) => -1 errno Some(3)   (ESRCH)
```

The pid is already free when the signal goes out. `VmmState::Lost` exists because the codebase
anticipated this. A supervisor with a `SIGCHLD` handler or a `waitpid(-1)` reaper hits it every
time. Pid reuse needs a wrap-around that could not be forced deterministically, which is why this
is filed rather than treated as urgent — but it is a hard rule with no enforcement today.

**Also open, smaller.** `status` gives each broker the full `deadline`, so a set of N brokers can
take N times it (measured: 302ms for one, 1.85s for six) while the membrane must answer
`membrane.status` within its own budget. And `ControlSocket`, the production `Probe`, is never
constructed anywhere — `Probe` has one adapter in use and it is a test double, so by the
two-adapter rule the seam is not yet earned. Both resolve when a real broker binary and a
production launcher exist.

## Plan

## Notes
