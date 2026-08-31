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
same plasmid is attached to two cells of one instance, which the control protocol allows. Spec 001
§2 chains addressing down `(kernel, cell, plasmid)`, so a plasmid is named relative to a cell, and
the verb schemas carry it: `plasmid.add`, `plasmid.remove` and `plasmid.reload` each take kernel,
cell and plasmid as separate parameters (§3.10–3.12), `plasmid.list` is addressed to one cell and
returns that cell's plasmids (§3.9), and `plasmid.add`'s `mock` is chosen per attachment (§3.10).
Nothing in the frozen set asserts uniqueness above the cell. The one line that reads that way on a
fast skim is `plasmosome.list`'s instance-level `"plasmids": 3` (§3.2) — but §3.2 never says what
that number counts, so it settles nothing in either direction. A plasmid is an attachment to a
cell, not a thing an instance has one of.

Spec 008's recovery work is where this stopped being theoretical. Quarantine reports every
snapshot object whose owner is a plugin named by a line of the cell's ledger that parsed, and
recovery unions expected state across adopted cells. Per cell, and across cells whose plasmids
are disjoint, both are exact. Where one plasmid spans two cells, both have to answer *which
cell*, and neither can.

## Decision

**An object's owner names the cell.** `OsObject`'s ownership becomes a plasmid together with the
cell it is attached to, rather than a plasmid alone.

The alternative is to make a plasmid name unique across an instance — either by declaring it so,
or by refusing the second attach. Both are rejected below. The short reason is that neither can
express what the protocol already offers: a `mock` mode chosen per attachment, so the same
plasmid may legitimately run `simulate` in one cell and `passthrough` in another. Under a
uniqueness rule those are either the same owner, or one of them is refused outright.

It also undercuts decision 002. That decision chose one ledger per cell so a corrupt file costs
one cell; ownership that cannot name a cell means quarantine cannot say which cell's objects it
found, which gives back the containment the file layout was chosen for.

## Rejected

**A globally unique `PluginId` per instance, declared.** Simpler, no churn. The type would assert
what the protocol does not — that a plasmid name identifies one attachment — and the only way to
keep the assertion while addressing two cells is a mangled name like `github-pr@cell-1`, cell
identity smuggled into a string the type system cannot see or check. Its failure mode is quiet,
which is what makes it the dangerous one: ownership that cannot name a cell cannot attribute. The
objects of two cells land in one flat `OsState` with nothing to partition them by, and anything
wanting the cell back has only the key to read it out of — which some classes happen to encode (§4
puts each cell's membrane socket under `cells/<cell>/`) and others do not. That is the mangled
name again, arrived at from the other side. Where the two cells also share a class and key — a
proxy map for the same host, a mount on the same target — the objects deduplicate outright, and
the drift between them is not merely unattributed but gone.

**A globally unique plasmid name per instance, enforced.** The stronger form, and the one that
will come back: the controller refuses a `plasmid.add` naming a plasmid already attached to
another cell, so uniqueness is a constraint the runtime maintains rather than a claim the type
makes. It does not fail quietly, and it deserves a straight answer. The objection is not that the
refusal is hard to signal — error `102` is already `already_exists` / `already_attached` and fits
without a new code. It is what the rule costs. It removes the one thing §3.10 allows that
uniqueness cannot express — one plasmid holding a different mode in each of two cells. And it
cannot stop at `plasmid.add`: §3.5 germinates a cell from a genome and attaches the plasmid set
that genome resolves to, so the same rule must refuse the second `cell.new --genome researcher`.
Two cells from one genome is the ordinary case — a second researcher, a retry alongside the
first — and giving it up to keep an owner field narrow is the wrong trade. The residual contract cost is
small but real: `102` carries only `target`, so naming which cell already holds the plasmid wants
a new field on an existing code, and §1 forbids a client parsing `message` to branch on it.

**Leaving ownership alone and disambiguating at the call site.** Every consumer would carry the
cell separately and remember to pair it with the owner. That is the same information with more
places to get it wrong, and `OsState` is a set with no cell dimension — the objects are already
unattributable by the time any call site sees them.

**Deferring until a second real backend exists.** Tempting, since nothing today attaches one
plasmid twice. But spec 008 has to be written against some model now, and writing it against the
wrong one means rewriting recovery rather than extending it.

## Consequences

No new mechanism appears, but this is not a one-field edit, and two of the four surfaces are more
than a widening.

- **`OsObject`, `UniverseOp`, and what embeds them.** The owner field, and every constructor that
  fills it. `OsState`, `Diff` and `ResidueReport` all carry `OsObject` and change shape with it,
  `ResidueReport` in its `Residue` variant. `plasmosome-freeze-checks` lists all of them, but what
  it asserts is that each is serde in both directions — which a widened `OsObject` keeps. No test
  there pins a field set, so the gate will not tell you the shape moved. What makes it a contract
  change is the spec: §3.4 puts `ResidueReport` on the wire as `plasmosome.stop`'s `residue_items`,
  so a published response body gains a field. The choice is not freeze-touching against
  freeze-preserving; it is a field added to a body nothing durable has written yet, against a
  capability withdrawn from the protocol.
- **Where the cell type lives.** `CellId` is in `plasmosome-core`, which depends on
  `plasmosome-backend`, so the backend cannot name it without a cycle. It moves down to the
  backend or is mirrored there — the same carve-out spec 008 already makes for `MockMode`.
- **Where the owner comes from.** `Grant` and `LedgerEntry` carry a `PluginId` and no cell, and
  an object's owner is built from `grant.plugin`. A cell has to reach that point: either those
  types grow one, or every construction site supplies it.
- **`OsState`'s lookups, which need rework rather than a new signature.** `owner_of`, `contains`
  and `remove` key on class and key alone, and `owner_of` returns the first match — so `remove`,
  which resolves its owner through that match, can withdraw an object it was not asked for. That
  hazard exists already: `SetProxyMap` drops its `route` when forming the key, so two plasmids
  proxying one host collide under a single class and key today. What the widening changes is how
  ordinary the collision becomes, since two cells running one plasmid is the case this decision
  exists for. The fix is a cell in the lookup key, not a new return type. These are inherent
  methods on the `OsState` every backend returns from `snapshot_os_state`, and the conformance
  suite drives them — nine of its assertions go through `contains` — so this reaches every
  backend implementation, not only ours.

Two neighbours this deliberately does not reach. `plasmosome-ledger`'s `LogRecord` and `Ledger`
stay plasmid-keyed, because spec 008 takes the cell from the ledger's path and the line does not
need to carry one. `ToolRegistry` maps a tool name to a `PluginId` with no cell, and
`withdraw_plugin` withdraws across every cell a plasmid serves — the same assumption again,
left alone only because nothing wires it into a controller yet.

Spec 008 is unblocked by this and needs a small follow-up: `found` and the union behind
`expected` restated in terms of the decided identity, plus acceptance cases for two cells sharing
a plasmid.

Three claims made while arguing for this did not survive checking, and are recorded so nobody
repeats them. Spec 001 §3.3's example does **not** show one plasmid in two cells — its `cell-2`
carries an empty plasmid list. Refusing a duplicate attach would **not** need a new error code:
`102` is already `already_attached`, and only the field naming the other cell is missing. And
`plasmosome-freeze-checks` does **not** pin these types' fields — it asserts they are serde in
both directions, which a widened `OsObject` still is, so the wire commitment is the spec's and
not the test's. The case for this decision rests on the verb signatures and the per-attachment
mock mode, not on any of those.
