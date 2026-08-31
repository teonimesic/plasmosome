---
id: 012
title: A capability exists only while it is needed
date: 2026-08-31
originator: Stefano
---

An agent reads untrusted text all day. Any page it fetches, any response it gets back, any file
it opens can carry instructions somebody else wrote. That is not a bug waiting for a patch; it is
what reading is. So nothing inside the boundary can be trusted to defend itself, and being careful
is no help — an agent that has been poisoned is following instructions faithfully, just not mine.

The only defence that holds is not having the capability in the first place. What was never
granted cannot be turned against me, however convincing the text asking for it.

That makes the capabilities a cell holds the thing I want under control, and their lifetime the
security property. Every minute a capability exists past the moment it was needed is exposure I
am paying for and getting nothing back.

Three things follow. A cell gets the smallest set that lets it do the work in front of it. Every
grant is obvious and explicit — I can see what a cell was given, and nothing arrives ambient,
inherited, or by accident. And a capability is revoked as soon as it stops being needed, not when
the session ends: an agent that is finished with GitHub should not still have GitHub.

Short and automatic, both. I will not remember to take things away, and the agent inside is the
last thing that should be asked to.

004 is about a revoke taking exactly what it was asked to take, and 011 about isolation the
workload never notices. This one is about whether a capability should be there at all, and for
how long.

## Outcome

(filled in later)
