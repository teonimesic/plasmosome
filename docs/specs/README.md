# Specs

One file per spec, named `NNN-slug.md`. A spec says how something must behave, precisely enough
that a stranger could build it and know when to stop. Copy `docs/templates/spec.md`.

A spec is `draft` until its pull request merges, and `accepted` after. A task that crosses the
spec threshold — 100 lines or more, or enforcement semantics, or a public contract — may not be
claimed until the spec it names is `accepted`. See `.agents/skills/tasks`.

Numbers are permanent and never reused, so the sequence has gaps where a spec was withdrawn.
There is no `002`: it was written, then withdrawn and rewritten as
[`../decisions/001-instruction-rules-measured.md`](../decisions/001-instruction-rules-measured.md),
because it recorded a settled argument rather than a contract to build against. Nothing is
missing.
