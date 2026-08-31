---
id: 023
title: A leader that exits first keeps its workers
status: todo
priority: 2
specs: []
intents: []
refs: [crates/plasmosome-membrane/src/vmm.rs, tasks/013-a-broker-may-outlive-its-set.md]
done_when: >-
  a `VmmChild` whose leader exits on its own leaves no worker running — both when
  drop is the first thing to observe the exit, and when `state()` or
  `wait_terminal` observed it earlier — with a test for each that is watched
  failing against today's code. The path where a *competing reaper* took the exit
  status is settled either way: the group is signalled and the residual hazard is
  named, or it is not and the reason is written down with what was measured. A
  child that cannot put itself in its own process group is not handed back as a
  running child; that one is checked by reading `spawn`, since `setsid` failure
  cannot be forced. `VmmChild`'s doc and
  `a_second_reaper_leaves_the_childs_workers_running` say the same thing as the
  code when this is done.
pr:
evidence:
---

## Why

`VmmChild`'s doc says a dropped handle "kills and reaps the child **and everything the child
forked**", and the root `AGENTS.md` makes that the second of the two rules the project exists
for. It does not hold when the child exits by itself.

Drop asks `waitpid(pid, WNOHANG)` before signalling. When the leader has already exited, that
call reaps it and returns, and `kill(-self.pid, SIGKILL)` is never reached — so every worker the
leader forked is still running, now reparented to init. Nothing reports it. The same hole opens
one step earlier through `state()`: it caches the terminal status, after which drop returns on
its first line and the group is never signalled at all. A supervisor that polls its child, which
is what `state()` is for, takes this path every time.

Reproduced against `main` at `faaae5a`, in a copy of `vmm.rs` outside the repo, with the existing
`ForkAWorker` launcher set to exit after forking and the worker's end of a pipe as the liveness
witness. Both fail:

```
worker 77440 outlived the handle: the leader exited on its own, drop reaped it and
             returned without a group kill
worker 77593 outlived the handle after state() observed the leader's exit
```

The existing suite does not cover this. `dropping_a_child_kills_the_workers_it_forked` keeps the
leader alive in `pause()`, so drop always reaches the group kill; `ThenTheChild::Exits` is used
only by the second-reaper test.

**A fix direction that works, verified in the same copy.** Peek at the child with
`waitid(P_PID, pid, WEXITED | WNOHANG | WNOWAIT)` — which does not consume the exit status — to
tell "already reaped by someone else" (`ECHILD`, leave alone) from "exited but still ours"
(the pid is still reserved, so the process group is still ours to signal). Signal the group
before reaping in both `drop` and `state()`. With that, all 14 existing tests and both new ones
pass. It is one workable shape, not a decision — the point here is that the defect is closable
without giving up the carve-out below.

**The third branch is the one to settle, not to preserve.** Drop also returns without signalling
when a competing reaper has already taken the exit status. `VmmChild`'s doc gives the reason — the
pid is free and may belong to someone else — and
`a_second_reaper_leaves_the_childs_workers_running` witnesses the workers surviving. CodeRabbit
asked for the opposite on PR #21, and the reason given for refusing it does not hold in the case
that matters. POSIX reserves a pid while a process group still has that pid as its group id, so
while there are workers left to recover, the group is still ours. Measured:

```
leader=60713 worker=60714 worker_pgid=60713 kill(-60713)=0 worker_died=true
```

The worker is still in the reaped leader's group, the group kill returns 0, and the worker dies.
An independent reviewer reached the same result from the other side: 433,910 sequential forks over
200s swept the pid space four times without ever handing out the reaped leader's pid, while its
immediate neighbour came round repeatedly.

What survives of the original worry is narrower: once the last worker exits the group's lifetime
ends, the pid becomes reusable, and a process that then takes it *and* makes itself a group leader
re-creates the group. In that case there is nothing left to recover anyway. Task 013's notes
raised this as an open question and asked that the POSIX guarantee be verified before anyone acted
on it. It is verified. Settling it is part of this task — including the doc and the test, which
currently assert the leak as intended behaviour.

**`setsid` is unchecked, and it is the same guarantee.** `spawn` calls `libc::setsid()` and
discards the result. If it ever fails the child stays in the parent's process group, every later
`kill(-self.pid, ...)` silently finds nothing, and `spawn` still hands back a handle that claims
to own a group. It is close to unreachable — a freshly forked child is not a group leader unless
pid reuse landed exactly on the parent's group id — but the failure is silent and the guarantee
is the one thing this type sells. It belongs here because it is the other half of the same
sentence: the group has to be established, and it has to be killed.

**Where it came from.** CodeRabbit raised the drop half twice on PR #13, the `setsid` half once,
and the competing-reaper branch on PR #21 — all as "outside diff range" comments in the review
body. Those never become review threads, so both PRs merged green with nothing to answer.

This crosses the spec threshold in `.agents/skills/tasks` — it touches the enforcement semantics
and `VmmChild`'s public contract, since `state()` gains the side effect of tearing down the group.

## Plan

## Notes
