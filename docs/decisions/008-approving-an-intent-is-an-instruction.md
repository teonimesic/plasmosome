---
id: 008
title: Approving an intent is enforced by instruction, not by a merge gate
date: 2026-08-31
status: accepted
---

## Context

An intent now carries `status: draft` or `status: approved`, and approval is the owner's: an
approved intent may spin up many specs, each of which spawns tasks, so it is the point where one
decision multiplies into a lot of work. An agent may record an approval it is carrying — relayed
by another agent or heard directly — and may never originate one.

The question this settles is what enforces that. Nothing in the repository reads these fields:
`.githooks/` and `.github/workflows/` do not, and the searches added alongside the field report
what is *missing* rather than what is *wrong*. An independent reviewer put it precisely: an intent
born `status: approved` is invisible to every one of them.

Two facts bound every mechanical answer. **Agents act as the owner's GitHub identity** — 42 of the
43 human-authored pull requests here are one account, and the reviews on them are posted under it —
so a relayed approval and a fabricated one are written by the same actor, in the same voice, and no
record distinguishes them. And **the thing being protected is a sentence in a document**, not a
capability the operating system can withdraw; there is no seam where a check could sit that the
agent writing the sentence does not also control.

## Decision

The rule is an instruction. `AGENTS.md` carries it in its shortest form, `docs/intents/README.md`
carries it in full with what it costs, and nothing in CI or in branch protection tries to hold it.

This is the form this project already uses to govern what agents do. The rules in `AGENTS.md` — who
may write into `tasks/`, what earns a place in a skill, that a new rule arrives with evidence — are
instructions that no check enforces, and they work because agents read them. The mechanical gates
that do exist here guard facts about the tree that a script can read on its own: a forbidden term,
a trailer crediting a model. Whether a person really said yes is not that kind of fact.

## Rejected

**CODEOWNERS requiring the owner's review on `docs/intents/`.** The natural mechanism, and
unusable: GitHub forbids approving your own pull request, so while author and approver are one
account the rule would not gate these changes, it would block them outright. It becomes available
only after agents have an identity of their own, which is a much larger change than the one it
would protect.

**A path-based split — drafts in one directory, approvals in another, the move being the
approval.** This is the only option that closes the hole structurally, because a file's location is
not something the file's author can assert. But it gates nothing until the identity split above
happens, so today it buys the cost without the protection: the `status:` field would have to be
removed rather than kept beside the path, four searches would be rekeyed, and numbering would have
to stay global across both directories so a file's id survives the move. Worth revisiting the day
agents have their own identity; not before.

**An approval workflow or a protected environment.** Required reviewers on a deployment environment
gate a job, not a merge, so holding a merge behind one means making a bot the merge authority for
this folder. That is more machinery, with its own credentials and its own failure modes, than the
risk justifies.

**Scoping the agents' token so it cannot write this path.** The same identity problem in a
different place, and it would put every intent — drafts included — behind a second actor, which
defeats the reason drafts were allowed at all.

## Consequences

**The residual risk is now a stated price rather than a gap.** An agent that lies about carrying an
approval can start a chain of specs and tasks under a goal the owner never asked for, and it runs
until the owner reads their own folder. That is bounded — the specs and tasks beneath it are each a
pull request — and it is detectable late by the one person who can tell. `docs/intents/README.md`
says so, and says the pull-request record does not narrow it, because the same actor writes the
file line, the commit and the review.

**"Nothing enforces this" is not a defect report.** An agent finding the gap and filing work to
close it would be doing the thing the work chain exists to stop: manufacturing a task from a review
observation that maps to no goal. This record is the answer to that finding; if the reading is
wrong, the argument goes rather than the rule bending.

**What would reopen it:** agents getting a GitHub identity distinct from the owner's. Everything
rejected above turns on that one fact, and CODEOWNERS becomes both possible and cheap the moment it
changes.

**Evidence.** The rule this adds to `AGENTS.md` carries no A/B result, and decision 001 asks for one
before a rule lands. `AGENTS.md` limits that requirement to a rule about what an agent writes,
which the method can score by running a task twice; a rule about who may make a decision produces no
code to score. Such a rule lands on its reasoning and must name the failure it prevents, and this
record is that reasoning. The failure is concrete enough to check for: an intent reaching `main` as
`approved` that the owner never approved.
