---
id: 003
title: An object's owner is a plasmid in a cell, not a plasmid
date: 2026-08-31
status: accepted
---

## Context

`OsObject` is `{ class, key, owner: PluginId }`, and `crates/plasmosome-backend` has no notion of
a cell anywhere in it. So the snapshot of the world cannot say which cell an object belongs to —
only which plasmid asked for it.

That is fine while a plasmid can only ever be attached once. It stops being fine the moment the
same plasmid is attached to two cells of one instance, which the control protocol plainly allows.
Spec 001 §2 settles it: addressing "chains down `(kernel, cell, plasmid)`", so a plasmid name is
scoped by the cell it is attached to and is unique only within one. The verb schemas carry that
through — `plasmid.list`, `plasmid.add`, `plasmid.remove` and `plasmid.reload` each take kernel,
cell and plasmid as separate parameters (§3.9–3.12) — and nothing in the frozen set asserts
uniqueness above the cell. A plasmid is an attachment to a cell, not a thing an instance has one
of.

Spec 008's recovery work is where this stopped being theoretical. Quarantine reports every object
"owned by a plugin the ledger names", and recovery unions expected state across adopted cells —
both of which need to answer *which cell*, and neither can.

## Decision

**An object's owner names the cell.** `OsObject`'s ownership becomes a plasmid together with the
cell it is attached to, rather than a plasmid alone.

The alternative is to make a plasmid name unique across an instance — either by declaring it so,
or by refusing the second attach. Both are rejected below. The short reason is that neither can
express something the protocol explicitly supports: `plasmid.add` carries a `mock` mode per
attachment, and an absent mode means "the cell's inherited default", so the same plasmid may
legitimately run `simulate` in one cell and `passthrough` in another. Under a uniqueness rule
those are either the same owner or one of them is refused outright.

It also undercuts decision 002. That decision chose one ledger per cell so a corrupt file costs
one cell; ownership that cannot name a cell means quarantine cannot say which cell's objects it
found, which gives back the containment the file layout was chosen for.

## Rejected

**A globally unique `PluginId` per instance, declared.** Simpler, no churn. The type would assert
what the protocol does not — that a plasmid name identifies one attachment — and the only way to
keep the assertion while addressing two cells is a mangled name like `github-pr@cell-1`, cell
identity smuggled into a string the type system cannot see or check. Its failure mode is quiet,
which is what makes it the dangerous one. Ownership that cannot name a cell cannot attribute: the
objects of two cells land in one flat `OsState` with nothing to partition them by, so no report
can say which cell it is describing. That holds universally, however the keys differ. Where the
two cells also share a class and key — a proxy map for the same host, a mount on the same target
— the objects deduplicate outright, and the drift between those cells is not merely unattributed
but gone.

**A globally unique plasmid name per instance, enforced.** The stronger form, and the one that
will come back: the controller refuses a `plasmid.add` naming a plasmid already attached to
another cell, so uniqueness is a constraint the runtime maintains rather than a claim the type
makes. It does not fail quietly, and it deserves a straight answer rather than the previous
paragraph's. It loses on what it costs. It removes the per-cell mock mode §3.10 offers —
simulating a plasmid in one cell while another passes through is a use of the protocol, not an
accident of it — and refusing that attach needs an error code, in a §1 table that is closed. So
it is a change to the frozen protocol rather than a way of avoiding one, and it buys that change
by giving up a capability. Widening a wire type that nothing durable has written yet is the
cheaper side of that trade.

**Leaving ownership alone and disambiguating at the call site.** Every consumer would carry the
cell separately and remember to pair it with the owner. That is the same information with more
places to get it wrong, and `OsState` is a set with no cell dimension — the objects are already
unattributable by the time any call site sees them.

**Deferring until a second real backend exists.** Tempting, since nothing today attaches one
plasmid twice. But spec 008 has to be written against some model now, and writing it against the
wrong one means rewriting recovery rather than extending it.

## Consequences

The change is shallow — a type widening, with no algorithm to redesign — but it is wider than
`OsObject`, and four surfaces carry it:

- **`OsObject` and `UniverseOp`.** The owner field, and every constructor that fills it. Both are
  frozen wire types, `wire_serde` over each in `plasmosome-freeze-checks`, so the serde shape
  changes. Nothing durable exists in the old shape, so there is no migration; the freeze
  checklist is still a contract and this touches it.
- **Where the cell type lives.** `CellId` is in `plasmosome-core`, which depends on
  `plasmosome-backend`, so the backend cannot name it without a cycle. It moves down to the
  backend or is mirrored there — the same carve-out spec 008 already makes for `MockMode`.
- **Where the owner comes from.** `Grant` and `LedgerEntry` carry a `PluginId` and no cell, and
  an object's owner is built from `grant.plugin`. A cell has to reach that point: either those
  types grow one — they are frozen wire types too — or every construction site supplies it. This
  is the largest of the four and the one to size before committing to the rest.
- **`OsState`'s accessors, and the tests that assert owner identity.** `owner_of` returns
  `Option<PluginId>` and `remove` resolves an owner through it. The owner tests follow,
  `a_change_of_owner_is_both_a_loss_and_a_leak` among them, whose meaning becomes sharper rather
  than different: the same plasmid in a different cell now genuinely is a different owner.

Spec 008 is unblocked by this and needs a small follow-up: `found` and the union behind
`expected` restated in terms of the decided identity, plus acceptance cases for two cells sharing
a plasmid.

One claim in the argument that led here did not survive checking, and is recorded so nobody
repeats it: spec 001 §3.3's example does **not** show one plasmid in two cells — its `cell-2`
carries an empty plasmid list. The case for this decision rests on the addressing chain, the verb
signatures and the per-attachment mock mode, not on that example.
