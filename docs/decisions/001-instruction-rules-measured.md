---
id: 001
title: Which agent instructions we keep, measured rather than argued
date: 2026-08-31
status: accepted
---

## Context

Nearly every line of code here is written by an agent, so the instructions agents read are the
main lever on quality. Those instructions were written from intuition and nobody had checked
whether any of them changed what an agent produced. Every rule costs context on every turn, so a
rule that changes nothing makes the whole system slower and no better.

We ran an experiment instead of arguing. 112 runs of `claude -p --model sonnet`, six candidate
rules, eight runs per arm, task text identical between arms apart from the appended rule, scored
on a mechanical outcome. Sonnet because the executor is who reads these rules while writing code.

The raw outputs, the task and rule files, and the scoring scripts are not in this repository —
they were scratch. What follows is the account of what they showed, checked by an independent
reviewer who wrote their own scorers and re-derived the headline numbers from the raw outputs.

## Decision

Four existing rules keep their place, one is added, one is deleted. Every rule proposed for
`AGENTS.md` or a skill from now on arrives with an A/B result.

| Rule | With | Without | p |
| --- | --- | --- | --- |
| Every spawn is paired with a reap; reaping happens on drop | 8/8 | 0/8 | 0.0002 |
| Never signal a pid after its terminal state was observed | 8/8 | 0/8 | 0.0002 |
| Accept dependencies, do not create them — **added** | 8/8 | 0/8 | 0.0002 |
| A review must not accept a test that cannot fail | 8/8 | 2/8 | — |
| No inline `//` comments | 16/16 | 10/16 | 0.018 |
| Retry a syscall that returns `EINTR` — **deleted** | 16/16 | 16/16 | 1.0 |

The seam rule is new and costs about a third more code (mean 181.6 to 244.5 lines), so it carries
its own brake: two adapters means a real seam, one means a hypothetical one.

The `EINTR` rule is deleted and recorded here so nobody proposes it again. The model already
retries, through `io::ErrorKind::Interrupted`. Three baseline bodies were read by hand to confirm
the match was not spurious.

## Rejected

**Scoring by looking for words.** The first `EINTR` scorer grepped for the literal string and
produced a clean 0/8 versus 8/8 — a confident verdict for a rule that does nothing. It was
measuring vocabulary. A scorer that read the control flow reversed it to 16/16 versus 16/16. Any
future sweep writes a scorer that inspects behavior and states what it inspects.

This mistake was made twice. The falsifiability rule was first scored 8/8 versus 0/8; the true
baseline is 2/8, because two runs reached the same conclusion in different words. Reading for a
framing rather than for the substance is the failure mode, and it survives being warned about.

**"Does it compile" as an outcome.** 94 of 96 generated files compiled, and the two failures were
unrelated to any rule under test. A passing build says nothing at this task size.

**Restructuring the review skill around the suppression result.** Appending the falsifiability
rule to a general review appeared to cost other findings: pid detection fell and mean findings per
review fell from 6.9 to 5.5. At p=0.28 that is the same evidence strength as a result we called
inconclusive, at half the sample. It is not acted on. The rule stays where it is.

**Importing `mattpocock/skills`.** Three of its nineteen skills carry an idea that transfers and
none carry a file worth importing. The dependency-seam rule above is the one that measurably
changed behavior. Importing the collection would add roughly 3,900 lines every agent pays for.

## Consequences

Eight runs per arm is a floor, not a guarantee. One hypothesis looked like 5/8 versus 1/8 at n=8
and washed out to 11/16 versus 7/16 when the sample doubled. Report inconclusive as inconclusive.

Nine comparisons were run without correcting for multiple testing. The four 8/8-versus-0/8 results
are extreme enough that it does not matter. The comment rule at 16/16 versus 10/16 does not
survive a Bonferroni correction, so it is kept as the weakest thing in the table, not as proof.

**Every appended rule makes the model write more comments** — the reap rule 3.62 lines against a
1.00 baseline, but the pid rule 1.62 against 0.00 and even the deleted `EINTR` rule 3.38 against
1.81. This collides with the comment ban. The cause is not known: no arm varied the rationale, so
whether it is the explanation or merely the presence of any extra instruction is untested, and
this decision does not prescribe a fix for it. Task 007 does not act on it.

A sweep is cheap — 112 runs took 7 minutes at 12-way parallelism, about 45 seconds per run — which
is why the entry requirement is affordable.

The global `~/.claude/CLAUDE.md` could not be isolated. It was present in both arms, so it raises
the baseline rather than biasing the comparison, but it likely understates any rule about testing.
