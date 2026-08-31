---
id: 002
title: Agent instruction rules, and the test every new one must pass
status: draft
intents: [001]
---

## Behavior

Every rule an agent reads in this repository must earn its place by changing what an agent
produces. This spec sets the rules that passed that test, removes the one that failed, and makes
the test itself the standing entry requirement for any future rule.

The test is an experiment, not an opinion. Give a model the same task twice — once with the
candidate rule appended, once without — and score a mechanical outcome on the code it writes. A
rule with no measurable effect does not ship, however sensible it reads. Every agent pays context
for every rule on every turn, so a rule that changes nothing makes the whole system slower and no
better.

The evidence here comes from 112 runs of `claude -p --model sonnet`, six candidate rules, eight
runs per arm, task text otherwise identical between arms. Sonnet because the executor is who
reads these rules while writing code. Full method, limits and per-rule output are in `## Design`.

## Contract

**Rules that stay, with their measured effect.**

| Rule | Home | With | Without |
| --- | --- | --- | --- |
| Every spawn is paired with a reap; reaping happens on drop | root `AGENTS.md` | 8/8 | 0/8 |
| Never signal a pid after its terminal state was observed | root `AGENTS.md` | 8/8 | 0/8 |
| A review must not accept a test that cannot fail against the bug it names | `pr-review` | 8/8 | 0/8 |
| No inline `//` comments | root `AGENTS.md` | 16/16 | 10/16 |

**One rule is added.** *Accept dependencies, do not create them. If a test has to drive the real
operating system to observe the behavior, the seam is in the wrong place.* Measured 8/8 versus
0/8 for declaring a trait, 7/8 versus 0/8 for a test that drives a fake. It costs about a third
more code, so it carries its own brake: two adapters means a real seam, one means a hypothetical
one.

**One rule is removed and must not come back.** Any instruction to retry a syscall that returns
`EINTR`. Measured 16/16 in both arms — the model already does it. This is recorded so nobody
proposes it again.

**Two rules currently conflict, and the conflict is resolved here.** A rule that explains why it
exists makes the model narrate that reasoning in `//` comments: the reap rule produced a mean of
3.6 inline comment lines against a baseline of 1.0 on the same task, directly violating the
comment ban. Rule text in `AGENTS.md` and in skills states the rule only. The reasoning moves to
the crate's own document, which is prose an agent reads for understanding rather than a rule it
is trying to satisfy.

**The entry requirement.** A new rule proposed for `AGENTS.md` or any skill arrives with an A/B
result: the rule text, the task, at least eight runs per arm, the scored outcome, and the
verdict. A rule without one is not rejected on principle — it is not yet decided.

## Design

**Scoring must be semantic, never textual.** The first scorer in this experiment grepped for the
literal string `EINTR` and produced a clean 0/8 versus 8/8, which would have shipped a rule that
does nothing. The baseline was already retrying, through `io::ErrorKind::Interrupted`. The grep
was measuring vocabulary. A scorer that matched braces and read the control flow reversed the
result to 16/16 versus 16/16. Any future sweep writes a scorer that inspects behavior, and states
what it inspects.

**Compilation cannot discriminate.** All 72 generated files compiled under `cargo check
--all-targets`, in both arms, on every task. At this task size a passing build says nothing about
whether a rule worked. Do not use it as an outcome.

**Eight runs per arm is a floor, not a guarantee.** A secondary hypothesis looked like 5/8 versus
1/8 at n=8 and washed out to 11/16 versus 7/16, p=0.29, when the sample doubled. Report the
larger sample, and report inconclusive results as inconclusive.

**A rule can suppress findings elsewhere.** Appending the falsifiability rule to a general review
prompt raised detection of the planted vacuous test from 0/8 to 8/8, but pid-reuse detection fell
from 7/8 to 4/8 and mean findings per review fell from 6.9 to 5.5. It ships as an explicitly
invoked test-falsifiability pass, not as another line in a general review.

**Known limit of the method.** The global `~/.claude/CLAUDE.md` could not be isolated — it was
present in both arms, so it raises the baseline rather than biasing the comparison, but it likely
understates any rule about testing. Runs are single-shot generations with no tool use, so this
measures first-draft code, not behavior across a session.

**Cost.** 112 runs took about 7 minutes of wall clock at 12-way parallelism, roughly 40 seconds
per run. A six-rule sweep is a coffee break, which is why it is affordable as a standing gate.

**Prior art.** Of the 19 skills in `mattpocock/skills`, three carry an idea that transfers here
and none carry a file worth importing: the dependency-seam rule adopted above, the tautological
test as a review target, and the practice of settling an instruction by running it rather than
arguing about it. Importing the collection would add roughly 3,900 lines that every agent pays
for on every turn.

## Acceptance

- Root `AGENTS.md` states the reap rule and the pid rule with no explanatory clause, and the
  reasoning for each appears in `crates/plasmosome-membrane/AGENTS.md` instead.
- The pid rule appears in root `AGENTS.md`, not only in the membrane crate.
- `crates/plasmosome-backend/AGENTS.md` states the dependency-seam rule and its two-adapter brake.
- No file in the repository instructs anyone to retry `EINTR`, and this spec is what a future
  proposal is pointed at.
- The `pr-review` skill describes the test-falsifiability check as its own pass, named and
  separately invoked, rather than a bullet inside the general review.
- `AGENTS.md` states that a proposed rule arrives with an A/B result, naming the four things it
  must report.
- `git grep -n 'EINTR'` returns only source code, never instruction text.
- The gate is green: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo fmt --all -- --check`, `./.githooks/provenance-guard`.
