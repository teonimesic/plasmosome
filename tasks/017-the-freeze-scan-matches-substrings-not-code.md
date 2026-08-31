---
id: 017
title: The freeze scan matches substrings, not code
status: todo
priority: 2
specs: []
intents: []
refs:
  [
    crates/plasmosome-freeze-checks/tests/freeze_rules.rs,
    docs/decisions/,
  ]
done_when: >-
  the shared-memory check reads Rust rather than whole-file text: a doc comment
  or a string naming a forbidden type does not fail the build, and a use of one
  under a different spelling does. Both directions have a test.
pr:
evidence:
---

## Why

`controller_wire_state_shares_no_memory_across_the_seam` in
`crates/plasmosome-freeze-checks/tests/freeze_rules.rs` reads each listed file as a string and
asserts `!source.contains(pattern)` for each of nine patterns. It never parses anything, so it is
wrong in both directions.

**It fails on text that is not code.** Writing `/// This type holds no Mutex and no lock.` in a
doc comment fails the build, with a message that accuses the file of breaking 86 §4 rule 2. So
does a test name, a string literal, or an error message that mentions one of the nine words. The
rule the check exists to hold is about what the code links; the check is about what the bytes
spell. The gap is not theoretical: task 014's plan carries a warning telling its executor that
none of those words may appear anywhere in `protocol.rs`, "tests and doc text included" — the
plan had to route around the check.

**It misses shared memory not spelled with its nine tokens.** The patterns are `Arc<`, `Rc<`,
`Mutex`, `RwLock`, `UnsafeCell`, `thread_local`, `lazy_static`, `once_cell`, `static mut`. A wire
type holding a `RefCell`, a `Cell<u32>`, an `AtomicUsize`, or an alias — `use std::sync::Mutex as
Guard;` and then `Guard<u8>` — passes. A re-export under a local name defeats it completely.

Pre-existing: the check has worked this way since it was written. PR #14 only added
`crates/plasmosome-core/src/protocol.rs` to its file list, which is what put a fresh set of doc
comments under it.

## Plan

## Notes
