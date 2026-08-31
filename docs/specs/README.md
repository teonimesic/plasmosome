# Specs

One file per spec, named `NNN-slug.md`. A spec says how something must behave, precisely enough
that a stranger could build it and know when to stop. Copy `docs/templates/spec.md`.

Every **new** spec names an approved intent in its `intents:` field, and may not be planned until
it does; a spec already `accepted` keeps its place whether or not that field is filled in. An
intent on `main` is approved; the owner writes it. That is where the owner's gate sits — a spec is
accepted by the planner who wrote it, not by the owner.

A spec's status flips to `accepted` in the last commit before its pull request merges, so `main`
never holds a spec whose status lies. No task may be claimed until the spec it names is
`accepted`, and every task names one. See `.agents/skills/tasks`.

Numbers are permanent and never reused, so the sequence has gaps where a spec was withdrawn.
There is no `002`: it was written, then withdrawn and rewritten as
[`../decisions/001-instruction-rules-measured.md`](../decisions/001-instruction-rules-measured.md),
because it recorded a settled argument rather than a contract to build against. Nothing is
missing.
