# Intents

One file per intent, named `NNN-slug.md`. An intent says what is wanted and why, with no design
and no solution. Copy `docs/templates/intent.md`.

`status:` is `draft` or `approved`. Anyone may write a draft — proposing an intent is real work,
and this folder is where a proposal belongs — and it does not matter who writes one. **Approval
originates with the owner**: an agent may record one it is carrying, relayed by another agent or
heard directly, and may never originate one, its own draft least of all. The question is never who
typed the line, only whether the owner really approved it.

**A pull request proposing an intent, or moving one to `approved`, stays a draft until the owner
has read it on GitHub and approved it there**; an agent does not mark it ready. That is where their
reading happens, so it is where the waiting is visible.

Intent descriptions, approval state and outcomes remain versioned Markdown; task descriptions,
plans, notes and lifecycle live entirely in shared native Beads under
[spec016](../specs/016-native-beads-task-authority.md). Its full-task cutover approves no unrelated
draft intent. Read tasks by Beads ID using `./tools/work-state show ID`, not a former task path.

Spec acceptance and task admission are described in [the spec index](../specs/README.md).
A refused intent stays `draft` and fills `outcome:`, distinguishing it from a forgotten proposal.
Each intent declares its ID and state once in frontmatter. Its three-digit ID is permanent and
belongs to the intent namespace, not the native task namespace; coordinate new document IDs
against Git documents and open document PRs.

**Nothing mechanical enforces any of this, and that is a choice rather than an omission.**
[`../decisions/008-approving-an-intent-is-an-instruction.md`](../decisions/008-approving-an-intent-is-an-instruction.md)
records what was rejected, what it costs, and what would reopen it. See
[the task skill](../../.agents/skills/tasks/SKILL.md) for downstream work.
