---
id: 008
title: An AI-native way of building Plasmosome, with constraints that can refuse work
date: 2026-08-31
originator: Stefano
---

Plasmosome development should have a great AI Native SDLC with strict guardrails and well defined
processes for its own development with AI self-improvement loops and graph engineering.

The other intents say what Plasmosome should do for the people who run it. This one says how
Plasmosome gets built. Almost every line in this repository is written by an agent, so the way that
work is organised is part of the product's quality, and almost none of it has been written down as
something I want.

By graph engineering I mean the loop engineering already being practised here, made explicit. The
work already runs as steps that pass results to each other, and the rules between those steps get
decided in one conversation and then forgotten. I want them stated as constraints instead: which
steps may run at the same time, what passes between them, which result counts as evidence, who may
reject one, what is retried and what is not, what survives a restart, where a person has to
approve, and how much may be spent before the whole thing stops. A person approving is one of those
constraints. It is not the only one, and treating it as the only one is how the rest stays
unwritten.

Something outside the loop has to be able to say the work failed. Many agents on one model agree
with each other cheaply and in volume, and a process built only out of that agreement looks well
governed while proving nothing. So anything built under this intent needs at least one check whose
answer does not come from the agents being checked: a test that really ran, a measurement of the
real thing, a rule the loop cannot rewrite, or a person's judgement about what better means. The
check has to be able to come back negative. Something that could only ever agree is not a check,
and a loop wired to one does not improve. It converges.

What this covers, and what it does not. Two things have to hold together, and either on its own
lets the wrong work through. A guardrail belongs here when something outside the loop can confirm
it catches what it claims to catch, and the kind of defect it catches is one somebody asked to have
caught. A process belongs here when it has a decision point something outside the loop can refuse
at, and the thing being decided is one somebody asked to have decided. The check alone is not
enough: an agent can invent a guard nobody wanted and write a test that proves the guard works, and
that test will pass honestly. Being asked for alone is not enough either, because then nothing can
tell me it failed.

What this does not cover is an agent noticing something while doing its own work and writing a rule
about it. The noticing, the rule, and the verdict that the rule was good all come from the same
place. It does not cover how the kernel behaves either; the other intents have that. And it does
not turn work that has already shipped into work I asked for — mapping a change here is a claim
about what that change was for, not a way to give an old change a parent.

Start with one loop that has a real verifier, state I can look at, and a hard stop. Draw the rest
around it once that one works. I am not asking for a large graph of agents.

## Outcome

(filled in later)
