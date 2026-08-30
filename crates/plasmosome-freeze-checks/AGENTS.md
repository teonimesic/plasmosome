# plasmosome-freeze-checks — working notes

## What this crate is

Architectural invariants as tests. It contains no product code; it inspects the workspace.

## Hard rules

- **Never relax a rule to make a change pass.** The rule is the decision; the change is the
  candidate. If they conflict, re-examine the change first, and if the rule is genuinely wrong,
  change it deliberately with its rationale updated — not as a side effect.
- **A rule must be mutation-tested.** A check that cannot fail proves nothing: verify it catches
  the violation it claims to catch, then revert the violation.
- **Rules describe boundaries, not style.** Formatting belongs to fmt and clippy.

## Conventions

- No inline `//` comments. `///` contract docs on public items only.

## Testing

`cargo test -p plasmosome-freeze-checks`.
