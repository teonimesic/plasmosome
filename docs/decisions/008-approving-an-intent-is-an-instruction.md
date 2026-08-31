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

The question this settles is what enforces that. No hook and no workflow reads these fields:
`.githooks/` and `.github/workflows/` do not, so nothing refuses a push or fails a build over
them. One check does read them — the cross-layer loop in `.agents/skills/heartbeat` step 4 — and it
catches an accepted spec whose intent is unapproved or absent. It cannot catch the case that
matters here. An independent reviewer put that precisely: an intent born `status: approved` is
invisible to every check in the tree, because a relayed approval and a fabricated one are the same
line of text.

Two facts bound every mechanical answer. **Agents act as the owner's GitHub identity** — every
human-authored pull request in this repository is one account, and the reviews on them are posted
under it — so a relayed approval and a fabricated one are written by the same actor, in the same
voice, and no record distinguishes them. And **the thing being protected is a sentence in a document**, not a
capability the operating system can withdraw; there is no seam where a check could sit that the
agent writing the sentence does not also control.

**Authorship is not the control, and was never the risk.** It does not matter who writes an intent,
as long as the owner approves it: an agent may draft one in its own words, and the owner reads it,
asks for changes, and approves it or does not. The failure that matters is an agent claiming an
approval nobody gave. Those are two different failures and only the second one is worth a rule.

## Decision

An instruction, and a visible waiting state.

**The instruction.** `AGENTS.md` carries the rule in its shortest form and `docs/intents/README.md`
carries it in full with what it costs. Nothing in CI or in branch protection tries to hold it.

**The waiting state.** A pull request proposing an intent, or moving one to `status: approved`,
stays a draft until the owner has read it on GitHub and approved it there. An agent does not mark
it ready; the owner does. This is where their reading actually happens, so it is where the waiting
belongs. It costs nothing, `gh pr list` shows it, no agent has a reason to flip it on the owner's
behalf, and — unlike everything under "Rejected" — it does not need identity separation to mean
something, because it is a coordination signal rather than a permission boundary.

It is a convention, not a boundary. An agent *can* mark such a pull request ready, exactly as it
can write `status: approved` into a file. That is the same class of guarantee as every other link
in this chain, and it is accepted deliberately rather than overlooked.

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

**Recording provenance in the file — `approved_by:`, `relayed_by:`.** It makes the claim specific
without making it checkable: an agent willing to write `status: approved` writes a name under it
just as easily, and no search, hook or reviewer gets stronger for it. Every intent would pay
ceremony for a more detailed version of the same untruth. Asking for the provenance in the pull
request instead costs one sentence and puts it where a reviewer is already reading — not because it
is harder to write untruthfully there, only because it is read.

## Consequences

**The residual risk is now a stated price rather than a gap, and a smaller one than the instruction
alone would leave.** An agent that lies about carrying an approval can start a chain of specs and
tasks under a goal the owner never asked for. The draft state does not stop that — an agent can
mark its own pull request ready, so the waiting state makes the claim **visible**, never blocked.
What it buys is that the approval arrives somewhere the owner is already watching. Where the lie
survives that, it runs until the owner reads their own folder — bounded, because the specs and tasks
beneath it are each a pull request, and detectable by the one person who can tell. The
pull-request record does not narrow it either: the same actor writes the file line, the commit and
the review.

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
