# Intents

One file per intent, named `NNN-slug.md`. An intent says what is wanted and why, with no design
and no solution. Copy `docs/templates/intent.md`.

**`status:` is `draft` or `approved`, and only the owner sets `approved`.** Anyone may write a
draft: proposing an intent is real work, and this folder is where a proposal belongs, so that it
survives instead of dying in a pull request body. An agent may also transcribe an intent the owner
dictates — word for word, never summarized. What an agent may never do is approve one, its own
draft included.

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

**`approved` is a claim in a file, and nothing mechanical checks who wrote it.** Before this, the
folder held only what the owner wrote, so a file's existence was its provenance. A typed line is
weaker: an agent that writes `status: approved` into its own draft produces a file no grep here can
tell from a real approval. That is a deliberate trade for being able to hold a proposal at all, and
it moves the enforcement to the pull request — `main` is protected, so every approval is a diff
somebody has to look at, and an approval arriving in the same change as the work it authorizes is
the shape to refuse.

Work that maps to no intent here is not work that needs an intent written for it; it is work that
has not been asked for. Drafting one puts that question to the owner on the record — it does not
answer it. See `.agents/skills/tasks`.
