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
same plasmid is attached to two cells of one instance, which the control protocol plainly allows:
every plasmid verb is addressed by kernel, cell and plasmid together (`plasmid.list`,
`plasmid.add`, spec 001 §3.10–3.12), and `plasmid.add` takes the cell and the plasmid as separate
parameters. A plasmid is an attachment to a cell, not a thing an instance has one of.

Spec 008's recovery work is where this stopped being theoretical. Quarantine reports every object
"owned by a plugin the ledger names", and recovery unions expected state across adopted cells —
both of which need to answer *which cell*, and neither can.

## Decision

**An object's owner names the cell.** `OsObject`'s ownership becomes a plasmid together with the
cell it is attached to, rather than a plasmid alone.

The alternative — declaring a `PluginId` unique across an instance — is rejected. It contradicts
the frozen protocol's own shape, and it cannot express something the protocol explicitly supports:
`plasmid.add` carries a `mock` mode per attachment, so the same plasmid may legitimately run
`simulate` in one cell and `passthrough` in another. Under a uniqueness rule those are the same
owner, and the only way out is a mangled name like `github-pr@cell-1` — cell identity smuggled
into a string the type system cannot see or check.

It also undercuts decision 002. That decision chose one ledger per cell so a corrupt file costs
one cell; ownership that cannot name a cell means quarantine cannot say which cell's objects it
found, which gives back the containment the file layout was chosen for.

## Rejected

**A globally unique `PluginId` per instance.** Simpler, no churn, and someone will propose it
again — which is why it is written down here rather than left implied. It is wrong for the three
reasons above, and its failure mode is quiet: two cells sharing a plasmid produce object sets that
silently collapse into one, so cross-cell drift disappears instead of being reported.

**Leaving ownership alone and disambiguating at the call site.** Every consumer would carry the
cell separately and remember to pair it with the owner. That is the same information with more
places to get it wrong, and `OsState` is a set — the collapse happens before any call site sees it.

**Deferring until a second real backend exists.** Tempting, since nothing today attaches one
plasmid twice. But spec 008 has to be written against some model now, and writing it against the
wrong one means rewriting recovery rather than extending it.

## Consequences

The change is wide but shallow: `OsObject`, `UniverseOp`'s constructors, and the tests that assert
owner identity — including the `a_change_of_owner_is_both_a_loss_and_a_leak` family, whose meaning
becomes sharper rather than different, since the same plasmid in a different cell now genuinely is
a different owner.

Spec 008 is unblocked by this and needs a small follow-up: `found` and the union behind
`expected` restated in terms of the decided identity, plus acceptance cases for two cells sharing
a plasmid.

One claim in the argument that led here did not survive checking, and is recorded so nobody
repeats it: spec 001 §3.3's example does **not** show one plasmid in two cells — its `cell-2`
carries an empty plasmid list. The case for this decision rests on the verb signatures and the
per-attachment mock mode, not on that example.
