---
id: 039
title: A network list declared as a scalar parses to nothing, and pin_cidrs fails open
status: done
priority: 2
specs: [001]
intents: [003, 004, 009, 012]
refs:
  [
    crates/plasmosome-core/src/manifest.rs,
    tasks/034-mocks-live-in-plasmids.md,
    docs/specs/001-control-protocol.md,
  ]
done_when: >-
  `PlasmidManifest::parse` refuses a `[network]` — top-level or under
  `[commands.commands.<id>]` — that is not a table at all, and one whose `hosts`
  or `pin_cidrs` is not an array of strings, each refusal naming the section and,
  where a field is at fault, the field; an explicit `pin_cidrs = []` still parses
  and pins nothing; and no plasmid in the tree declares either field as a scalar —
  checked with
  `grep -rnE "^[[:blank:]]*(hosts|pin_cidrs)[[:blank:]]*=[[:blank:]]*[^[[:blank:]]" --include='*.toml' --include='*.rs' --include='*.md' .`,
  which flags an assignment holding any TOML scalar — double-quoted, literal-quoted,
  numeric or boolean — by matching every value that does not open a list, and whose
  only hits are the five fixtures inside `manifest.rs` that exist to prove the refusal.
pr: 68
evidence: squash commit 96ad908 on main; parse_network refuses a non-table [network] and a hosts or pin_cidrs that is not an array of strings at both call sites, naming the section and field, and an explicit pin_cidrs = [] still parses
---

## Why

`pin_cidrs` is an egress restriction: it says which address ranges a plasmid's traffic may reach.

The lax reader behind it returned an empty list for anything that was not an array, so
`pin_cidrs = "140.82.112.0/20"` parsed clean to no pins at all — the author declared a
restriction and nothing was restricted, with no check anywhere behind it to notice.

Task 034 fixed the same defect in `[mock] hosts` and named this one as the residual it left
behind. This is that task.

## Plan

The deliverable, in one sentence: `parse_network` becomes fallible and reads `hosts` and
`pin_cidrs` through the strict `declared_string_list` helper task 034 introduced, so both of its
call sites — the top-level `[network]` and the per-command `[commands.commands.<id>.network]` —
refuse anything that is not an array of strings, naming the section and the field.

**Why reject rather than coerce.** The repo already decided this question for the identical typo
in `[mock]`, and refused it. Coercing a scalar to a one-element list here while `[mock]` rejects
recreates the asymmetry that hid this bug for a release, and coercion only handles the one
malformed shape it anticipates — a mixed-type array would still drop entries in silence.

**Why not reshape `NetworkSpec`.** The manifest grammar is frozen and the struct is built
literally by `plasmosome-testkit`. Parse-time rejection already gives the guarantee that matters.
An explicit `pin_cidrs = []` is a legal, deliberate state and must keep parsing; a non-empty type
would outlaw it to catch a typo the parser can name directly.

**Why not strengthen the lax `string_list` helper itself.** A useful refusal needs the plasmid id,
the section and the field for its message, which is exactly what the strict helper already takes —
so "strengthening the helper" means migrating callers one at a time. The remaining callers fail
closed, not open, and have no motivating defect. They are recorded in the Notes instead, which is
the precedent task 034 set.

**Out of scope.** `[network] ports`, which reads through a lax integer filter with the same shape;
the absent emptiness check on a command-level `hosts = []`; and every surviving `string_list`
caller. The top-level `"declares [network] without hosts"` check stays — it still owns the
genuinely absent and explicitly empty cases, and the scalar case now fails earlier with the
truthful type error.

Read `crates/plasmosome-core/src/manifest.rs` and the two files above it in `refs:`, and edit
nothing beyond the first.

| Test | What it proves |
| --- | --- |
| `a_pin_declared_as_a_bare_string_must_not_parse_to_no_pins_at_all` | the motivating regression: a declared pin can no longer parse to no pins |
| `a_pin_cidrs_holding_a_non_string_is_refused` | an entry that is not a string is refused, not dropped, so egress cannot widen in silence |
| `a_network_hosts_declared_as_a_bare_string_is_refused_as_a_type_error_naming_the_field` | the refusal names the malformed field, instead of reporting a typo as an absence |
| `a_command_network_hosts_declared_as_a_bare_string_is_refused` | the second call site, which had no check behind it at all, inherits the same refusal |
| `an_explicitly_empty_pin_cidrs_list_parses_and_pins_nothing` | the rejection targets malformed shapes, not the legal explicit-empty state |
| `a_network_section_declared_as_a_scalar_is_refused_as_a_type_error_not_an_absence` | the section one level above the field: a `[network]` that is not a table is a type error, no longer reported as missing hosts |
| `a_command_network_section_declared_as_a_scalar_is_refused` | the same section-shaped typo under a command, which had nothing behind it and parsed to a network restricting nothing |
| `a_command_pin_cidrs_declared_as_a_bare_string_is_refused` | the fourth cell the criterion claims, pinned by a test rather than by the shared helper alone |

