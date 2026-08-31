---
id: 004
title: A freeze rule about what the code does parses the code, and says where parsing stops
date: 2026-08-31
status: accepted
---

## Context

`controller_wire_state_shares_no_memory_across_the_seam` held one of the 86 §4 rules: state
crossing the controller⇄supervisor seam moves as serde data, never as shared memory. It held it by
reading each wire file as one string and asserting the string did not contain any of nine
substrings — `Arc<`, `Rc<`, `Mutex`, `RwLock`, `UnsafeCell`, `thread_local`, `lazy_static`,
`once_cell`, `static mut`.

That is a different question from the rule. It was wrong in both directions, and both were
observed, not argued. A doc comment reading `/// This type holds no Mutex and no lock.` failed the
build with a message accusing the file of breaking the rule. A struct holding `RefCell<u32>`,
`Cell<u32>` and `AtomicUsize` passed, because none of those are spelled with the nine. So did a
lock imported under another name. Task 014's plan had already noticed the first half and told its
executor to keep those words out of `protocol.rs`, doc text included — a plan routing around a
check is the check asking to be fixed.

A guard that fires on a comment and clears a file that shares memory is worse than no guard,
because a green from it is believed.

## Decision

A freeze rule that makes a claim about what the code does reads the code as code. The
shared-memory rule now parses each wire file with `syn` and inspects only the positions where Rust
means a path, a type, an import, a macro or a `static`. Comments, doc blocks, test names and string
literals are not those positions, so prose about locks can no longer fail a build. Identifiers are
compared whole, so `CellRecord` is not `Cell`. The forbidden set moved from nine spellings to the
constructs themselves: reference-counted sharing, locks and guards, every cell type, every standard
atomic, raw pointers, `static mut`, and the crates that exist to supply these.

The rule follows the aliases a file declares about itself — `use std::sync::Mutex as Guard;` and
`type Guard = Mutex<u32>;` both make a later `Guard<u32>` a violation, reported under both names.
It does **not** follow an alias declared elsewhere, and that limit is written into the module
documentation, the crate's `AGENTS.md`, and a test that asserts the miss so it stays visible.

## Rejected

**Keep scanning text, with a longer list.** Adding `RefCell`, `Cell<`, `Atomic` and the rest closes
one example each and leaves the shape intact: still no way to tell a doc comment from a field, and
still one rename away from blind. It would also read as stronger than it is, which is the actual
harm.

**Strip comments and strings first, then scan.** This is a Rust lexer written badly. Raw strings,
nested block comments and doc attributes each want their own special case, and the miss half of the
problem is untouched.

**Shell out, as the neighbouring rule does.** Sixty lines away,
`controller_crates_have_no_dependency_path_to_a_vmm_or_netstack_crate` runs `cargo tree` instead of
parsing text, and the obvious question is why this rule does not do the same. Because a dependency
edge is a fact cargo already computes and prints; "which type does this identifier resolve to" is a
fact only the compiler holds and does not expose on any stable surface. There is no `cargo` command
that answers it.

**Resolve names properly, with rustc or rust-analyzer.** This would close the remaining miss, and
it is the only thing that would. It costs a nightly compiler driver or a rust-analyzer dependency,
run over the workspace on every test run, to catch a case nobody has written yet. Not now. It is
recorded as the price of closing the limit, so the next person does not have to rediscover it.

## Consequences

`plasmosome-freeze-checks` now depends on `syn` and `proc-macro2`. Both were already in
`Cargo.lock` through `serde_derive`; the change enables `syn`'s `full` and `visit` features. The
crate ships nothing, so this reaches no product binary.

A wire file that does not parse is a rule failure, not a silent pass. Files under this rule must
stay valid Rust as `syn` understands it, which is the same constraint the compiler already applies.

The remaining miss is real and named: a lock re-exported under a local name by a sibling module,
imported here, is not found. Closing it is name resolution and needs a compiler. Anyone extending
this rule should widen the construct list rather than reach for a regex, and anyone tempted to trust
its green on the alias case should read the test that pins the limit.

That miss has a second side, accepted with it. Because a name is matched on its spelling, an
identifier that is one of the construct words but means something else — an enum variant
`SyncPoint::Barrier`, a grid `Cell`, a file-local rename onto `Weak` — is reported as a use it is
not. It is one limit, not two: an identifier this rule cannot resolve is either missed or
over-reported, and the choice of which is the only freedom a parser has. This rule over-reports,
because a false positive argues with a person who can narrow the construct list, while a false
negative is a green that gets believed. Neither side is closed by a longer list; both close only
with name resolution. A wire file that must carry such a word narrows the list, and both sides are
pinned by a test.
