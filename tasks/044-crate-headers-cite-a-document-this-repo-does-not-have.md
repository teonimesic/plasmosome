---
id: 044
title: two crate headers cite a document this repository does not have
status: in_review
priority: 2
specs: [001]
intents: [003, 004, 009, 012]
refs: [crates/plasmosome-membrane/src/lib.rs, crates/plasmosome-membrane/README.md, crates/plasmosome-ledger/src/lib.rs, AGENTS.md, .githooks/provenance-guard]
done_when:
  - No shipped crate source cites a document number this repository does not contain.
  - plasmosome-membrane's `//!` is a sentence or two naming what the crate is for and how a caller reaches it, with the background it carried moved to the crate README.
  - The rule the ledger header cited is stated in its own words rather than by reference, so removing the citation costs a reader nothing.
  - All five gate commands exit 0, reported as bare exit codes.
pr: 78
---

## Why

`plasmosome-membrane/src/lib.rs` and `plasmosome-ledger/src/lib.rs` each cite "86 §4" for a design
rule. There is no document 86 in this repository — `docs/specs/` runs 001 to 014 — so the citation
resolves nowhere for anyone reading the published crate docs, and it points at a numbering scheme
that belongs to a private corpus rather than to this tree.

`.githooks/provenance-guard` does not catch it. The guard greps three literal terms, so a reference
that names a private document by number rather than by organisation passes it unseen. The guard is
not wrong — it is narrower than the rule it enforces, and this is what that gap looks like in
practice.

Separately, membrane's `//!` had grown to twenty-two lines carrying readiness rationale, VMM
lifecycle notes and broker-supervision detail. `AGENTS.md` allows a crate root a sentence or two
saying what the crate is for and how a caller reaches it, and sends background to the README.

## Plan

Trim membrane's `//!` to four lines and move the readiness rule, the reap guarantee and the
division of labour into `crates/plasmosome-membrane/README.md` under two new sections, plus the
`brokers` and `daemon` rows its module table was missing. In the ledger header, state the rule —
durable state never lives only in the crashiest process — instead of citing it.

`F9`, `D1b` and `D1c` are left alone, with a distinction worth recording. Spec 001 §2 *defines*
D1b and D1c outright. `F9` it only *uses* — the readiness rule's content is stated in §4, so a
reader who follows the label lands on real in-repo text, but nothing here says what F9 is. That
makes it weaker than D1b/D1c and still stronger than `86 §4`, which resolves to nothing at all.

## Notes

Not done here, and left for the owner. The remaining `86 §4` citations, counted rather than
estimated:

| File | Count |
| --- | --- |
| `docs/specs/001-control-protocol.md` | 5 |
| `tasks/017-the-freeze-scan-matches-substrings-not-code.md` | 5 |
| `tasks/014-control-socket-answers.md` | 2 |
| `tasks/024-the-dependency-freeze-reads-text-not-toml.md` | 2 |
| `docs/decisions/004-a-rule-about-code-parses-code.md` | 1 |

An earlier draft of this record said "about nine task records", which was wrong twice: the count
was a guess, and the most significant holder is not a task record at all but an **accepted spec**.

The guard could also be widened to refuse a document reference this repository cannot resolve.
Both changes alter what CI rejects, so neither is a drive-by.
