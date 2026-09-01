---
id: 013
title: Two lifecycle gaps with no witness, and a deadline that multiplies
status: done
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
  status answers within one deadline for a set of any size; the Probe seam has two real
  adapters or one fewer unused one; and either drop's no-signal-after-external-reap path gains a
  witness, or this task records that it cannot have one and why.
pr: https://github.com/teonimesic/plasmosome/pull/21
evidence: squash commit 3e180fc on main; one deadline covers a whole broker set's status answer, and drop's no-signal-after-external-reap path gained the witness it lacked
---

## Why

Three things left over from PR #13. The two process-lifecycle defects that prompted this task were
fixed there instead of deferred, so what remains is smaller and different.

**Fixed in PR #13, recorded here so the history is legible.** A broker's forked worker used to
survive the set: `VmmChild` killed the broker and nothing else. The child now calls `setsid` and
drop signals the whole process group, verified by a test that forks a worker, reports its pid up a
pipe, and fails with `worker 59230 outlived the child that forked it` when the group kill is
removed. Drop also observes terminal state before signalling now, so a child that already exited
or that something else reaped is recorded rather than signalled.

**One of those fixes has no witness, and that is the open question.** Drop's early return on
`ECHILD` cannot be tested through the public API: sending `SIGKILL` to a freed pid is a harmless
`ESRCH`, so a test asserting the pid is gone passes whether or not the signal was sent. The first
test written for it did exactly that and was replaced. What has a witness is `state()` reporting
`Lost` — mutating it to `Running` turns the test red. Distinguishing drop's behaviour needs the pid
to be reused by an unrelated process, which cannot be forced deterministically. Either find an
observation, or write down that there is none.

**The check-then-signal window cannot be closed portably.** Drop asks whether the child is gone
before signalling, but a competing `waitpid(-1)` or `SIGCHLD` handler in the same process can reap
between the check and the signal, and the freed pid may be reused before it lands. `pidfd_open`
plus `pidfd_send_signal` closes it on Linux; macOS has no equivalent, and this crate targets macOS
first. The constraint — this handle must be the only reaper of its child — is stated in
`VmmChild`'s doc. It needs enforcing when `membraned` grows a real supervision loop, because that
is exactly where a `waitpid(-1)` reaper appears.

**`status` gives each broker the full deadline**, so a set of N brokers can take N times it —
measured at 302ms for one broker and 1.85s for six — while the membrane must answer
`membrane.status` inside its own budget. Probing concurrently, or spending one budget across the
set, would fix it.

**`ControlSocket` is never constructed.** The production `Probe` has no caller anywhere, so the
seam has one adapter in use and it is a test double. By the two-adapter rule in the root
`AGENTS.md` it is not yet earned. It becomes earned when a real broker binary and a production
launcher exist.

## Plan

**Deliverable:** the three items in `## Why` are each closed or recorded as unclosable with a
reason. Out of scope: the daemon, the control protocol, and anything in `plasmosome-core`.

**1. `status` must cost one deadline per call, not one per broker.** Give the whole call a single
budget: track the time already spent and pass each probe what remains, so a set of six brokers
cannot take six times the deadline. A broker reached after the budget is exhausted is not ready,
and the report says which one ran the clock out. Test with a fake prober that consumes time.

**2. Give `ControlSocket` a caller, or delete it.** It is the production `Probe` and nothing
constructs it, so the seam has one adapter and it is a test double. Either wire it into a
constructor a daemon would use, or remove it and let the seam earn itself when a real broker
exists. Decide, say which in the task Notes, and do not leave it unused and undiscussed.

**3. The check-then-signal window.** Already documented on `VmmChild` as a constraint — this
handle must be the only reaper of its child. Add a test that a second reaper is what breaks it, if
one can be written; if it cannot be observed without forcing pid reuse, record that in the Notes
and leave the constraint documented. Do not invent a test that passes either way.

**Watch each test fail first**, against the behaviour it replaces, and record the output.

**Done when:** `done_when` holds, and the gate in root `AGENTS.md` is green.

## Notes

**2026-08-31, on the `done_when` wording.** It asked for "a production Probe has a caller". The
executor deleted `ControlSocket` instead, and flagged the tension rather than rewording the
criterion to match what it did — the right call, since quietly editing a criterion to fit the work
is how a task closes without doing what it said.

Resolved in favour of the deletion. The only caller `ControlSocket` could have is `membraned`'s
supervision loop, which this task puts out of scope, so "give it a caller" was unreachable without
inventing one. Keeping a production adapter nothing constructs leaves the seam hypothetical under
the two-adapter rule; deleting it is the honest state until a real broker exists, and
`readiness::probe` stays public so nothing is lost.

All three items are closed. Item 3 turned out to have a witness after all — the open question in
`## Why` is answered, not recorded as unanswerable.

### 1. One deadline for the set

