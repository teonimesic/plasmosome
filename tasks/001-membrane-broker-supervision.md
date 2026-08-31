---
id: 001
title: The membrane supervises its cell's brokers
status: todo
priority: 2
specs: [001]
intents: []
refs: [crates/plasmosome-membrane/AGENTS.md, docs/specs/001-control-protocol.md]
done_when: >-
  membraned spawns each of a cell's brokers, answers membrane.status ready only
  once every broker answers its own control socket, reports a broker that stops
  answering rather than staying ready, and leaves no broker process behind when
  the supervisor is dropped — the last proven by a raw waitpid returning ECHILD.
pr:
evidence:
---

## Why

`plasmosome-membrane` owns the VM child today and nothing else. The brokers a cell needs have no
owner: nothing spawns them, nothing notices when one dies, and nothing reaps them. A broker that
keeps running after its cell is gone is exactly the failure this project exists to prevent.

Readiness is affected too. `membrane.status` is the one answer the controller trusts about a
cell, and today it can report ready while the brokers behind it are not serving.

`docs/specs/001-control-protocol.md` §4 already reserves broker spawn and supervision for the
next step of P1, so the shape of the work is bounded but the behavior is not yet specified.

## Plan

## Notes
