---
id: 034
title: A mock lives inside the plasmid that owns the hosts, and a spec records what was delivered
status: in_review
priority: 2
specs: [001]
intents: [003, 004, 009, 012]
refs:
  [
    docs/specs/001-control-protocol.md,
    docs/specs/008-cell-recovery-contract.md,
    crates/plasmosome-core/src/manifest.rs,
    crates/plasmosome-core/src/protocol.rs,
    crates/plasmid-sdk/src/lib.rs,
    tasks/024-the-dependency-freeze-reads-text-not-toml.md,
  ]
done_when: >-
  `PlasmidManifest::parse` refuses a manifest carrying `[mock]` with no
  `[network]`, one whose `[mock]` names a host its `[network]` does not, and one
  whose `[mock] hosts` is not an array of strings — a bare scalar, or an array
  holding a non-string — each refusal naming what is missing. The third is not
  covered by the first two: a malformed `hosts` reads as no hosts at all, so the
  host-by-host comparison has nothing to walk and the manifest passes while
  standing in for nothing; no manifest, fixture, spec example or
  crate doc declares a plasmid whose whole content is a `[mock]` section — that
  is a claim about the whole tree and not about `refs:` alone, so it is checked
  with `grep -rn 'mock-github' --include='*.toml' --include='*.wit' .` returning
  nothing and every surviving `[mock]` sitting beside a `[network]` in the same
  manifest, `mock-github` remaining only as the `MOCK_WITHOUT_NETWORK` fixture
  that proves the refusal; spec
  001 §3.10 states that refusal and its code `104` resolutions name only
  resolutions still reachable once no plasmid is a mock; D2b's three propagation
  rules and its safety-wins clause are all still stated in §3.5 and §3.10; spec
  001 describes itself as a record of what was delivered rather than as a text
  that may not be edited, its §6 still carrying six items in the same order with
  the same claims, every §5 RESERVED item still reserved, and the
  `plasmosome-freeze-checks` crate untouched.
pr: 63
evidence:
---

## Why

Two rulings by the owner, both about things spec 001 got wrong.

**A mock is not a plasmid.** `[mock]` is already a section of every plasmid manifest, parsed into
`MockSpec` by `crates/plasmosome-core/src/manifest.rs`. A plasmid whose entire content is that one
section — `mock-github` — therefore buys nothing and costs a second copy of the `hosts` list it
stands in for, free to drift from the `[network]` hosts in the plasmid it mocks. The mock belongs
in `github-pr`, next to the hosts it answers for, and the parser is what holds it there. This
descends from intent 010, *plasmids anyone can write*: a mock an author writes inside their own
plasmid is one fewer thing that author has to know about how the kernel resolves a closure.

**There are no frozen specs, only delivered ones.** Spec 001 calls itself "the P1 freeze" and
carries a §6 "Freeze checklist (what makes this 'frozen')". A spec records what was delivered; when
its design is wrong it is corrected in place, rather than kept intact under an amendment layered on
top. The first ruling is the case in point: the mock plasmid is wrong, and correcting it should not
need a ceremony.

`intents:` is copied from spec 001, which this branch fills in for the first time: `003` (cells
come back after a crash — §4's generation-numbered desired-state push, §3.10's attach receipts
carrying the ledger generation, and §6 item 6's replayable-from-log ledger, all of which mean
nothing unless the controller restarts), `004` (removing a capability removes exactly that one —
§3.11 in wire form), `009` (a command line an agent can drive — the protocol is the only control
surface, and what the CLI speaks) and `012` (a capability exists only while it is needed — the
attach and detach lifecycle). Those were always the goals the control protocol served; nobody had
written them down. Intent 010 is where the first ruling comes from, and it is not among them: a
mock an author writes inside their own plasmid is squarely *plasmids anyone can write*, but the
control protocol as a whole is not, so recording 010 on spec 001 would map the spec to a goal it
does not serve.

## Plan

The deliverable, in one sentence: `mock-github` stops existing anywhere, a manifest that tries to
be one is refused, spec 001 shows a mock declared inside `github-pr`, and spec 001 stops claiming
to be unrevisable.

**Out of scope.** The `plasmosome-freeze-checks` crate, its tests, and the must-not-bake-in CI
rules — that is a different sense of the word, invariants pinned in code rather than a spec
claiming immunity from revision. `docs/decisions/` is not edited: a decision is never edited, and
the two that lean on spec 001's freeze language reach conclusions that still hold. The plasmid WIT
world and every other RESERVED item in §5 stay reserved. No fourth plasmid replaces `mock-github`;
the P1 set is `model-provider`, `workspace-bind`, `github-pr`. `MockSpec` keeps its `hosts` field:
removing one is a manifest-grammar change, and the refusal below makes the duplicate harmless.

Read the files in `refs:` and edit nothing beyond them. The one thing that reaches wider is a
read: the sweep for a mock-only plasmid in `done_when` is a claim about the whole tree, and
`refs:` names the files this change edits rather than every place such a declaration could hide,
so that one is greped repo-wide and its command is written out above. Two of the refs are
downstream of the rewrite rather than subjects of it: `docs/specs/008-cell-recovery-contract.md` and
`tasks/024-the-dependency-freeze-reads-text-not-toml.md` each quote spec 001's freeze language and
need the quote refreshed.

| Test | What it proves |
| --- | --- |
| `a_plasmid_carries_its_own_mock_alongside_the_hosts_it_stands_in_for` | a manifest declaring `[network]` and `[mock]` parses both, and the mock's hosts are the hosts that same file declares |
| `a_manifest_whose_whole_content_is_a_mock_is_refused` | the ruling binds the parser, not only the prose: a mock-only manifest no longer parses, and the refusal names the `[network]` it lacks |
| `a_mock_naming_a_host_its_own_manifest_does_not_declare_is_refused` | the two host lists cannot drift, because the drifted one is refused by the host that drifted |
| `every_application_error_serializes_its_code_and_structured_fields` | code `104` still carries `node`, `modes`, `plasmids`, `resolutions`, over a conflict built from two plasmids that both exist |

Done when the `done_when` above reads true and the gate in the root `AGENTS.md` is green, each
exit code read bare rather than through a pipe.

STOP when done — do not start the next piece of work.

## Notes

**A malformed `[mock] hosts` was accepted in silence, and the asymmetry is what made it easy to
miss.** `string_list` returns an empty vector for anything that is not an array and drops array
entries that are not strings, so `hosts = "api.github.com"` parsed to `MockSpec { hosts: [] }` and
`hosts = ["api.github.com", 443]` parsed to one host with the `443` gone. In `[network]` the same
typo is caught, but only incidentally, by the separate "declares [network] without hosts" check;
`[mock]` has no such check, because an empty mock host list is a legal subset of the network's. So
the identical mistake was refused in one section and accepted in the other, and the accepted one
produced a mock that stood in for nothing.

`declared_string_list` now reads that field strictly — absent is an empty list, anything that is
not an array of strings is a refusal naming the section and field. Two tests were written first and
watched fail against the old parser, one per shape. The pre-existing `string_list` callers in
`parse_network` are the same latent defect at two more call sites (`hosts`, and `pin_cidrs`, which
has no emptiness check behind it at all); they are untouched here because this change does not add
them, and they want their own task.
