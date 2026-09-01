---
id: 034
title: A mock lives inside the plasmid that owns the hosts, and a spec records what was delivered
status: in_progress
priority: 2
specs: [001]
intents: []
refs:
  [
    docs/specs/001-control-protocol.md,
    crates/plasmosome-core/src/manifest.rs,
    crates/plasmosome-core/src/protocol.rs,
    crates/plasmid-sdk/src/lib.rs,
  ]
done_when: >-
  no manifest, fixture, spec example or crate doc declares a plasmid whose whole
  content is a `[mock]` section; spec 001 §3.10 shows a mock declared in the
  manifest of the plasmid that declares the hosts it stands in for, and its code
  `104` resolutions name only resolutions still reachable once no plasmid is a
  mock; spec 001 describes itself as a record of what was delivered rather than
  as a text that may not be edited, with every §5 RESERVED item still reserved
  and the `plasmosome-freeze-checks` crate untouched.
pr:
evidence:
---

## Why

Two rulings by the owner, both about things spec 001 got wrong.

**A mock is not a plasmid.** `[mock]` is already a section of every plasmid manifest, parsed into
`MockSpec` by `crates/plasmosome-core/src/manifest.rs`. A plasmid whose entire content is that one
section — `mock-github` — therefore buys nothing and costs a second copy of the `hosts` list it
stands in for, free to drift from the `[network]` hosts in the plasmid it mocks. The mock belongs
in `github-pr`, next to the hosts it answers for. This descends from intent 010, *plasmids anyone
can write*: a mock an author writes inside their own plasmid is one fewer thing that author has to
know about how the kernel resolves a closure.

**There are no frozen specs, only delivered ones.** Spec 001 calls itself "the P1 freeze" and
carries a §6 "Freeze checklist (what makes this 'frozen')". A spec records what was delivered; when
its design is wrong it is corrected in place, rather than kept intact under an amendment layered on
top. The first ruling is the case in point: the mock plasmid is wrong, and correcting it should not
need a ceremony.

`intents:` is `[]` because it is copied from the spec this task names, and spec 001 is the one
accepted spec that names no intent — the closed amnesty `.agents/skills/tasks` describes. Intent
010 is where the first ruling comes from; it is not the intent behind the whole control protocol,
so backfilling it into spec 001 would record something false.

## Plan

The deliverable, in one sentence: `mock-github` stops existing anywhere, spec 001 shows a mock
declared inside `github-pr`, and spec 001 stops claiming to be unrevisable.

**Out of scope.** The `plasmosome-freeze-checks` crate, its tests, and the must-not-bake-in CI
rules — that is a different sense of the word, invariants pinned in code rather than a spec
claiming immunity from revision. `docs/decisions/` is not edited: a decision is never edited, and
the two that lean on spec 001's freeze language reach conclusions that still hold. The plasmid WIT
world and every other RESERVED item in §5 stay reserved. No fourth plasmid replaces `mock-github`;
the P1 set is `model-provider`, `workspace-bind`, `github-pr`.

Files to read, and nothing beyond them: `docs/specs/001-control-protocol.md`,
`crates/plasmosome-core/src/manifest.rs`, `crates/plasmosome-core/src/protocol.rs`,
`crates/plasmid-sdk/src/lib.rs`.

| Test | What it proves |
| --- | --- |
| `a_plasmid_carries_its_own_mock_alongside_the_hosts_it_stands_in_for` | a manifest declaring `[network]` and `[mock]` parses both, and the mock's hosts are the hosts that same file declares — the drift a separate mock plasmid allowed |
| `every_application_error_serializes_its_code_and_structured_fields` | code `104` still carries `node`, `modes`, `plasmids`, `resolutions`, over a conflict built from two plasmids that both exist |

Done when the `done_when` above reads true and the gate in the root `AGENTS.md` is green, each
exit code read bare rather than through a pipe.

STOP when done — do not start the next piece of work.

## Notes
