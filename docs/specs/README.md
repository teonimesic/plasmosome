# Specs

One file per spec, named `NNN-slug.md`. A spec says how something must behave, precisely enough
that a stranger could build it and know when to stop. Copy `docs/templates/spec.md`.

Every **new** spec names an intent in its `intents:` field, and may not become `accepted` until
that intent reads `status: approved`. Writing it before then is fine and is meant to happen: a
`draft` spec may name a `draft` intent, so a human reading does not idle the queue. A spec already
`accepted` keeps its place whether or not that field is filled in.

A spec's status flips to `accepted` in the last commit before its pull request merges, so `main`
never holds a spec whose status lies — **unless the intent it names is still `draft`, in which case
it merges as a `draft` spec and a later one-line pull request flips it once that intent is
approved.** The planner who wrote the spec is who accepts it; what that
flip waits on is the owner approving the intent above it, not a second reading of the spec. No task
may be claimed until it names a spec and that spec is `accepted`, which is how one approval reaches
every task under it without the owner reading a single one.

That binds what is claimed, not what is already here. Tasks and specs filed before this rule name
nothing above them in some cases, and they are not retroactively wrong — they wait to be mapped
instead of being started. See `.agents/skills/tasks`.

Numbers are permanent and never reused, so the sequence has gaps where a spec was withdrawn.
There is no `002`: it was written, then withdrawn and rewritten as
[`../decisions/001-instruction-rules-measured.md`](../decisions/001-instruction-rules-measured.md),
because it recorded a settled argument rather than a contract to build against. Nothing is
missing.
