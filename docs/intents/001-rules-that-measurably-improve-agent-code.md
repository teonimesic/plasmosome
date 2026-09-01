---
id: 001
title: Instructions for agents that measurably improve the code they write
status: approved
date: 2026-08-30
originator: Stefano
outcome:
---

Almost all the code in this repository is written by agents, and the only lever on its quality is
the instructions they read: the skills in `.agents/skills/`, the rules in `AGENTS.md`, and the
per-crate notes. Those were written from intuition. Nobody has checked whether any of them
changes what an agent actually produces.

I want that checked, and I want the rules that survive the check kept and the rest dropped. There
is prior art worth reading first — https://github.com/mattpocock/skills — but the point is not to
copy a set of files in. Most of a collection like that will not apply here, and adding rules an
agent has to read but that change nothing makes every future agent slower and no better.

The check should be an experiment, not an opinion: give an agent a concrete task with a candidate
rule and without it, and compare the output. `claude -p` can run both sides. If a rule produces
no observable difference, it should not ship, however sensible it sounds. Two areas are most
likely to pay: Rust-specific guidance, and the instructions that govern reviewing and QA — the
places where a wrong habit is repeated across every change.

## Outcome

(filled in later)
