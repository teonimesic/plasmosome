---
id: 039
title: A network list declared as a scalar parses to nothing, and pin_cidrs fails open
status: in_review
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
  `[commands.commands.<id>]` — whose `hosts` or `pin_cidrs` is not an array of
  strings, each refusal naming the section and the field; an explicit
  `pin_cidrs = []` still parses and pins nothing; and no plasmid in the tree
  declares either field as a scalar — checked with
  `grep -rn 'pin_cidrs *= *"' --include='*.toml' --include='*.rs' .` and the same
  for `hosts`, whose only remaining hits are the fixtures inside `manifest.rs`
  that exist to prove the refusal, and prose in this task and task 034.
pr: 68
evidence:
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

**Named residuals this task does not touch.** `[network] ports` still reads through a lax
`filter_map(as_integer)`, so a scalar or mixed-type `ports` still parses to fewer ports than were
declared — the same fail-open shape, in integers rather than strings. A command-level `hosts = []`
has no emptiness check, only the top-level one. And `string_list` survives at four call sites:
`requires.capabilities` and `provides.*.tools`, whose scalars yield fewer grants and fewer tools
and so fail closed and loud at use; `normalize_scope`, which only calls it behind an `is_array`
guard; and `exec`, which has its own named emptiness error.
