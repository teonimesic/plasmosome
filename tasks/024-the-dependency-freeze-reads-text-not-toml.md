---
id: 024
title: The dependency freeze reads text, not TOML
status: todo
priority: 2
specs: []
intents: []
refs:
  [
    crates/plasmosome-freeze-checks/tests/freeze_rules.rs,
    tasks/017-the-freeze-scan-matches-substrings-not-code.md,
  ]
done_when: >-
  the controller dependency check resolves each declaration to the package it
  actually pulls in, and looks in every table that can pull one in. Adding
  `sneaky = { package = "libc", version = "0.2" }` to `plasmosome-core`, or
  `"libc" = "0.2"`, or `nix` under `[target."cfg(unix)".dependencies]`, or
  `libc` under `[dev-dependencies]` or `[dependencies.libc]`, fails the check; a
  test covers each of the five. The manifests as they stand today still pass.
pr:
evidence:
---

## Why

`controller_crates_declare_no_fork_or_socketpair_plumbing_dependency` is the guard that keeps
`fork`/`socketpair` plumbing out of the controller. It reads the manifest as lines of text and
takes whatever sits left of the first `=` as the dependency name, so it compares declaration keys
rather than packages. Three ways past it:

- **An alias.** `sneaky_libc = { package = "libc", version = "0.2" }` records `sneaky_libc`. The
  crate depends on `libc`; the check never sees the word.
- **A quoted key.** `"libc" = "0.2"` records `"libc"`, quotes included, which matches nothing in
  `FORBIDDEN_DIRECT_DEPENDENCIES`.
- **Another table.** `declared_in` starts at `[dependencies]` and stops at the next `[`, so
  `[target."cfg(unix)".dependencies]`, `[dev-dependencies]` and `[build-dependencies]` are not
  read at all.
- **A sub-table**, which needs no alias at all. `in_section` is set by `line == section`, so a
  plain `[dependencies.libc]` header ends the section and declares the dependency in one line.

Confirmed against `main`. Cargo refuses several of these at once, so each was tested alone by
adding it to `crates/plasmosome-core/Cargo.toml`: the alias, the quoted key, the
`[target."cfg(unix)".dependencies]` table with `nix`, `[dev-dependencies.libc]` and
`[dependencies.libc]`. All five leave the freeze suite green, while the control — plain
`libc = "0.2"` under `[dependencies]` — fails it, so the check is not vacuous:

```
sneaky_libc = { package = "libc", version = "0.2" }   5 passed; 0 failed
"libc" = "0.2"                                        5 passed; 0 failed
[target."cfg(unix)".dependencies] nix = "0.29"        5 passed; 0 failed
[dev-dependencies.libc]                               5 passed; 0 failed
[dependencies.libc]                                   5 passed; 0 failed
libc = "0.2"          (control, under [dependencies]) 4 passed; 1 FAILED
```

The companion `cargo tree` test does not cover the gap: it matches
`FORBIDDEN_CRATE_FRAGMENTS`, which lists VMM and netstack crates and no plumbing crate — not
`libc`, `nix` or `rustix`. So a controller crate can take a direct dependency on any of the three
and the whole suite stays green.

`toml` is already a workspace dependency, and `cargo metadata` reports the resolved package name
and the rename for every dependency in every table. Either is enough; parsing the manifest by
hand a second time is not.

`workspace_members` in the same file is the same shape of mistake and worth taking in the same
pass: it looks for a literal `members = [` line and strips quotes by hand, held up by an assertion
that the root manifest still lists its members one per line. Whichever parser this task brings in
covers both.

**Not the same as task 017.** That one is the shared-memory scan reading whole-file text
(`SHARED_MEMORY_PATTERNS`), and its branch rewrites that check alone — `declared_in` and
`FORBIDDEN_DIRECT_DEPENDENCIES` are untouched by it. The two land in the same file, so whoever
takes this one rebases after 017 merges. The shape of the complaint is the same in both, which is
the more interesting fact: these guards assert on bytes and are named for what the code does.

**Where it came from.** CodeRabbit raised the alias case on PR #8 as an "outside diff range"
comment in the review body. Those never become review threads, so the PR merged with zero
unresolved threads and nothing to answer.

## Plan

## Notes
