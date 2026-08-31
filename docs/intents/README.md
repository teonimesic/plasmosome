# Intents

One file per intent, named `NNN-slug.md`. An intent says what is wanted and why, with no design
and no solution. Copy `docs/templates/intent.md`.

**`status:` is `draft` or `approved`, and approval originates with the owner.** Anyone may write a
draft: proposing an intent is real work, and this folder is where a proposal belongs, so that it
survives instead of dying in a pull request body. An agent may also transcribe an intent the owner
dictates — word for word, never summarized.

**An agent may set `approved` only when it is carrying the owner's actual approval.** That approval
may reach it directly, or by proxy — another agent relaying that the owner approved it. Proxy is
the normal path here and not an edge case: every intent in this folder reached its author through
somebody relaying the owner's words, and a rule forbidding that would forbid how this repository
works. What an agent may never do is **fake** an approval: originate one, infer one from silence,
decide a draft is obviously wanted, or put its own judgement where the owner's belongs. The
question is never who typed the line. It is whether the owner actually approved this and the agent
is recording it, or whether the agent is inventing it. An agent approving its own draft on its own
judgement is the prohibited case, and it stays prohibited.

**Approval is where one decision multiplies into a lot of work.** An approved intent may spin up a
great many specs, and each spec spawns tasks. Drafting costs one document; approving commits a
queue. That is why the owner's gate sits here rather than further down, and it is the test to
apply to a case this file does not cover: if a step turns one person's yes into work nobody has
counted, it belongs on the owner's side of the line.

**A draft may be worked against; it may not be committed to.** A spec may be generated from a draft
intent and stays `draft` itself, so the queue does not idle while a human reads. A spec becomes
`accepted` only once the intent it names is `approved`, and tasks come from accepted specs — so
nothing is ever started against an intent the owner has not approved. A draft spec whose intent is
then refused is discarded. That is the price of working ahead, paid on purpose, and it is nobody's
fault for having written it.

**There is no `rejected` status.** Two values answer the only question that gates anything — may
this be built on — and a third would be a third gate to reason about. A refused draft keeps
`status: draft`, records the refusal in `## Outcome`, and fills the `outcome:` field so the two can
be told apart without reading: **blank means waiting on the owner, non-blank means settled.**
Nothing reads the word itself; the prose under `## Outcome` is what a person reads. Without that
field a refused draft and a forgotten one are the same line forever, and every refusal the project
ever makes stays in the owner's queue.

This does not represent an approval being withdrawn — an intent has no `superseded`, the way a spec
and a decision do, and specs accepted under a rescinded approval would stay accepted. Nothing here
has needed it yet. If it comes up it is the owner's to decide, not an agent's to invent.

**Nothing mechanical tells a relayed approval from a fabricated one, and that follows from the rule
rather than being a gap in it.** Before this, the folder held only what the owner wrote, so a
file's existence was its provenance. Once an agent may record an approval handed to it, a real
approval and an invented one are the same line of text, and no search here can separate them. The
only thing standing between the two is whether the agent recording it is telling the truth. That
is worth stating rather than leaving for someone to discover.

**Agents here act as the owner's GitHub identity**, so nothing about the pull request record
narrows this: the same actor writes the file line, the commit, the description and the review. Any
argument that the record is harder to fake than the file is wrong today, and saying otherwise would
be the false comfort this section exists to prevent.

Three things are true about the risk, and not one of them is a check:

- **The owner reads their own folder.** A fabricated approval is not undetectable, it is detectable
  late, by the only person who can tell. What it costs is whatever was built in between — bounded,
  because the specs and tasks under it are pull requests too.
- **Every approval is a diff**, with an author, a description and a thread. That does not make the
  claim harder to forge; it puts it where a reviewer reads it rather than where a search reports it
  clean, and it turns an ambient property of the folder into a specific statement somebody can
  confront later.
- **Two shapes are refused on sight**: an approval arriving in the same change as the work it
  authorizes, and an approval whose pull request does not say where it came from.

**Nothing mechanical enforces any of this, and that is a decision rather than an omission.**
CODEOWNERS on this folder, a split putting drafts and approvals in separate directories, an
approval workflow and a scoped token were each considered and each turned down: while agents act as
the owner's account they gate nothing, and the one that would work needs an identity split larger
than the thing it protects.
[`../decisions/008-approving-an-intent-is-an-instruction.md`](../decisions/008-approving-an-intent-is-an-instruction.md)
records what was rejected and what would reopen it. **Read "nothing enforces this" as a choice
somebody made, not as work waiting to be done** — and the risk above as its stated price. The rule
itself is in `AGENTS.md`, which every agent reads first.

**Why there is no `approved_by:` field.** Recording who approved an intent and who relayed it would
make the claim specific, not checkable: an agent willing to write `status: approved` writes a name
under it just as easily, and no search, hook or reviewer gets stronger. It buys a more detailed
version of the same lie, and charges ceremony on every intent for it. Asking for the provenance in
the pull request instead costs one sentence and puts it where a reviewer is already reading, which
is the whole of the advantage — not that it is harder to write untruthfully, only that it is read.

Work that maps to no intent here is not work that needs an intent written for it; it is work that
has not been asked for. Drafting one puts that question to the owner on the record — it does not
answer it. See `.agents/skills/tasks`.
