---
id: 003
title: Weekly cargo audit workflow
status: todo
priority: 3
specs: []
intents: []
refs: []
done_when: >-
  A workflow runs cargo audit on a weekly schedule and on any pull request that
  changes Cargo.lock, fails the job when a dependency has an open RustSec
  advisory, and has completed one green scheduled run.
pr:
evidence:
---

## Why

Nothing checks the dependency tree for known vulnerabilities. `Cargo.lock` is committed and
versions are pinned, so an advisory published after a dependency was added stays invisible until
somebody happens to look.

A weekly run catches it when nobody is touching the code, which is when a new advisory usually
lands. Running on `Cargo.lock` changes catches it at the moment a dependency arrives.

## Plan

## Notes
