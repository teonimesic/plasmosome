---
id: 015
title: The readiness probe sends a verb the control protocol does not define
status: done
priority: 2
specs: [001]
intents: [003, 004, 009, 012]
refs:
  [
    crates/plasmosome-membrane/src/readiness.rs,
    docs/specs/001-control-protocol.md,
  ]
done_when: >-
  readiness::probe sends a verb the spec defines, or the spec
  names the verb the probe sends, and a test asserts the two agree rather than
  leaving it to a reader.
pr: https://github.com/teonimesic/plasmosome/pull/16
evidence: squash commit 93113bc on main; the probe sends membrane.status, and a test reads the verb out of the spec so the two cannot drift
---

## Why

`crates/plasmosome-membrane/src/readiness.rs:38` sends:

```text
{"id":0,"method":"status","params":{}}
```

Every verb in `docs/specs/001-control-protocol.md` is namespaced — `cell.status`,
`plasmosome.status`, `plasmosome.start`, `plasmosome.stop`. A bare `status` is not among them.

Nothing is broken today, because no membrane server exists to receive it: the probe is exercised
only against test doubles that answer whatever they are asked. That is exactly why it will not be
noticed. Whoever writes `membraned`'s server side implements the spec, the probe keeps asking for
`status`, and the answer is an unknown-method error at the moment the first real broker is asked
whether it is serving.

Found while planning task 014, by reading the spec and the code side by side.

## Plan

**Deliverable:** `readiness::probe` asks for a verb `docs/specs/001-control-protocol.md` defines,
and a test fails if the two stop agreeing. Out of scope: writing the membrane's server side, and
any other verb.

**Decide which way to reconcile before changing anything.** Every verb in the spec is namespaced
(`cell.status`, `plasmosome.status`), so a bare `status` is the outlier — but check §4 for what it
names the membrane's own verb and follow the spec rather than guessing a pattern. If §4 does not
name one, that is a spec gap: stop and report it rather than inventing a verb and calling the task
done.

The test must compare the probe's request against the spec, not against a copy of itself. A test
asserting the probe sends what the probe sends is the failure mode this repo has hit four times.
Read the verb out of the spec file, or name the source of truth in one place both use.

**Watch it fail first:** revert the verb to `status`, run the test, record the output.

**Done when:** `done_when` holds, and the gate in root `AGENTS.md` is green.

## Notes
