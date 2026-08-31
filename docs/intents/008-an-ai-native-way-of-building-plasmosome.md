---
id: 008
title: An AI-native way of building Plasmosome, with constraints that can refuse work
date: 2026-08-31
originator: Stefano
---

Plasmosome development should have a great AI native SDLC with strict guardrails and well defined
processes for its own development with AI self-improvement loops and graph engineering.

The other intents say what Plasmosome should do for the people who run it. This one says how
Plasmosome gets built. Almost every line in this repository is written by an agent, so the way
that work is organised is part of the product's quality, and it is the part nobody has written
down as something I want.

By graph engineering I mean the loop engineering already being practised here, made explicit. The
work already runs as steps that pass results to each other, and the rules between those steps get
decided in one conversation and then forgotten. I want them stated as constraints instead: which
steps may run at the same time, what passes between them, which result counts as evidence, who may
reject one, what is retried and what is not, what survives a restart, where a person has to
approve, and how much may be spent before the whole thing stops. A person approving is one of
those constraints. It is not the only one, and treating it as the only one is how the rest stays
unwritten.

Something outside the loop has to be able to say the work failed. This is the part I care most
about. Many agents on one model, reading the same context, agree with each other cheaply and in
volume, and a process built only out of that agreement looks well governed while proving nothing.
So anything built under this intent needs at least one check whose answer does not come from the
agents being checked: a test that really ran, a measurement of the real thing, a rule the loop
cannot rewrite, or a person's judgement about what better means. A self-improvement loop with no
such check does not improve anything. It converges.

What this covers, and what it does not. It covers a guardrail that catches a class of defect
something outside the loop can confirm it caught, and a process with a decision point something
outside the loop can refuse at. It does not cover an agent noticing something while doing other
work and writing a rule about it — the noticing, the rule, and the verdict that the rule was good
all come from the same place, and that is the thing I want stopped rather than organised. It does
not cover how the kernel behaves; the other intents have that. And it does not turn work that has
already shipped into work I asked for. Mapping a change here is a claim about what that change was
for, not a way to give an old change a parent.

Start with one loop that has a real verifier, state I can look at, and a hard stop. Draw the rest
around it once that one works. I am not asking for a large graph of agents.

## Outcome

(filled in later)
