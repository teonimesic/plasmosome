# Intents

`NNN-slug.md`: goal/why, not design/solution. Copy `docs/templates/intent.md`.
Declare ID/status once in frontmatter; coordinate permanent three-digit intent IDs against
Git/open PRs, not native task IDs.

Anyone may draft; only the owner originates approval and coverage values/prose.
Agents may record and cite judgments, never infer them from checks/counts.
Proposal/approval PRs and either coverage-judgment change (including backfill) stay draft
until owner GitHub reading/approval and owner-ended draft; agents never mark them ready.

`status:` is `draft`/`approved`; refusal stays draft and fills `outcome:`.
`served:`: `none` nothing, `partly` some, `substantially` most built.
`## What is served` records built/remaining, not spec/task IDs/counts; retain its final
account when settled. No terminal value; open work is normal. Coverage neither approves
goals nor gates admission/priority.

Blank template means missing judgment, not `none`; disclose fault3. The existing local
gate permits its draft publication; never gate on live coverage.
Read-only checkout-root commands (no synchronization); invalid inputs/unknown evidence refuse first:
- `./tools/intent-coverage show INTENT_ID`: JSON Lines of specs/once-only
  native tasks/statuses/dispositions/merge-or-cancellation evidence; ignores served.
- `./tools/intent-coverage check`: exit2 refusal; exit1
  [009](../specs/009-how-much-of-an-intent-is-built.md)'s three faults;
  exit0/silent means consistency, not satisfaction.

Git intents; Beads tasks/plans/notes: `./tools/work-state show ID`, not task files.
[016](../specs/016-native-beads-task-authority.md): lifecycle;
[spec index](../specs/README.md): acceptance/admission;
[tasks](../../.agents/skills/tasks/SKILL.md): downstream work.
[Decision008](../decisions/008-approving-an-intent-is-an-instruction.md) records the unauthenticated
approval boundary, costs and reconsideration.
