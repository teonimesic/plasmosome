---
id: 044
title: two crate headers cite a document this repository does not have
status: in_review
priority: 2
specs: [001]
intents: [003, 012]
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

`F9`, `D1b` and `D1c` are left alone: those are this repository's own vocabulary, defined in
`docs/specs/001-control-protocol.md`.

## Notes

Not done here, and left for the owner: about nine task records under `tasks/` carry the same
"86 §4" citation, and the guard could be widened to refuse a document reference this repository
cannot resolve. Both change what CI rejects, so neither is a drive-by.
