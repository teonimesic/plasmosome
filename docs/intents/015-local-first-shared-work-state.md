---
id: 015
title: Shared work state agents can read locally
status: approved
date: 2026-09-01
originator: Stefano
outcome:
---

Plasmosome should keep intents and specs as durable versioned documents, and complete task records
in durable shared state. An agent returning in another session or on another machine should be
able to see the task's description, plan, acceptance, notes and evidence as well as what is ready,
claimed, blocked, under review or done, without reconstructing that truth from old files and
remote activity. Concurrent sessions sharing the active task store must not both claim and carry
out the same work.

Agents ask these questions repeatedly. The state needed for ordinary planning should be available
locally and while disconnected, so reading it does not require another network request each time.
When that local view is not current, the difference must be visible rather than letting an agent
mistake an old answer for shared truth.

Keep this small, fast and reliable. The need is current work state and enough coordination to stop
agents duplicating or losing work, not a general-purpose project-management system whose process
becomes work of its own. The durable documents should remain useful on their own and should not be
replaced by transient tracking data.

The foundation must be open source, free to use, or genuinely free for open-source projects. The
way Plasmosome coordinates its own development should not create a per-user licensing cost or make
continued access to its work history depend on buying a commercial service.

## Owner scope amendment

The owner explicitly selected all task content in Beads, including descriptions, plans, notes,
status and ownership, and retirement of task Markdown. This supersedes the original separation
between task documents and changing coordination state; it does not replace specs/intents or
approve another intent. Spec016 records the resulting native runtime contract and supersedes
spec014's custom shadow/ledger design.

The chosen coordination domain is one clone's shared store across its linked worktrees.
Explicit remote replication provides backup and another machine's local view; it does not
provide global exactly-once dispatch between independent writer clones. Moving active writing
to another clone requires an explicit quiescent handoff rather than pretending replication
provides that exclusion. Local reads do not establish remote freshness.

## Outcome

(filled in later)
