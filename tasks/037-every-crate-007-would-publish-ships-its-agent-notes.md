---
id: 037
title: Every crate spec 007 would publish ships its agent working notes
status: todo
priority: 3
specs: [007]
intents: [002]
refs:
  [
    docs/specs/010-holding-the-crates-io-names.md,
    crates/plasmosome-core/Cargo.toml,
  ]
done_when: >-
  `cargo package -p <crate> --list` prints neither `AGENTS.md` nor `CLAUDE.md` for any crate spec
  007 names as publishable, while each of those crates still carries both files in the repository,
  and the exclusion spec 010 already requires for the two held names is left standing.
pr:
evidence:
---

## Why

Every member of this workspace carries an `AGENTS.md` and a `CLAUDE.md`, and both are ordinary
files in the crate directory, so `cargo package` picks them up. Measured at `17ace62`, all five
crates spec 007 names as publishable — `plasmid-sdk`, `plasmosome-core`, `plasmosome-ledger`,
`plasmosome-backend`, `plasmosome-membrane` — list both files. Two members do not, and they are
the two `docs/specs/010-holding-the-crates-io-names.md` covers; that spec put every other crate
out of scope in as many words, which is the gap this task records.

What those files contain is instructions addressed to whoever is editing the crate. They would
ship to people evaluating whether to depend on it, in a tarball that stays downloadable after a
yank, so this is not a mistake a later release corrects — only one a later release stops repeating.
Nothing is wrong on the tree today, because none of these crates has ever been published. The cost
is that the fix has to be remembered at the moment of publishing, by whoever is publishing.

**The shape of the fix is open, and worth a thought rather than a paste.** Seven manifests could
each carry the same `exclude`, which works and leaves a crate added next month shipping its notes
again. Whether it is that, something the workspace states once, or a check that reads the packaged
listing, is the planner's call — the line above says only what has to be true afterwards.

## What this waits on

Spec 007 is `draft` and blocked on four items of its own, so this task is filed rather than
planned, and it is written down now so that it is not rediscovered at the moment of publishing.

That spec also says it files no task until those are settled. This one exists because
`docs/specs/010-holding-the-crates-io-names.md` promised it when it put the workspace-wide case out
of scope — a filing that waits, not a plan that starts.

Its `intents:` copies what spec 007 carries today. Task 036 asks for that value to change; if it
lands first, this file moves with it.

## Plan

## Notes
