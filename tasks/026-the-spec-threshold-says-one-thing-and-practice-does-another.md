---
id: 026
title: The spec threshold says one thing and every merged PR has done another
status: todo
priority: 3
specs: []
intents: []
refs: [.agents/skills/tasks/SKILL.md]
done_when: >-
  `.agents/skills/tasks` states a spec threshold that the last ten merged pull requests would
  each have satisfied, or names the exception that lets them not.
---

## Why

`.agents/skills/tasks` says a change of 100 lines or more needs an accepted spec before building
starts, and repeats it flatly in the table. No merged change has done this. PR #24 shipped the
attribution guard at 126 lines with no spec and no task, #26 shipped 218 lines with neither, #18
167, #17 103.

A rule nothing follows is worse than no rule: every author has to decide privately whether it
applies to them, and the honest ones lose time arguing it while the rest ignore it. It also makes
the rule useless as a review standard, because a reviewer citing it is citing something the
repository has never once done.

The question is what the threshold should count, and that is the owner's to answer, not an
author's. Candidates, none obviously right:

- **Product lines only**, excluding tests, fixtures and documentation. Most of what pushes a
  change past 100 lines here is tests, and a change is not risky because it is well tested.
- **Contract surface**, dropping the line count entirely — a spec when a change alters something
  a caller depends on, whatever its size.
- **Keep the number and start obeying it**, accepting that most changes here become two pull
  requests.

Whichever is chosen, the text and the practice have to end up agreeing.

## Plan

## Notes

Filed from the independent review of PR #35, which hit the gap directly: that change was 297 lines
by `--numstat` with no spec, the author judged the clause to be about kernel capability work rather
than a hook fix, and the reviewer confirmed the judgement matched every precedent while noting the
text contains no such carve-out. Reading a carve-out into text that does not have one is what this
task exists to stop.
