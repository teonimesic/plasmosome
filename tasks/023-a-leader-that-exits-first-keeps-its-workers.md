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
  status ends in one of exactly two states, chosen and written down: the group is
  signalled, and the reused-pgid hazard below is prevented by a check that the
  group is still ours — not merely noted; or it is not signalled, and the reason
  says what was measured. Either way the residual hazard is answered rather than
  left to "there was nothing to recover". All three of `drop`, `state` and `kill`
  answer the same way, since each of them early-returns on the cached terminal
  state. `VmmChild`'s doc and
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

```text
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

```text
leader=60713 worker=60714 worker_pgid=60713 kill(-60713)=0 worker_died=true
```

The worker is still in the reaped leader's group, the group kill returns 0, and the worker dies.
An independent reviewer reached the same result from the other side: 433,910 sequential forks over
200s swept the pid space four times without ever handing out the reaped leader's pid, while its
immediate neighbour came round repeatedly.

What survives of the original worry is narrower, and it is a hazard in its own right rather than a
cost of doing nothing: once the last worker exits the group's lifetime ends, the pid becomes
reusable, and a process that then takes it *and* makes itself a group leader re-creates group `P`.
A signal sent then reaches a stranger. That there was nothing left to recover does not excuse it —
the harm is the signal, not the miss — so signalling this path means first establishing that the
group is still ours, and refusing when that cannot be established. Task 013's notes
raised this as an open question and asked that the POSIX guarantee be verified before anyone acted
on it. It is verified. Settling it is part of this task — including the doc and the test, which
currently assert the leak as intended behaviour.

**Where it came from.** CodeRabbit raised the drop half twice on PR #13, the `setsid` half once,
and the competing-reaper branch on PR #21 — all as "outside diff range" comments in the review
body. Those never become review threads, so both PRs merged green with nothing to answer.

**What it is blocked on.** This crosses the spec threshold, and there is no spec to name. Spec 001
reserves exactly this ground: §4's last bullet holds "broker spawn/supervision verbs" for P1 step
2, and §5 lists "the membrane's VMM/shim/broker verb set" among what the draft deliberately does
not freeze. So the guarantee this task defends traces to the root `AGENTS.md` — "nothing outlives
its owner unnoticed" — and to no spec. `specs: []` is the honest answer, not a gap to be papered
over by naming spec 001; what is missing is a supervision contract saying what a `VmmChild` owes
the processes below it, and that has to be written before this is claimed.

## Plan

## Notes

**2026-08-31, on the third finding, which is deliberately not in `done_when`.** CodeRabbit raised
on PR #13 that `spawn` calls `libc::setsid()` and discards the result. It is recorded here rather
than as a completion criterion, because it cannot be one: a criterion that no observation can fail
dilutes the two above it that can.

A child whose `setsid` fails did not create the session and group the handle assumes it leads, so
`-self.pid` stops naming a group `VmmChild` owns. How it then misfires depends on which `EPERM` it
was: a child left in its parent's group is not reached by `kill(-self.pid, ...)` at all, while a
child that was already a group leader would be reached along with whatever else shares that group.
The guarantee is broken either way.

It is not reproducible. `setsid` fails with `EPERM` when the caller is already a process group
leader, or when its pid is the group id of a group in another session, and POSIX requires `fork` to
give the child a pid matching no active process group id — which excludes both. On a conforming
implementation this call cannot fail, so no test will be watched failing against it.

The parent could not report it in any case: `spawn` returns as soon as `fork` returns, before the
child has reached `setsid` at all, so surfacing the failure through the handle would need a
handshake this type does not have. The child is where it is answerable — it can refuse to run the
launcher, which the parent then observes as an exited child through the machinery that already
exists. Whether it *should* is a contract sentence, so it belongs to the supervision spec this task
is blocked on rather than to the repair.
