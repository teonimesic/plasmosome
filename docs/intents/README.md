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
queue. That is why the owner's gate sits here rather than further down, and it is the test to apply
to a case this file does not cover: if a step turns one person's yes into work nobody has counted,
it belongs on the owner's side of the line.

**A draft may be worked against; it may not be committed to.** A spec may be generated from a draft
intent and stays `draft` itself, so the queue does not idle while a human reads. A spec becomes
`accepted` only once the intent it names is `approved`, and tasks come from accepted specs — so
nothing is ever started against an intent the owner has not approved. A draft spec whose intent is
then refused is discarded. That is the price of working ahead, paid on purpose, and it is nobody's
fault for having written it.

**There is no `rejected` status.** Two values answer the only question that gates anything — may
this be built on. A refused draft keeps `status: draft` and records the refusal in its `## Outcome` — which is the section's
"or why nothing was" — so the reasoning survives where the next person asking the same question
will find it. A draft with an empty `## Outcome` is waiting; one with a filled-in `## Outcome` is
settled.

Work that maps to no intent here is not work that needs an intent written for it; it is work that
has not been asked for. Drafting one puts that question to the owner on the record — it does not
answer it. See `.agents/skills/tasks`.