`status` now starts a clock and hands each probe `deadline - elapsed`, so a set of any size answers
within one budget. A broker the budget never reached comes back as a new `SetStatus::DeadlineSpent`
carrying `unreached` (the broker never asked) and `asked` (the brokers that spent the budget, in
order — its last entry is the one that ran the clock out). Neither `DeadlineSpent` nor `Empty` is
ready.

Two tests, both watched failing against the old per-broker deadline:

- `one_deadline_covers_the_whole_set_however_many_brokers_it_has` — eight brokers, a probe costing
  90ms each, a 200ms budget. Failed with
  `each probe must be given what is left of the set's budget, but the deadlines handed out were
  [200ms, 200ms, 200ms, 200ms, 200ms, 200ms, 200ms, 200ms]`. The elapsed-time assertion behind it
  discriminates too: the old code took 720ms of a 200ms budget (the suite ran in 0.77s against
  0.32s now), which is the same multiplication `## Why` measured at 302ms for one broker and 1.85s
  for six.
- `a_broker_the_budget_never_reached_is_named_with_the_ones_that_spent_it` — a probe that overruns
  the deadline it is given spends the whole budget on the first broker. Failed with
  `a set with a broker that was never asked cannot be ready`, because the old code went on to ask
  all three and answered `Ready`.

The first assertion is deterministic rather than timing-dependent: it reads the deadlines the
prober was actually handed, which are all equal under the old code and strictly decreasing under
the new one.

### 2. `ControlSocket` — deleted

Deleted, not wired up. The only caller it could have is `membraned`'s supervision loop, and the
plan puts the daemon out of scope, so "give it a caller" was not reachable inside this task without
inventing one. Leaving it would have kept a production adapter that nothing constructs, which by
the two-adapter rule in the root `AGENTS.md` makes the `Probe` seam hypothetical: the only
implementor in use was the test double. `readiness::probe` stays public, so whoever writes the
supervision loop restores the adapter in five lines against a seam that has by then earned itself.

This is the second of the two branches the plan authorised. `done_when` is worded for the first
only ("a production Probe has a caller") and should be read as satisfied by the plan's alternative
rather than by the letter — flagging it here rather than editing `done_when` to match what I did.

### 3. The no-signal-after-external-reap path — witnessed

`## Why` said this path could not be observed without forcing pid reuse. It can. Signalling a freed
pid is a harmless `ESRCH`, so nothing about the *pid* discriminates — but the child's **process
group** does. `VmmChild::spawn` calls `setsid`, so a worker the child forks joins the child's group,
and drop's signal is a group kill. Give that worker a pipe to hold open, let a competing reaper take
the child's exit status, then drop the handle: the pipe staying open is proof that no group kill was
sent. `poll` on the read end answers in one call and is immune to the zombie window that makes
`kill(pid, 0)` ambiguous.

`a_second_reaper_leaves_the_childs_workers_running` is that test. Mutation watched: adding
`libc::kill(-self.pid, libc::SIGKILL)` to drop's `ECHILD` branch turns it red with
`worker 68447 was signalled after a competing reaper had already taken the child's exit status, so
drop reached a pid it no longer owns`. Both directions are witnessed — the pipe stays open while
the worker lives, and hangs up when it dies — so the test cannot pass either way.

The test asserts a leak, which is uncomfortable in a repository whose first rule is that nothing
outlives its owner unnoticed. That is the point: the cost of a second reaper is now noticed, and
`VmmChild`'s doc says it, where before the doc claimed a dropped handle leaves neither the child nor
its workers behind without qualification.

**Open question, deliberately not acted on.** The mutation that turns the test red may be the
correct behaviour. POSIX reserves a pid against reuse for as long as a process group with that pid
as its group id still exists, so `kill(-pid, SIGKILL)` after an external reap either finds our own
group or returns `ESRCH` — it cannot reach an unrelated process while the group has members. If that
holds, drop could send the group kill on the `ECHILD` path and keep refusing only the bare
`kill(pid, SIGKILL)`, recovering the workers without taking the pid-reuse hazard. It is out of this
task's plan, which says to leave the constraint documented, and the POSIX guarantee should be
verified before anyone acts on it.

### The check-then-signal race itself

Still unwitnessed, and still unclosable portably. What this task witnesses is the *ordering* case —
a reaper that wins before drop looks. The *interleaving* case, where the reap lands between drop's
check and its signal, needs the freed pid to be reused by an unrelated process inside that window,
which is not forcible on macOS. `pidfd_open` plus `pidfd_send_signal` closes it on Linux; there is
no macOS equivalent. The constraint stays stated on `VmmChild` and needs enforcing when `membraned`
grows a supervision loop, because that is where a `waitpid(-1)` reaper appears.

### Gate

`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all -- --check` and `./.githooks/provenance-guard` all green. `ps -eo pid,ppid` after
the runs shows nothing reparented to init from this workspace.
