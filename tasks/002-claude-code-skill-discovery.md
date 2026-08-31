---
id: 002
title: Make the skills discoverable to Claude Code
status: planned
priority: 3
specs: []
intents: []
refs: []
done_when: >-
  every skill under .agents/skills/ — planning-work, pr-review, tasks and heartbeat —
  has a committed symlink at .claude/skills/<name> (git mode 120000) pointing at
  ../../.agents/skills/<name>, a fresh clone resolves all of them, and Claude Code
  lists all of them. A skill added later without its symlink is the failure this
  guards against.
pr:
evidence:
---

## Why

The skills live in `.agents/skills/`, which is where they should stay: it is not tied to any one
tool. Claude Code only discovers skills under `.claude/skills/`, so today it finds none of them
and an agent has to be pointed at each file by hand.

Symlinks solve this without a second copy that can drift. One directory holds the files; the
other points at it.

## Plan

**Deliverable:** `.claude/skills/<name>` is a committed symlink (git mode 120000) to
`../../.agents/skills/<name>` for every skill that exists, and a test fails when one is missing.
Out of scope: changing any skill's content, and adding new skills.

Create the symlinks with `ln -s`, then confirm git stored them as links rather than copies —
`git ls-files -s .claude/skills` must show mode `120000` on every entry. A copied file would pass
a naive "the file exists" check and then drift, which is the thing this task exists to prevent.

Add a test to `plasmosome-freeze-checks` that reads the directory listing of `.agents/skills/` and
fails naming any skill without a matching symlink. Derive the list — do not hardcode four names,
or the fifth skill added later is exactly the failure `done_when` describes.

**Watch it fail first:** delete one symlink, run the test, record the output naming that skill.

**Done when:** `done_when` holds, and the gate in root `AGENTS.md` is green.

## Notes
