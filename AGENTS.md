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
| `.agents/skills/build-slice` | Starting a piece of work, or briefing another agent |
| `.agents/skills/pr-review` | Opening a PR, addressing review feedback, or merging |

Short version: work moves in narrow slices; the strongest model plans and the next one executes;
`main` is protected and every change arrives by pull request; two reviewers look at each PR and
one of them verifies claims by breaking things rather than reading.

## Style

No inline `//` comments. Documentation is `///` on public items: what a caller must pass, what
it gets back, what it must not do. If deleting a doc block costs a caller nothing, delete it.

## The gate

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
./.githooks/provenance-guard
```
