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

The same limit has a second side. A name is matched on its spelling, so an identifier that is one
of the construct words but means something else — an enum variant `SyncPoint::Barrier`, a grid
`Cell`, a file-local `use crate::registry::Handle as Weak;` — is reported as a use it is not. An
unresolvable name is either missed or over-reported, and this rule chooses to over-report: a wire
file that has to carry such a word narrows the construct list. That choice is pinned by
`a_name_that_is_a_construct_word_but_means_something_else_is_over_reported`. None of the seven wire
files carries such a word today.

A third case is refused rather than missed. A `mod name;` keeps its body in another file, so a scan
of the declaring file cannot see it — the rule reports the declaration and fails, naming the remedy
(add that file to `wire_sources`, or inline the module). This one is decidable by the parser, so it
is closed rather than documented as a limit: `out_of_line_modules` answers it, and the rule asks
before it scans.

## What the publish rule assumes

`no_workspace_crate_is_publishable_to_a_registry` cross-checks the packages `cargo metadata`
reports against `workspace_members()`, which reads the member *paths* out of the workspace
manifest and takes each one's last path component as the crate name. A member whose directory is
not named exactly like its package therefore reads as a mismatch. That is a false alarm, but it
fails closed and prints both lists, and `testkit_is_dev_only` fails beside it for the same
reason. If a crate ever needs a directory name that differs from its package name, resolve the
name through `cargo metadata` instead of the path — do not weaken the cross-check, which is what
lets the rule claim it saw every member.

## Testing

`cargo test -p plasmosome-freeze-checks`.

**Count the tests by running them, not by grepping for them.**
`grep -c '^#\[test\]' tests/shared_memory_reads_rust.rs` answers 26; the file holds 25 tests. The
extra hit is inside the raw-string fixture belonging to `a_test_name_naming_a_lock_is_not_a_use`,
which is a `#[test]` written as text so the rule can be shown ignoring it. A count taken that way
is off by exactly the fixtures this crate exists to distinguish from code, and the same is true of
any file here: the fixtures are Rust-shaped on purpose. `cargo test` prints the number that is
true.