Done when the `done_when` above reads true and the gate in the root `AGENTS.md` is green, each
exit code read bare rather than through a pipe.

STOP when done — do not start the next piece of work.

## Notes

**2026-09-01 — the four refusals were watched fail first.** All five tests were written against
the old parser before any edit. The scalar `pin_cidrs`, the mixed-type `pin_cidrs` and the scalar
command-level `hosts` each panicked on `unwrap_err()` over an `Ok`, and the panic output is the
evidence: `NetworkSpec { hosts: ["api.github.com"], ports: [443], pin_cidrs: [] }` for the
declared pin that vanished, `pin_cidrs: ["140.82.112.0/20"]` for the array whose `20` was dropped,
and `network: Some(NetworkSpec { hosts: [], ports: [443], pin_cidrs: [] })` for the command that
ended up with no host restriction. The scalar top-level `hosts` failed differently, as an
assertion mismatch rather than a panic: it already erred, but with
`plasmid github-pr declares [network] without hosts` — a typo reported as an absence, which is the
misleading message this change replaces. The fifth test passed before and after, by design.

**2026-09-01 — review added the section one level up, and the two refusals were watched fail
first.** Both reviews landed on the same shape: a `network` declared as a scalar rather than a
table. `toml::Value::get` returns `None` for a non-table receiver, so all three fields took their
absent default and `parse_network` returned `Ok`. The command-level test panicked on `unwrap_err()`
over an `Ok` carrying `network: Some(NetworkSpec { hosts: [], ports: [], pin_cidrs: [] })` — a
command whose declared network restricted nothing. The top-level test failed as an assertion
mismatch on `Invalid("plasmid probe declares [network] without hosts")`, the same absence-for-a-type-error
report this change exists to stop emitting. A non-table guard at the top of `parse_network` closes
both call sites by name. `network_section_without_hosts_is_rejected` still passes: the emptiness
check keeps owning the genuinely absent and explicitly empty cases. The fourth criterion cell,
a command-level scalar `pin_cidrs`, passed before the guard and after it — both call sites already
share one `declared_string_list`, and the test exists so a future split of those paths goes red.

**2026-09-01 — the criterion's scan proved less than it claimed.** The old
`grep -rn 'pin_cidrs *= *"'` matched only double-quoted values, so `hosts = 'api.example.com'` and
`hosts = 5` were both invisible to it. The replacement matches any assignment whose value does not
open a list, over `*.toml`, `*.rs` and `*.md`. Its full output over the tree:

```text
crates/plasmosome-core/src/manifest.rs:687:hosts = "api.github.com"
crates/plasmosome-core/src/manifest.rs:713:pin_cidrs = "140.82.112.0/20"
crates/plasmosome-core/src/manifest.rs:731:hosts = "api.github.com"
crates/plasmosome-core/src/manifest.rs:761:hosts = "alpha.ak.local"
crates/plasmosome-core/src/manifest.rs:805:pin_cidrs = "10.29.0.0/24"
```

Five hits, every one a refusal fixture. The line anchor drops what the old wording listed as
expected prose hits in this task and task 034 — those mentions all sit mid-sentence in backticks,
not at the start of a line — and it drops the `capabilities = ["network:hosts=api.github.com"]`
false positive an unanchored pattern collects.

**Two residuals raised in review and deliberately deferred.**

_The refusal label is one `commands.` short of the header an author types._ A command network
section is labelled `[commands.git.network]`, while the literal TOML header is
`[commands.commands.git.network]`. This is the file's existing convention, not something this
change introduced: `manifest.rs` already emits `[commands.{id}]` in the "declares no exec" refusal
and in the secret-ref refusal. Making this one message literal would leave it inconsistent with
those two, and correcting all three is its own change with its own test updates.

_Manifest parse errors have no consumer mapping to wire code 108._ `ErrorCode::ManifestInvalid` and
`WireError::manifest_invalid` exist in `protocol.rs` and are tested there in isolation, but nothing
in the repo calls `PlasmidManifest::parse` or `load` outside `manifest.rs`'s own test module —
`lib.rs:32` only re-exports them. There is therefore no consumer boundary at which a
`ManifestError::Invalid` could be converted into a `108`. The gap is real and becomes live the day
a caller appears; it is not something this change introduces or can close, so it is deferred rather
than fixed here.

**Named residuals this task does not touch.** `[network] ports` still reads through a lax
`filter_map(as_integer)`, so a scalar or mixed-type `ports` still parses to fewer ports than were
declared — the same fail-open shape, in integers rather than strings. A command-level `hosts = []`
has no emptiness check, only the top-level one. And `string_list` survives at four call sites:
`requires.capabilities` and `provides.*.tools`, whose scalars yield fewer grants and fewer tools
and so fail closed and loud at use; `normalize_scope`, which only calls it behind an `is_array`
guard; and `exec`, which has its own named emptiness error.
