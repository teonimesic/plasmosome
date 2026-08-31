---
id: 003
title: An object's owner is a plasmid in a cell, not a plasmid
date: 2026-08-31
status: accepted
---

## Context

`OsObject` is `{ class, key, owner: PluginId }`, and `crates/plasmosome-backend` names no `CellId`
and carries no cell in any of its types. So the snapshot of the world cannot say which cell an
object belongs to — only which plasmid asked for it.

That is fine while a plasmid can only ever be attached once. It stops being fine the moment the
same plasmid is attached to two cells of one instance, which nothing in the protocol forbids. Spec
001 §2 chains addressing down `(kernel, cell, plasmid)`, so a plasmid is named relative to a cell,
and the verb schemas carry it: `plasmid.add`, `plasmid.remove` and `plasmid.reload` each take
kernel, cell and plasmid as separate parameters (§3.10–3.12), `plasmid.list` is addressed to one
cell and returns that cell's plasmids (§3.9), and `plasmid.add`'s `mock` is chosen per attachment
(§3.10). Nothing in the frozen set asserts uniqueness above the cell. The one line that reads that
way on a fast skim is `plasmosome.list`'s instance-level `"plasmids": 3` (§3.2) — but §3.2 never
says what that number counts, so it settles nothing in either direction. A plasmid is an
attachment to a cell, not a thing an instance has one of.

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

The declared form also blunts decision 002's quarantine report. That decision chose one ledger per
cell so a corrupt file costs one cell. An owner that cannot name a cell means `found` is filtered
by plasmid name alone, so it over-reports — another cell's objects listed as this cell's. Adoption
containment survives; the report's precision does not.

## Rejected

**A globally unique `PluginId` per instance, declared.** Simpler, no churn. The type would assert
what the protocol does not — that a plasmid name identifies one attachment — and the only way to
keep the assertion while addressing two cells is a mangled name like `github-pr@cell-1`, cell
identity smuggled into a string the type system cannot see or check. Its failure mode is quiet,
which is what makes it the dangerous one: ownership that cannot name a cell cannot attribute. The
objects of two cells land in one flat `OsState` with nothing to partition them by, and anything
wanting the cell back has only the key to read it out of — which §4's own path convention would
encode, since each cell's membrane socket sits under `cells/<cell>/`, though no granted object
lives there today. Where the two cells also share a class and key — a proxy map for the same host,
a mount on the same target — the objects deduplicate outright, and the drift between them is not
merely unattributed but gone.

**A globally unique plasmid name per instance, enforced.** The stronger form, and the one someone
will propose again: the controller refuses a `plasmid.add` naming a plasmid already attached to
another cell, so uniqueness is a constraint the runtime maintains rather than a claim the type
makes. It does not fail quietly, and it deserves a straight answer. The objection is not that the
refusal is hard to signal — error `102` is already `already_exists` / `already_attached`. It is
what the rule costs. It removes what §3.10's per-attachment `mock` leaves open — one plasmid
holding a different mode in each of two cells. And it cannot stop at `plasmid.add`: §3.5
germinates a cell from a genome and attaches the plasmid set that genome resolves to, so the same
rule must refuse the second `cell.new --genome researcher`. Two cells from one genome is the
ordinary case — a second researcher, a retry alongside the first — and giving it up to keep an
owner field narrow is the wrong trade. The residual contract cost is real: `102` carries only
`target`, so naming which cell already holds the plasmid adds a field to a structured-field spec
§6 freezes alongside the code itself, and §1 forbids a client parsing `message` to branch on it.

**An opaque owner minted per attachment.** The controller mints an `OwnerId` at attach time and
the backend stores it without knowing what it stands for. It partitions as well as a cell does,
and it needs no `CellId` in `plasmosome-backend`, so it dissolves the crate move below — which
makes it the strongest of them. It loses what the decision is for. The backend can no longer say
*which* cell in a residue report without a side table, and a restarted controller has to rebuild
that table before any report means anything. That is the recovery problem spec 008 exists to
solve, moved rather than removed.

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
  change is the spec: §3.4 puts these objects on the wire in `plasmosome.stop`'s `residue_items`,
  so a published response body gains a field. The choice is not freeze-touching against
  freeze-preserving; it is a field added to a body nothing durable has written yet, against a
  capability withdrawn from the protocol.
- **Where the cell type lives.** `CellId` is in `plasmosome-core`, which depends on
  `plasmosome-backend`, so the backend cannot name it without a cycle. It moves down to the
  backend or is mirrored there — the same carve-out spec 008 already makes for `MockMode`.
- **Where the owner comes from.** `Grant` and `LedgerEntry` carry a `PluginId` and no cell, and
  an object's owner is built from `grant.plugin`. A cell has to reach that point: either those
  types grow one, or every construction site supplies it.
- **`OsState`'s lookups, which answer *someone* and never *which cell*.** `owner_of`, `contains`
  and `remove` key on class and key alone, and `owner_of` returns the first match. Two plasmids
  already collide under one class and key — `SetProxyMap` drops its `route` when forming it — so
  a lookup can already answer for an object it was not asked about, and two cells running one
  plasmid makes that ordinary rather than rare. Widening the owner does not settle it: what an
  owner *is*, and which object a removal *names*, are separate questions, and this decision
  answers only the first. These are inherent methods on the `OsState` every backend returns from
  `snapshot_os_state`, and nine conformance assertions go through `contains`, so changing their
  signatures fails every backend's certification — all three of them, all in this repository.

Two places this deliberately does not reach. `plasmosome-ledger`'s `LogRecord` and `Ledger` stay
plasmid-keyed, because spec 008 takes the cell from the ledger's path and the line does not need
to carry one. `ToolRegistry` keys a tool name to one `PluginId` with no cell, so two cells
attaching one plasmid share a single entry and the first detach empties the registry for the
second — a live-capability consequence, not only an attribution one. `withdraw_plugin` filters by
plasmid alone for the same reason. Left alone only because nothing wires it into a controller yet.

Spec 008 is unblocked by this. The follow-up: `found` and the union behind `expected` restated in
terms of the decided identity, acceptance cases for two cells sharing a plasmid, and its `Blocked
on` bullet and `status: draft` cleared.

Three claims made while arguing for this did not survive checking, and are recorded so nobody
repeats them. Spec 001 §3.3's example does **not** show one plasmid attached to two live cells —
its `cell-2` is draining with an empty plasmid list. It does show two cells built from one genome,
which §3.5 resolves to a single plasmid set, so it supports the germination argument rather than
the attachment one. Refusing a duplicate attach would **not** need a new error code — `102` is
already `already_attached` — though naming the other cell still adds a field the spec freezes. And
`plasmosome-freeze-checks` does **not** pin these types' fields — it asserts they are serde in
both directions, which a widened `OsObject` still is, so the wire commitment is the spec's and not
the test's. The case for this decision rests on the verb signatures and the per-attachment mock
mode, not on any of those.