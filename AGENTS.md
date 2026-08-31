# Working in this repository

Plasmosome is a capability kernel: it grants and revokes operating-system capabilities for AI
agents running inside isolated cells. Read the root `README.md` for what that means, then the
`AGENTS.md` of whichever crate you are touching — each one carries the rules that constrain
changes to it.

## The two rules that shape everything else

**Enforcement is not cooperation.** A capability is revoked when the operating system stops
permitting it — a closed socket, an unmounted path, a dead credential handle — never when an
agent is asked nicely to stop. Any change that moves a check from the OS into the harness is
moving in the wrong direction.

**Nothing outlives its owner unnoticed.** A process that keeps running after its capabilities
were revoked is the bug class this project exists to prevent. Every spawn is paired with a reap;
every grant records how to undo it.

## How work happens

| Skill | Use it when |
| --- | --- |
| `.agents/skills/planning-work` | Starting a piece of work, or briefing another agent |
| `.agents/skills/pr-review` | Opening a PR, addressing review feedback, or merging |
| `.agents/skills/tasks` | Finding, filing, or closing work — the layers, formats and links |
| `.agents/skills/heartbeat` | The start-of-session routine: reconcile, then pick what to do next |

Short version. Work is written down before it is done: in `tasks/` always, and in `docs/specs/`
first when it is large or touches a contract. It moves in small, finishable pieces. The strongest
model available plans; the next one down executes. `main` is protected, so every change arrives
by pull request. Two reviewers read each PR, and one of them checks claims by breaking things
rather than by reading.

**What earns a place in a skill:** a rule that changes what the next agent does, and will again.
Not a one-off — a migration you just ran, a path that moved, a bug you just fixed. Those belong
in the PR and the git log. If it cannot happen twice, it is not a skill.

## Style

**Writing.** The first three paragraphs of any document should explain almost everything: what it
is, why it exists, how to use it. Lead with the point; details come after. Use plain English —
short sentences, concrete nouns, no clever words. Write for a good engineer who is new here and
should not have to pause over a sentence.

**Code.** No inline `//` comments. Documentation is `///` on public items: what a caller must
pass, what it gets back, what it must not do. If deleting a doc block costs a caller nothing,
delete it.

## The gate

```shell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
./.githooks/provenance-guard
```
