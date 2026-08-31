---
id: 010
title: Plasmids anyone can write, including the agent that needs one
date: 2026-08-31
originator: Stefano
---

Plasmid authors should be able to extend the kernel without understanding much about it. Writing
one should not require knowing how a capability is enforced, what a detach has to undo, or how a
cell is supervised.

The author I have in mind is usually not a person who set out to write a plasmid. It is an agent
already working inside a cell, which realises it is missing a capability it needs and generates
the plasmid code that fills the gap. That is the case I want to be easy, because it is the one I
expect to come up most.

Between generating a plasmid and it taking effect there is a gate — approved, merged, whatever
that turns out to be. I have not decided its shape and I am not deciding it here, but the gate is
part of what I am asking for rather than a precaution attached to it. Once through it, the plasmid
can be applied directly to the cell that asked for it, and that cell has expanded its own
capabilities.

The easy path must not close the deep one. Someone technically minded who wants to dive in and
craft the perfect plasmid has to be able to, and the existence of a generated path must never be
the reason they cannot.

There is already a line in this repository saying that plasmid authors build against a stability
boundary rather than against the kernel. This is the goal that line serves. What sits underneath
it is still to be designed.

## Outcome

(filled in later)
