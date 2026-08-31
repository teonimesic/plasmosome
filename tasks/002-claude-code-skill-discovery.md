---
id: 002
title: Make the skills discoverable to Claude Code
status: todo
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

## Notes
