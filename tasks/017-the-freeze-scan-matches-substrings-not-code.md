---
id: 017
title: The freeze scan matches substrings, not code
status: in_progress
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

1. **Record both failures first.** On the current check, plant a doc comment naming `Mutex` in a
   listed wire file and run `cargo test -p plasmosome-freeze-checks` — expect a red build blaming
   the file for breaking 86 §4 rule 2. Revert, plant a real `RefCell` field and an aliased lock
   (`use std::sync::Mutex as Guard;` then `Guard<u32>`), and run again — expect green. Paste both
   outputs into `## Notes`.

2. **Move the scan into the library and parse Rust.** Add a `shared_memory` module to
   `plasmosome-freeze-checks/src/` that takes a source string and returns the shared-memory uses it
   finds, each naming the construct and the line. It parses with `syn` (already in `Cargo.lock`
   through `serde_derive`; the change enables its `full` and `visit` features) and inspects only
   positions where Rust means a type or an item:

   - type paths and raw-pointer types,
   - `use` trees,
   - macro invocation paths,
   - `static` items with `mut`.

   Comments, `///` doc attributes and string literals are not in that set, so no amount of prose
   about locks can fail the build. That closes the first failure mode by construction rather than
   by a longer list of exceptions.

3. **Replace the nine spellings with categories.** Reference-counted sharing (`Arc`, `Rc`), locks
   and their guards (`Mutex`, `RwLock`, `ReentrantLock`, `Condvar`, `Barrier`, `OnceLock`,
   `LazyLock`), interior mutability (`Cell`, `RefCell`, `UnsafeCell`, `SyncUnsafeCell`, `OnceCell`,
   `LazyCell`), every type whose name starts with `Atomic`, raw pointers, the `thread_local!` and
   `lazy_static!` macros, the `once_cell` crate, and `static mut`.

4. **Resolve the aliases a single file can prove.** Two forms are declared in the file being read
   and are therefore decidable without a name resolver: `use std::sync::Mutex as Guard;` and
   `type Guard = Mutex<u32>;`. Both add their local name to the forbidden set for that file, so a
   later `Guard<u32>` is caught and the message names both the alias and what it aliases.

5. **Write down the limit instead of pretending there is none.** An alias declared in *another*
   module or crate — `pub use std::sync::Mutex as Lock;` in a sibling file, then `use crate::Lock;`
   here — is still missed. Catching it needs name resolution across the crate graph, which is
   rustc's or rust-analyzer's job and not something this repo can run from a test. There is no
   `cargo tree` equivalent here: `cargo tree` works for rule 1 because a dependency edge is a fact
   cargo already computes, whereas "which type does this identifier resolve to" is a fact only the
   compiler holds and does not expose. Record this in the module's `///` documentation, in the
   crate's `AGENTS.md`, and in `## Notes`.

6. **Both directions get a test.** Unit tests in the library over source strings: prose, a test
   name, a string literal and an error message naming a lock all pass; a `RefCell` field, an
   `AtomicUsize`, a `Cell<u32>`, a `use`-renamed lock and a `type` alias all fail with a message
   naming the file and the construct. A test also pins the known miss, so the limit is visible in
   the suite rather than only in a document.

7. **Re-run the two plants from step 1** against the new check and record the outputs beside the
   old ones. Then the gate: `cargo test --workspace`, `cargo clippy --workspace --all-targets --
   -D warnings`, `cargo fmt --all -- --check`, `./.githooks/provenance-guard`,
   `./.githooks/attribution-guard`.

## Notes

Both failure modes were planted in a real listed file
(`crates/plasmosome-core/src/state.rs`) and run against the old check and the new one. Every line
below is copied from the run, not reconstructed.

### 1. Prose fails the build

Planted: `/// This type holds no Mutex and no lock.` above `pub struct InstanceName(String);`.

Before — `cargo test -p plasmosome-freeze-checks --test freeze_rules controller_wire_state`:

```
test controller_wire_state_shares_no_memory_across_the_seam ... FAILED

86 §4 rule 2 broken: `crates/plasmosome-core/src/state.rs` uses `Mutex`; controller⇄supervisor state moves only as serde types, never as shared memory
```

After, same plant:

```
test controller_wire_state_shares_no_memory_across_the_seam ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out
```

### 2. Real shared memory passes

Planted: `use std::cell::Cell; use std::cell::RefCell; use std::sync::atomic::AtomicUsize;` and

```rust
pub struct PlantedSharing {
    pub seen: RefCell<u32>,
    pub hits: Cell<u32>,
    pub count: AtomicUsize,
}
```

Before:

```
test controller_wire_state_shares_no_memory_across_the_seam ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out
```

After:

```
86 §4 rule 2 broken: `crates/plasmosome-core/src/state.rs` uses `Cell`, `RefCell`, `AtomicUsize`, `RefCell` in `PlantedSharing`, `Cell` in `PlantedSharing`, `AtomicUsize` in `PlantedSharing`; controller⇄supervisor state moves only as serde types, never as shared memory
```

### 3. A lock renamed inside the file

Planted: `use std::sync::RwLock as Registry;` and a `Registry<u32>` field. The old check caught
this one by accident — the word `RwLock` is still on the import line — so it is not evidence of a
miss, but the new message says what was actually found:

```
86 §4 rule 2 broken: `crates/plasmosome-core/src/state.rs` uses `Registry`, a local alias for `RwLock`, `Registry`, a local alias for `RwLock` in `PlantedSharing`; controller⇄supervisor state moves only as serde types, never as shared memory
```

### 4. The limit, planted and still missed

Planted: a sibling module `crates/plasmosome-core/src/task017_aliases.rs` holding
`pub use std::sync::RwLock as Registry;`, then in `state.rs` `use crate::task017_aliases::Registry;`
and a `Registry<u32>` field. Before **and** after:

```
test controller_wire_state_shares_no_memory_across_the_seam ... ok
```

Nothing in `state.rs` says `Registry` is a lock. Deciding that is name resolution, and it needs a
compiler — rustc's own resolver or rust-analyzer — not a parser. There is no `cargo` command that
answers "what does this identifier resolve to", which is why `cargo tree` has no equivalent here
even though the neighbouring rule 1 check uses it: a dependency edge is a fact cargo already
computes and prints. This limit is stated in the module's `///` documentation, in
`crates/plasmosome-freeze-checks/AGENTS.md`, in
`docs/decisions/004-a-rule-about-code-parses-code.md`, and pinned by the test
`an_alias_declared_in_another_file_is_missed_because_that_needs_name_resolution` so it stays
visible in the suite rather than only in a document.

### What was not done

The construct list is wide but finite — reference-counted sharing, locks and guards, cell types,
the standard atomics, raw pointers, `static mut`, and the crates that supply these. A shared-memory
type outside it is missed. That is a list to extend, not a shape to replace.

Task 014's plan, which told its executor to keep those nine words out of `protocol.rs` "tests and
doc text included", no longer needs to route around the check. Task 014 is `done`, so its plan was
left as the record of what was true at the time.

### Gate

```
cargo test --workspace                                  all green, 16 new tests in
                                                        shared_memory_reads_rust.rs
cargo clippy --workspace --all-targets -- -D warnings   clean
cargo fmt --all -- --check                              clean
./.githooks/provenance-guard                            provenance-guard: clean
./.githooks/attribution-guard                           attribution-guard: clean
```
