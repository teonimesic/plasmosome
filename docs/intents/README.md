# Intents

One file per intent, named `NNN-slug.md`. An intent says what is wanted and why, with no design
and no solution. Copy `docs/templates/intent.md`.

`status:` is `draft` or `approved`. Anyone may write a draft — proposing an intent is real work,
and this folder is where a proposal belongs. **Approval originates with the owner.** An agent may
record one it is carrying, relayed by another agent or heard directly, and may never originate one,
its own draft least of all. The question is never who typed the line, only whether the owner really
approved it.

A spec may be drafted against a `draft` intent and may not become `accepted` until that intent is
`approved`; tasks come from accepted specs. A refused draft stays `draft` and fills `outcome:`,
which is what tells it from a forgotten one.

**Nothing mechanical enforces any of this, and that is a choice rather than an omission.**
[`../decisions/008-approving-an-intent-is-an-instruction.md`](../decisions/008-approving-an-intent-is-an-instruction.md)
records what was rejected, what it costs, and what would reopen it. See `.agents/skills/tasks`.
