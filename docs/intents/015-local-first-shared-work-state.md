---
id: 015
title: Shared work state agents can read locally
status: approved
date: 2026-09-01
originator: Stefano
outcome:
---

Plasmosome should keep intents, specs and tasks as durable documents while the changing status and
coordination of that work live in shared state. An agent returning in another session or on another
machine should be able to see what is ready, claimed, blocked, under review or done without
reconstructing that truth from old files and remote activity.

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

## Outcome

(filled in later)
