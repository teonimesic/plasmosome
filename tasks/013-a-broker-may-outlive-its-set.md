---
id: 013
title: Two lifecycle gaps with no witness, and a deadline that multiplies
status: in_progress
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
  status answers within one deadline for a set of any size; a production Probe
  has a caller; and either drop's no-signal-after-external-reap path gains a
  witness, or this task records that it cannot have one and why.
pr:
evidence:
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

**1. `status` costs one deadline per broker, not one per call.** Give the whole call a single
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
