---
id: 002
title: Make the skills discoverable to Claude Code
status: todo
priority: 3
specs: []
intents: []
refs: []
done_when: >-
  .claude/skills/planning-work, .claude/skills/pr-review and .claude/skills/tasks
  are committed as symlinks (git mode 120000) pointing at ../../.agents/skills/<name>,
  a fresh clone resolves all three, and Claude Code lists all three skills.
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
