---
id: 015
title: The readiness probe sends a verb the control protocol does not define
status: todo
priority: 2
specs: [001]
intents: []
refs:
  [
    crates/plasmosome-membrane/src/readiness.rs,
    docs/specs/001-control-protocol.md,
  ]
done_when: >-
  the request readiness::probe sends names a verb the spec defines, or the spec
  names the verb the probe sends, and a test asserts the two agree rather than
  leaving it to a reader.
pr:
evidence:
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

## Notes
