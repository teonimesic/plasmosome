# Decisions

One file per decision, named `NNN-title.md`. Copy `docs/templates/decision.md`: context,
decision, rejected alternatives, consequences — around half a page each.

Write one whenever a change settles a question someone will otherwise re-open: how a boundary
works, what a component owns, why the obvious approach was not taken. A decision made in a diff
and nowhere else gets undone by the next person who has the same idea.

A decision is never edited. When it stops holding, write a new one and mark the old one
`superseded` — the reasoning that was true at the time is the point of keeping it.
