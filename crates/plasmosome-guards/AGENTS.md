# plasmosome-guards — working notes

## What this crate is

The repository's own guards, as tests. It contains no product code; it inspects the workspace.

## What earns a guard here

**Only harm that is permanent or public.** A published crate version cannot be unpublished, a
commit crediting a model is in the history of a public repository, and the research corpus cannot
be un-leaked. Those are the shape. A guard that would merely keep a design as it is today does not
go here: write the design down where it can be argued with, and let the argument happen in review.

The test to apply before adding one: name what it prevents, and say whether that thing could be
undone by the next commit. If it could, this is not the place for it.

## Hard rules

- **Never relax a guard to make a change pass.** The guard is the decision; the change is the
  candidate. If they conflict, re-examine the change first. Removing a guard is its own piece of
  work, with its own task, and never a side effect of the change it was refusing.
- **A guard must be mutation-tested.** A check that cannot fail proves nothing: verify it catches
  the violation it claims to catch, then revert the violation.
- **Guards describe boundaries, not style.** Formatting belongs to fmt and clippy.
- **A guard about what the code does reads the code, not its bytes.** `source.contains("Mutex")` is
  a claim about text, and it is wrong in both directions: it fails on a doc comment that mentions a
  lock, and it clears a struct holding a `RefCell`. Parse instead. See
  [`docs/decisions/004-a-rule-about-code-parses-code.md`](../../docs/decisions/004-a-rule-about-code-parses-code.md),
  which still holds; the guard it was written about is gone, so nothing here parses Rust today.

## Why the publish guard counts members instead of naming them

`only_the_held_names_are_publishable_to_a_registry` checks every package `cargo metadata --no-deps`
reports, then compares only the *number* of them against the members the workspace manifest lists.
The count is there so a metadata read that silently returned fewer crates cannot let the guard
claim it saw them all.

Do not upgrade that to a name-by-name comparison. `workspace_members()` resolves each member path
to the `[package].name` that member's own manifest declares (task 030), so the names it reports are
the packages Cargo knows, whatever the directories are called. What stops the upgrade is no longer
the accuracy of those names: widening what a guard asserts is its own decision, with its own task,
and never a side effect of a change that made a name correct. The count already carries the
coverage guarantee this guard was written for. If the directory-naming convention is ever worth
enforcing, it earns its own named guard and a stated intent, not a silent clause in this one.

The publish allowlist is the one place that guard compares names: the source-controlled
`HELD_NAMES` in `tests/workspace_guards.rs` is checked against the package names `cargo metadata`
reports, never against the member paths, so it still asserts nothing about layout. The check is
there because the count agrees with itself whether or not the list is honest — an entry naming a
crate that was renamed or deleted is invisible to everything else the guard does, and it would hand
its publish exemption to whatever takes that name next.

## Testing

`cargo test -p plasmosome-guards`.

**Count the tests by running them, not by grepping for them.** The fixtures in this crate are
Rust-shaped and git-shaped on purpose, so a grep for `#[test]` counts the fixtures too. `cargo
test` prints the number that is true.
