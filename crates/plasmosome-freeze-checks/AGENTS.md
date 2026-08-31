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
- **A rule about what the code does reads the code, not its bytes.** `source.contains("Mutex")` is
  a claim about text, and it is wrong in both directions: it fails on a doc comment that mentions a
  lock, and it clears a struct holding a `RefCell`. Parse instead — `shared_memory::shared_memory_uses`
  does, and is the model for any rule that has to look inside a file. See
  [`docs/decisions/004-a-rule-about-code-parses-code.md`](../../docs/decisions/004-a-rule-about-code-parses-code.md).

## What the shared-memory rule cannot see

`shared_memory::shared_memory_uses` reads one file at a time. It follows the aliases that file
declares about itself — `use std::sync::Mutex as Guard;` and `type Guard = Mutex<u32>;` — and it
does not follow an alias declared anywhere else. A sibling module writing
`pub use std::sync::RwLock as Registry;`, imported here as `use crate::aliases::Registry;`, passes.

That is name resolution, and answering it needs the compiler rather than a parser. The miss is
pinned by `an_alias_declared_in_another_file_is_missed_because_that_needs_name_resolution` in
`tests/shared_memory_reads_rust.rs`. If that test ever fails, the limit has been closed and this
section is stale. Widen the construct list to catch more; do not reach back for a regex.

## Testing

`cargo test -p plasmosome-freeze-checks`.
