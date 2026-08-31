---
id: 013
title: Two lifecycle gaps with no witness, and a deadline that multiplies
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

**`status` gives each broker the full deadline**, so a set of N brokers can take N times it —
measured at 302ms for one broker and 1.85s for six — while the membrane must answer
`membrane.status` inside its own budget. Probing concurrently, or spending one budget across the
set, would fix it.

**`ControlSocket` is never constructed.** The production `Probe` has no caller anywhere, so the
seam has one adapter in use and it is a test double. By the two-adapter rule in the root
`AGENTS.md` it is not yet earned. It becomes earned when a real broker binary and a production
launcher exist.

## Plan

## Notes
