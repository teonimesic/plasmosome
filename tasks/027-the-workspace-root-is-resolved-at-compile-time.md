---
id: 027
title: Repository-wide checks resolve the workspace root at compile time and can inspect the wrong tree
status: todo
priority: 2
specs: []
intents: []
refs: [crates/plasmosome-freeze-checks/src/lib.rs, crates/plasmosome-membrane/src/readiness.rs]
done_when: >-
  a regression test over a copied checkout, with the original still in place, proves that
  `plasmosome-freeze-checks` and `plasmosome-membrane` both read the tree they were invoked in
  rather than the one they were compiled in; and a stale `target/` fails by naming staleness
  instead of a missing file.
---

## Why

`workspace_root()` derives the repository root from `env!("CARGO_MANIFEST_DIR")`, a **compile-time**
constant baked into each test binary. Cargo's fingerprint does not invalidate on a directory
rename, so a stale `target/` keeps serving binaries carrying the old path.

The idiom is not confined to that helper. `plasmosome-membrane/src/readiness.rs` and
`plasmosome-freeze-checks/src/lib.rs` each spell out `PathBuf::from(env!("CARGO_MANIFEST_DIR"))`
followed by `.ancestors().nth(2)`, so a fix applied to one leaves the other wrong. Whatever
replaces it should be the single thing both call.

Renaming the checkout is the mild case: every check that reads a file through `workspace_root()`
fails with `No such file or directory`, which reads as a broken invariant rather than a stale
build. Ten checks failed that way after a move, on both `main` and a feature branch, and
`cargo clean` restored them.

**Copying is the case that matters.** The old path still exists and still holds a valid workspace,
so nothing errors — the checks pass while inspecting the tree the binary was compiled in rather
than the one cargo was invoked in. Every check in this crate is a check on the wrong repository,
and it reports green. That is the failure this crate exists to prevent, occurring in the crate
itself.

CI never sees either: a fresh checkout compiles at the path it runs from. This is a local-only
false green, which is what makes it easy to leave in place.

One candidate is to read `CARGO_MANIFEST_DIR` from the environment at runtime rather than through
`env!`. Under `cargo test` here it was present and did name the invocation:

```text
compile-time: …/attribution-guard-nonterminal/crates/plasmosome-freeze-checks
runtime     : Ok("…/attribution-guard-nonterminal/crates/plasmosome-freeze-checks")
```

**Do not take that as the answer.** Cargo documents these variables as build-time and does not
guarantee them in the environment of the process it runs, so the fallback to `env!` is not a rare
path — it is whatever happens when the binary is run any other way, and it is the stale value. A
mechanism that is right only when an unguaranteed variable happens to be set has moved the failure
rather than removed it.

Nor is asserting that the resolved root holds the workspace `Cargo.toml` enough. That catches the
checkout that **moved**, where the old path is gone. It does not catch the checkout that was
**copied**, where the old path still exists and still holds a perfectly good `Cargo.toml` — the
case that reports green while reading the wrong tree, and the reason this is filed at all.

So: pass the root in explicitly, or find a source that is guaranteed at runtime. Whatever is
chosen, the thing that proves it is a regression test over a **copied** checkout, asserting the
check reads the tree it was invoked in rather than the one it was compiled in. Without that test
this task cannot be called done, because its own headline failure would go unobserved.

## Evidence the stale binary is real

The repository moved from `~/Documents/plasmosome` to `~/Documents/plasmosome/plasmosome` while
PR #35 was open. In the worktree at
`~/Documents/plasmosome/plasmosome/.worktrees/attribution-guard-nonterminal`, with the pre-move
`target/` in place, `cargo test --workspace` failed once:

```text
panicked at crates/plasmosome-membrane/src/readiness.rs:130:13:
the control protocol spec is readable at
/<home>/Documents/plasmosome/.worktrees/attribution-guard-nonterminal/docs/specs/001-control-protocol.md:
No such file or directory (os error 2)
```

The path in the panic is the pre-move one; the file was present at the post-move path throughout.
`cargo clean -p plasmosome-membrane` alone, with no source change and the same command, took that
crate from 1 failed to 29 passed. Sources unchanged, command unchanged, worktree unchanged — only
the cached artifacts differed.

## Plan

## Notes

Found when the repository moved on disk during PR #35, which adds two checks that execute a shell
script located through `workspace_root()` — so the stale constant made them look for a guard that
was not there. Deliberately left out of that pull request: it changes a public helper every other
check in the crate uses, and it is a different unit from commit-trailer detection.
