# Specs

Specs remain versioned Markdown, one `NNN-slug.md` per permanent three-digit ID. A spec says how
something must behave, precisely enough for a stranger to build it and know when to stop. Copy
`docs/templates/spec.md`; implementation plans belong in Beads tasks, not in this folder.

Each spec declares `id:`, `status:` and `intents:` once in frontmatter. `status:` is `draft`,
`accepted` or `superseded`. The intent links are a one-line flow list of three-digit IDs, such as
`intents: [015]`; every new spec names an intent. All referenced IDs must resolve uniquely.
Numbers are never reused, including gaps left by withdrawn specs. Coordinate new document IDs
against existing Git documents and open document PRs, not native task IDs.

## Acceptance and task admission

A draft spec may name a draft intent. A spec cannot become accepted until every intent it names
is approved under the [intent approval rule](../intents/README.md). The planner accepts the spec;
there is no additional owner spec-approval gate. Set `accepted` in the final reviewed commit
before merging its PR, after those approvals exist. If they do not exist, it remains draft,
including on main, until a later reviewed acceptance change.

An implementation task reads the accepted spec on main before it is claimed or its code worktree
opens. Work needing a new spec therefore lands that spec first. Spec012 governs the chain and
its two structural taskless PR shapes. Every accepted spec names intents; the former spec001
exception is closed, not a general amnesty. Imported unmapped tasks keep their history in Beads,
but cannot start on an empty chain.

## Native task authority

[Spec016](016-native-beads-task-authority.md) is the active task-storage and native command
contract under approved intent015 and the owner's complete-task-content decision.
[Spec014](014-local-first-work-state.md) is superseded historical design, not a supported shadow
API or a request to preserve task Markdown. [Spec012](012-how-work-enters-the-tree.md) retains
admission and review governance with native task records.

Find tasks and their plans through `./tools/work-state show ID` and the
[task skill](../../.agents/skills/tasks/SKILL.md), never a task-file link or a tracked export.
Spec/intent states are Git fields; Beads owns the complete task lifecycle and contents.

There is no spec002: it was withdrawn and rewritten as
[decision001](../decisions/001-instruction-rules-measured.md), a settled choice rather than a
buildable contract. That permanent gap does not reserve or number a task.
