---
id: 012
title: A capability exists only while it is needed
status: approved
date: 2026-08-31
originator: Stefano
outcome:
---

An agent reads untrusted text all day. Any page it fetches, any response it gets back, any file
it opens can carry instructions somebody else wrote. That is not a bug waiting for a patch; it is
what reading is. So nothing inside the boundary can be trusted to defend itself, and being careful
is no help — an agent that has been poisoned is following instructions faithfully, just not mine.

The only defence that holds is not having the capability in the first place. What was never
granted cannot be turned against me, however convincing the text asking for it.

That makes the capabilities a cell holds the thing I want under control, and their lifetime the
security property. Every minute a capability exists past the moment it was needed is exposure
bought for nothing.

Three things follow. A cell is not given access to anything that can do harm, and if it does not
need a thing it does not get it. Every grant is obvious and explicit — I can see what a cell was
given, and nothing arrives ambient, inherited, or by accident. And a capability is revoked as
soon as it stops being needed, not when the session ends: an agent that is finished with GitHub
should not still have GitHub.

Short, and automatic. The agent inside is the last thing that should be relied on to give a
capability up.

004 is about a revoke taking exactly what it was asked to take, and 011 about isolation the model
and the harness never have to know about. This one is about whether a capability should be there
at all, and for how long.

## Outcome

(filled in later)
