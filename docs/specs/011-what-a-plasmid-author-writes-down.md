---
id: 011
title: What a plasmid author writes down, and what attaching it promises
status: draft
intents: [010]
---

## Behavior

A cell is missing something. The agent working inside it can usually see exactly what — a host it
cannot reach, a command that is not there, a credential nothing hands it — and it is capable of
writing the code that would fill the gap. What stops it is not the code. Filling the gap today
means knowing how Plasmosome grants a capability, what a detach has to undo, and how a cell is
supervised: three things the author has no interest in and no reason to learn.

This spec says what an author writes instead. A plasmid is **a declaration of what it needs from
the outside world and what it offers back**, and nothing in it describes mechanism. The kernel
reads the declaration, grants exactly what it names, and refuses in the declaration's own
vocabulary when it cannot — so an author who knows nothing about the kernel can still be told, in
words they themselves wrote, what is missing.

Everything below follows from one property the kernel already has: **nothing a plasmid does not
declare is available to it.** That is what makes a declaration complete by construction, what makes
a refusal actionable, what lets a generated plasmid be reviewed without its code being read, and
what makes the hand-crafted path and the generated path one path rather than two.

### What someone wants to be true

Three people want three different things, and the same file has to serve all three.

The **agent inside a cell** wants: *I can see what I am missing. I want to write it down, have it
checked, and have it take effect on this cell, without leaving the work I was doing and without
reading anything about how the kernel is built.*

The **person who wants to craft one precisely** wants: *I write the file myself. Nothing on the
generated path is a shortcut I am locked out of, and nothing generated is a wrapper I have to
fight to get underneath.*

The **person who owns the cell** wants: *I can tell what a plasmid is able to reach by reading it,
rather than by trusting whoever or whatever wrote it.*

### The declaration is the whole of what an author writes

One file. It carries six kinds of thing:

- **Who it is, and what implements it** — a stable id, a version, and the component the
  declaration names.
- **What it is for** — a sentence in plain words. This is not decoration. It is what a person
  browsing reaches for and what a model reads before choosing to use the thing at all. A plasmid
  whose purpose can only be recovered by reading its code is unusable by both audiences this goal
  names.
- **What it needs from the outside world** — hosts and ports, workspace paths, credentials,
  commands it may run, a model endpoint, a recorded backend to stand in for a live one. Stated in
  the vocabulary of the outside world: `api.github.com`, not a routing rule.
- **What it offers back** — named tools, each with the sentence a caller reads before choosing it.
- **What it needs from other plasmids** — capabilities by name.
- **How long it may take to stop** — a drain budget.

Every section the grammar in `plasmosome-core::manifest` carries today falls into one of the six:
`[impl]` into the first; `[network]`, `[workspace]`, `[secrets]`, `[commands]`, `[model]` and
`[mock]` into the third; `[provides]` into the fourth; `[requires]` into the fifth; `[lifecycle]`
into the sixth. **This spec removes none of them and adds no seventh kind.**

Three of the six are served badly or not at all, and this spec fixes two of them. There is nowhere
to say **what the plasmid is for**, and nowhere to say **what a tool does** — tools are bare names
like `pr.read`, and the intended consumer of that list is a model, which cannot choose from a bare
name. Both fields are required rather than optional, because an optional purpose field is one
nobody fills in and a registry of blanks is worse than none. The third gap is named and left:
a requirement is a capability name with no version range, while the kernel underneath already
selects a version across competing providers. How an author writes a range, and how a conflict is
explained to them, is its own spec.

### What the author never writes

Naming this list is the whole of "without understanding much about it":

- **How a host becomes reachable.** The author names the host. Brokering, address plans and
  pinning are the kernel's.
- **How the plasmid is torn down.** A plasmid implements no revocation hook, gets no chance to
  refuse a detach, and cannot make one fail. Its own work may still be finishing inside the drain
  window its declaration asks for; what it never gets is a say in whether the teardown happens.
- **How the cell is supervised**, what happens if the plasmid crashes, and what the kernel has to
  undo afterwards.

**One part of the declaration as it stands does not meet that bar, and this spec changes it.** The
credential grammar makes an author choose a delivery mode — `handle`, `helper`, `inject`,
`mint` — precisely the kernel-internal knowledge intent 010 says an author should not need. What
an author does know is the **consumer**: the thing that will use the credential — a component,
`git`, an HTTP call, a spawned process. The delivery follows from the consumer and from whether
the reference is path-scoped, so the author may leave it out and the kernel fills it in.

**A derived delivery is always a single mode, and always the narrowest the frozen pairing
accepts.** That is the rule that keeps deriving from granting anything. It has one visible
consequence: a `git` credential derives `helper` alone, not the `helper`-then-`mint` pair the
canonical fixture writes. Minting is a second way to obtain a credential, and an author who never
asked for it should not get it — intent 012 is explicit that nothing arrives by accident. An
author who wants the fallback writes both, exactly as today.

This is a change to the credential grammar spec 001 §3.10 froze, not a way around it. It is stated
as an amendment below rather than slipped past as an addition.

### The refusal names the line that is missing

An author who does not understand the kernel cannot act on a refusal phrased in the kernel's
terms. So every refusal an author can cause names the declaration it read, the field in it that
is wrong or missing, and the line the author would write. This is what makes the write-attach-fix
loop converge for someone who never learns the rules: they do not have to know what is allowed,
because the refusal tells them what they did not say.

There are three shapes, and the third is the one that matters most.

**A declaration that does not hold together** is refused before anything is granted, naming the
field.

**A declaration that asks for something the cell will not give** is refused at attach, naming the
requirement and what it collided with.

**A plasmid that runs and is denied at the boundary** — it reached for a host it never
declared — is reported as a missing declaration, naming the host and the field it belongs in.
An author handed a connection error starts guessing about the network. An author told that
`api.github.com` is not in this plasmid's declared hosts writes one line and is finished.

That report names what was denied and never what would have been allowed, **and that is the whole
of the safety property.** Routing is not a second one. In the case intent 010 expects most the
author *is* the agent inside the cell, so "it goes to the author rather than to the calling code"
moves the report between two readers who are one reader, and any design leaning on that separation
is leaning on nothing. What holds instead is that the report adds no knowledge: it names a host
that this plasmid's own code chose to reach for, it enumerates nothing that is reachable, and a
poisoned agent learns from it only that the thing it was told to try was refused — which the
refusal already told it. What it gets is a ready-made widening request, which it must still take
through the gate. The denial itself is unchanged: enforcement is not cooperation, and a better
error message is not a second chance.

### Both authors write the same file

`plasmid new <name>` writes the declaration and stops there. It fills in what it can — the id, the
version, an empty purpose — and **grants nothing**. It guesses no capability, because a scaffold
that guesses grants what nobody asked for, which is the thing intent 012 refuses. The file it
writes therefore does not yet parse, and the command prints exactly which sections the author has
to add. That is the intended shape rather than a rough edge: a scaffold that produced a
valid-but-empty declaration would teach the author nothing about what a declaration is for.

The agent path is the same file, generated rather than typed. Nothing makes it work except that
the declaration is small enough to write from a sentence and that its refusals converge. There is
no generator-only field, no section a person could not have typed, and nothing the kernel reads
that records which path produced the file. Recording that a declaration was generated is a
reasonable thing for a reviewer to want; it is a record, not an input, and the kernel treats the
two files identically.

**This narrows the `plasmid new` reservation rather than overriding it.** The stub refuses today
because "the plasmid-sdk interface a scaffold would generate against is not frozen yet, so there
is no shape to write", and for the half it names that is right: a component skeleton generated
against an unfrozen world is generated against a shape that will change. The declaration is not
that half. It is parsed by a grammar that already ships and it names nothing the world decides. So
the scaffold writes the declaration now and still refuses the component, saying so.

### What attach promises

- **All or nothing.** Either every capability the declaration names is granted and the plasmid's
  tools are callable, or nothing about the cell changed. A half-attached plasmid is not a state a
  cell can be in.
- **Attached means usable.** The cell reports the plasmid active and every tool its declaration
  names resolves where a caller looks. A plasmid that attached but whose tools nothing can find
  has not attached.
- **Attach widens the cell and does nothing else.** It does not restart the cell, does not
  interrupt what is running in it, and does not change what any already-attached plasmid was
  **granted**. It does change one thing about them: a mock mode propagates across the dependency
  closure exactly as spec 001 §3.10 froze it. A mode decides whether a granted call reaches a live
  service or a recording; it is not itself a grant, and that propagation is untouched here. No
  restart is a promise the kernel keeps when the cell is made rather than when a plasmid arrives:
  a cell is created already arranged so that what is attached later can be reached from inside it,
  and that arrangement cannot be established in a cell that is already running.
- **Attach never rounds up.** One attach grants across the required closure: the named plasmid
  and every provider it transitively requires each receive exactly what their own declaration
  names, and no plasmid in the closure receives anything from outside its own. A requirer gains
  its provider's tools, never its provider's grants. If satisfying the closure would require
  granting any of them more than their declarations name, attach refuses rather than granting
  the wider thing.
- **Detach revokes what the plasmid's own declaration held, and nothing another plasmid still
  needs.** Grants are held per declaration, so detaching a plasmid revokes its own grants and
  unregisters its own tools. A provider that arrived as part of a closure stays while any
  attached plasmid still requires it and goes with its last requirer; a detach that would strand
  an attached requirer is refused, naming the requirers. A mode the detached plasmid had
  declared stops propagating with it; what the remaining declarations propagate is governed by
  spec 001 §3.10, unchanged.
- **Revocation is enforcement, not erasure.** After detach, no tool of the plasmid resolves and
  nothing its grants allowed passes the boundary — for processes that were already running when
  it happened, not only for ones started after. What detach does not claim is what had already
  crossed into a running process before it: a value a process read, an environment a process was
  started with, a handle a process opened. Nothing can unread those, and this contract does not
  pretend to; what it promises is that nothing so retained reaches past the boundary again. That
  a tool no longer *resolves* is a statement about what a name now finds, not a claim that a
  handle already open stops working. Detach needs nothing from the plasmid throughout — no hook,
  no veto, no say.
- **Detach returns when reachability is revoked, and is accountable for a bound after that.**
  Returning means no new reference to anything the plasmid placed can be obtained. It does not
  mean every reference is gone: one taken before the detach keeps its object alive, and detach
  neither waits for it nor destroys it. What the contract holds instead is the bound — every
  reference that outlives the detach is named, with the owner holding it, until the last one
  goes. These are two failures, and they are not observable at the same moment. A surviving
  reference with no owner is visible while the detach is still running, and it fails that detach:
  the check runs where it can still change the outcome, because a check that cannot is telemetry
  and this contract does not rest on one. A reference held past its bound cannot be seen then —
  the bound expires after the detach has returned — so it is not a detach failure but an
  obligation the contract keeps afterwards, reported against the owner named at detach. Naming
  the second as though the detach could have caught it would be claiming a check nothing can
  run.
- **What a refusal cannot protect.** A detach that would strand an attached requirer is refused,
  and that covers dependence some declaration states. It does not cover dependence nothing
  declared — work the agent did against a tool while it was attached, state it holds that assumes
  a capability. No declaration in the closure records that, so nothing can refuse on its behalf.
  Removing a capability is the operation with no declaration standing behind it, and this
  contract says so rather than implying a protection it does not have.

### What this amends in spec 001

Three of the changes above reach into an accepted spec. Gathering them here is deliberate: a
change to a frozen contract that is only visible as a new capability elsewhere in the document is
a change nobody reviewed.

- **§3.10, the credential grammar.** `delivery` becomes optional. Where it was "an ordered
  non-empty list over the closed enum", it is now that or absent, and absent derives the single
  narrowest mode the pairing rules accept for the declared consumer. The four modes, the consumer
  set and the pairing rules themselves are unchanged; what changes is that a well-formed reference
  need no longer carry the field.
- **§1, the error table.** No code is added, removed or renumbered — the closed set stands. The
  refusals an author can reach gain one structured field, `fix`, holding the line the author would
  write. Where the field is list-valued, `fix` holds the entry to add, not the whole line: a
  replacement line would either drop the entries already there or name them, and naming what is
  already granted is what the report above forbids.
- **§3.9 and §3.6, and only those two.** The `plasmid.add` reply of §3.10 and the
  `plasmid.reload` reply of §3.12 also carry a plasmid as an object and are deliberately left
  alone: both report a transition, and the plasmid on the far side of it has been denied nothing
  yet — a reloaded plasmid is a new generation, and the previous generation's denials belong to
  the status and list responses that report accumulated state. Each per-plasmid object
  gains one field, `denials` — omitted when empty, per §1's rule that a field with nothing in it
  is never sent. Each entry names one distinct denial: what was reached for, in the
  declaration's own vocabulary; the declaration field it belongs in; the `fix` line the author
  would write; and how often it was denied. A repeat of the same denial raises `count` rather
  than adding an entry; the session log, which records events rather than state, carries every
  denial as its own line.

  ```json
  {"plasmid": "github-pr", "mock": "simulate", "generation": 3, "state": "active",
   "label": "github-pr [mock:simulate]",
   "denials": [{"denied": "api.example.com", "field": "network.hosts",
                "fix": "hosts = [\"api.example.com\"]", "count": 3}]}
  ```

  §3.3 is untouched: its `plasmids` entries are labels, not objects, and the label grammar does
  not change here. This is a field added to two frozen response shapes, which is why it is named
  and drawn here rather than left to be discovered in the diff.

### The gate, and what this spec does not decide

Intent 010 names a gate between generating a plasmid and it taking effect, and says outright that
its shape is undecided. It stays undecided here. Four things about it are already settled — not
by this spec, but by approved intents and by how the kernel enforces — and stating them is what
keeps the open question from being answered by accident.

**A cell cannot approve its own widening.** The agent that found the gap is the one asking for the
capability, and intent 012 puts the agent inside the boundary last on the list of things to rely
on — it reads untrusted text all day, and a poisoned one is following instructions faithfully. A
gate operated by the party requesting it is not a gate. So the request leaves the cell and the
decision is made outside it, whatever the gate turns out to be.

**A declaration is enough to bound reach — the plasmid's own, and every declaration it requires.**
Because the kernel grants exactly what is declared, that closure is what a plasmid can touch, and
a reviewer who reads it knows the worst case without reading a line of code. The single
declaration is not enough on its own: a plasmid requiring a capability reaches, through its
provider's tools, whatever that provider's declaration grants, and attach brings the whole closure
with it.

**Reading bounds reach, not conduct.** A plasmid granted a repository can do anything to that
repository. The credential grammar's scopes narrow part of the gap and do not close it.

**One approval bounds reach in both directions.** A gate approves a declaration. Because the
kernel grants exactly what that declaration names, and detach revokes exactly what it held, a
detach cannot widen reach — so bounding reach needs no second artifact, and the fields that put
the strongest authority on removal are the ones whose approved artifact carries the reverse
procedure alongside the forward one. This is a statement about reach, on the same terms as the
paragraph above it, and it is not the claim that a detach therefore needs no gate. Removing a
plasmid changes conduct without widening reach — detaching a mock or an audit plasmid leaves the
closure narrower and the behaviour different — and whether that warrants its own approval is part
of the question this section leaves open. What one approval does not reach at all is dependence
no declaration expresses, which the detach promises above state as a limit rather than leave to
be inferred.

**This says what a declaration is sufficient for, not what a gate is limited to.** A gate that
also reads the code, the diff, or where the declaration came from is ruled out by nothing here,
and the reach-versus-conduct gap is a positive argument for one. Where the gate sits, who holds
it, and whether a hand-crafted plasmid on the author's own cell passes through the same one is the
owner's to settle and belongs to a sibling spec. Nothing above prejudges it.

## Contract

- **A plasmid is one declaration file plus the implementation it names.** The declaration is the
  only thing the kernel reads to decide what the plasmid may reach.
- **The declaration carries a required `description`** — one or two plain sentences saying what
  the plasmid is for. A declaration without one does not parse, and the error names the field.
- **Every declared tool carries a required description.** The `[provides]` binding's `tools` is a
  table of tool name to sentence rather than a list of names:

  ```toml
  [provides."github:tools".tools]
  "pr.read" = "Read a pull request's title, body and review state."
  "pr.comment" = "Post a comment on a pull request."
  ```

  A tool named with no sentence does not parse. This replaces the list form; no manifest in the
  tree keeps it, and the change is deliberate rather than additive — a grammar that accepts both
  leaves the sentence optional in practice. `plasmosome-testkit`'s manifest builder carries the
  same shape.
- **A secret ref's `delivery` is optional.** When absent it derives to **exactly one mode**, the
  narrowest the frozen pairing accepts for that consumer: `wasm` derives `handle`; `git` derives
  `helper`; `http` and `process` derive `inject` when the ref carries a non-empty `path_scope` and
  `mint` when it does not. An explicit `delivery` is still accepted and still validated exactly as
  today, including the `git` pair `["helper", "mint"]`, which an author who wants the minting
  fallback writes for themselves. **Deriving grants nothing:** each derived mode is a single mode
  the validator accepts, and no derived list is wider than one the author could have written for
  the same consumer and scope.
- **An `inject` reference whose `path_scope` is empty is refused**, whether the `delivery` was
  written or derived. Spec 001 §3.10 requires an absolute path scope for `inject`, and an empty
  list satisfies a check that walks its entries while scoping nothing.
- **Deriving rescues nothing else.** A reference with no `delivery` whose scope is malformed — a
  relative path-scope entry — is refused with the same named error an explicit `inject` gets.
- **Every refusal an author can cause carries the field path in the declaration and a `fix`
  sentence** holding the line the author would write, and carries the plasmid id wherever the
  declaration supplies one. A declaration with no `id` is the single case that cannot name a
  plasmid; it names the missing `id` instead. Where a refusal must name more than one plasmid —
  the requirers a detach would strand — it names them in the message and the `fix`, not in a new
  structured field: §1's assertion carries a single plasmid, and adding a field to it would be a
  fourth amendment this spec does not make.
- **A capability denied at the boundary is reported as a missing declaration.** The report names
  what was denied and the declaration field it belongs in, and names nothing that was granted or
  would have been. It is written to the cell's session log, one line per denial, and to the
  plasmid's `denials` status field, one entry per distinct denial carrying its count. The
  denial's own behavior is unchanged, and the report is not a channel the calling code reads.
- **Attach is all-or-nothing.** On any failure the cell holds no record of the plasmid, no tool of
  its own, and no grant made on its behalf.
- **A plasmid is reported active only when every tool its declaration names resolves.**
- **Attach grants nothing outside the required closure's own declarations.** Each plasmid in the
  closure — the one named and every provider it transitively requires — is granted exactly what
  its own declaration names; a requirer gains its provider's tools, never its provider's grants;
  and attach refuses rather than granting more to any of them. It changes no already-attached
  plasmid's grants; mock-mode propagation across the dependency closure is unchanged from spec
  001 §3.10.
- **Detach requires nothing from the plasmid** and cannot be refused or failed by it. It revokes
  every grant the plasmid's own declaration held and unregisters its tools. A provider is
  revoked only with its last requirer; a detach that would strand an attached requirer is
  refused, naming the requirers. That is not the only refusal: spec 001 §3.11 already refuses a
  safe removal with code `105` when external effects are outstanding, and force requires the
  operator/reason pair — unchanged here, and named so that the promise above is not read as a
  claim that a detach never refuses. Neither refusal is the plasmid's to make. Revocation is
  enforcement rather than erasure: after detach nothing the plasmid's grants allowed passes the
  boundary, and what a running process already read is not claimed back. Detach returns when no
  new reference can be obtained; a reference taken before it keeps its object alive, and every
  such reference is named with its owner until the last one goes. A surviving reference with no
  owner fails the detach, on a check that runs where it can still fail it. A reference held past
  that bound is not a detach failure — the bound expires after the detach returns — but an
  obligation reported against the owner named at detach.
- **`plasmid new <name>` writes exactly one file, the declaration**, grants nothing, attaches
  nothing, and prints the sections the author must add. It writes no component, says that it
  will not until the plasmid interface is frozen, and exits `2` — the code decision 010 fixed
  for that refusal, kept for as long as the component half is refused.
- **The generated path and the hand path produce the same artifact.** No field, section or value
  is reachable from only one of them, and neither path writes a field recording which produced
  it. Provenance is a TOML comment, which costs the grammar nothing; a field would have to be
  admitted by the parser, and a parser that admits fields it does not define cannot tell a
  provenance note from a misspelled one.
- **This spec amends spec 001 in three named places** — `delivery` optionality in §3.10, the
  `fix` field in §1, and the `denials` field on the per-plasmid objects of §3.9 and §3.6 — and
  nowhere else. No error code is added, removed or renumbered.
- **Nothing here decides the gate.** Two properties hold whatever it turns out to be: the approval
  is not the cell's to give, and the declarations of a plasmid and its required closure are
  sufficient for a reviewer to bound its reach. Neither limits what else a gate may read.

## Acceptance

- A declaration with no `description` is refused, and the refusal names `description`.
- A `[provides]` binding naming a tool with no sentence is refused, and the refusal names the
  tool.
- The list form of `tools` is refused, and no manifest anywhere in the tree still uses it.
- `plasmosome-testkit`'s manifest builder produces the table form, and every test that drives it
  still passes.
- A well-formed secret ref with a `consumer` and no `delivery` parses, and its derived list is
  asserted for all four consumers — including both `http`/`process` branches, one ref carrying a
  non-empty `path_scope` and one carrying none.
- Every derived list holds exactly one mode, and is passed through the frozen pairing validator in
  the same test and accepted, rather than being compared to a table by eye.
- A `git` ref with no `delivery` derives `["helper"]` and not `["helper", "mint"]`.
- An `inject` ref whose `path_scope` is the empty list is refused, asserted once with an explicit
  `delivery` and once with a derived one.
- A ref with no `delivery` and a relative `path_scope` entry is refused with the same named error
  the explicit form gets.
- An explicit `delivery` still parses; an illegal explicit pairing, an empty explicit list, and an
  unknown mode are each still the same named error they are today.
- Each of these refusals carries the field and a non-empty `fix`, and the plasmid id where there
  is one: missing `description`; a tool with no sentence; an illegal delivery pairing; an empty
  explicit delivery list; an unknown delivery mode; `inject` with no path scope; an empty path
  scope; a relative path-scope entry; a network section with no hosts; a declaration naming no
  capability and no implementation; a command secret naming no subject; and a declaration with no
  `id`, which names `id` and carries no plasmid id.
- A plasmid granted two hosts and denied a third produces a report naming the denied host and the
  hosts field, in which neither granted host appears anywhere.
- The denial report reaches the session log and the plasmid's status, and the calling code inside
  the plasmid sees the denial unchanged.
- A host denied three times appears in the plasmid's `denials` once, carrying `count` 3, and in
  the session log three times.
- An attach failed partway leaves no trace: the cell lists no such plasmid, the tool registry
  holds none of its tools, and the ledger records no grant for it.
- An attach that could only succeed by granting more than the declaration names is refused, and
  the refusal names the wider thing it would have taken.
- A plasmid requiring a capability whose provider itself requires a second capability attaches
  as one closure: each of the three plasmids holds exactly the grants its own declaration names,
  the requirer holds none of its providers' grants, and a refusal at the deepest provider leaves
  no record, tool or grant for any of the three.
- An attach leaves every already-attached plasmid's grants unchanged, asserted by comparing the
  cell's grants before and after; a mock mode propagating across the closure in the same attach is
  asserted separately and is not counted as a change of grant.
- A plasmid one of whose declared tools does not resolve is not reported active.
- A plasmid whose code is unresponsive still detaches: the cell reports it gone, the registry
  holds none of its tools, and the ledger records the reverse of every grant its own declaration
  held.
- Two plasmids requiring the same provider attach; detaching the first leaves the provider
  attached and the second plasmid's tools resolving; detaching the second removes the provider,
  after which a call the provider's grants had allowed is denied.
- Detaching a provider that an attached plasmid still requires is refused, and the refusal names
  the requirer.
- A detach whose object outlives it and whose owner is named succeeds, and reports that object
  with its owner; a detach whose object is gone reports none.
- A detach whose surviving object has no owner **fails**, and the failure names that object. The
  same staging with an owner recorded succeeds — the two cases differ only in the owner, so a
  detach that fails for any other reason does not satisfy this clause, and a failure branch no
  case ever reaches does not either.
- Neither check is satisfied by one that only ever runs after the detach has been reported done,
  nor by one that finds nothing because the name was destroyed while the object it named is
  still alive: the object is identified by what it is, not by what it was called.
- A reference still held when its bound expires is reported against the owner named at detach.
  This is asserted after the detach returned successfully, and no case requires the detach to
  have anticipated it.
- After a detach, a process that was already running when it happened is denied at the boundary
  on every capability the detached declaration named, and no tool of that plasmid resolves for
  it.
- `plasmid new <name>` writes exactly one file, creates nothing else, exits `2`, and prints the
  names of the sections the author must add.
- The file `plasmid new` writes is refused by the parser, and the refusal names the same sections
  the command printed. The existing test asserting that `plasmid new` creates nothing at all is
  replaced by this pair rather than kept alongside it.
- The scaffold's output, with the sections it named filled in, parses to the same declaration as
  the same content typed from scratch, and the scaffold writes no field the grammar does not
  already define.
- A declaration carrying a comment recording how it was produced parses to the same declaration
  as one without it, and the kernel grants the two identically. A declaration carrying an extra
  *field* is refused, and the refusal names that field — including when the field is a plausible
  provenance note, since the parser cannot distinguish one from a misspelling of a field the
  grammar does define.
- Every manifest fixture in the tree is updated to the new grammar; none is exempted, and no
  fixture parses without a description.

## Out of scope

- **The plasmid's own interface** — the WIT world, what a component exports, how a tool is
  invoked. `plasmid-sdk` reserves it and spec 001 §5 deliberately leaves it unfrozen. This spec
  sits upstream of it: it says what an author declares, never what their code implements, and
  nothing here waits on the world being designed. Freezing the world is its sibling spec, and it
  is the larger of the two.
- **Where the approval gate sits, who holds it, and whether a hand-crafted plasmid on the author's
  own cell passes through it.** Open in intent 010, open here, and the owner's to settle. The two
  properties named above are the only ones this spec asserts about it, and neither narrows what a
  gate may read.
- **The registry.** Finding a plasmid somebody else wrote and publishing your own is intent 014
  and a different goal. This spec says what the thing in a registry would be, not where it lives.
- **The skill an agent inside a cell invokes to generate one.** Intent 010 names that path as the
  common case; what the skill is made of is downstream of this contract rather than part of it.
- **Mock modes and their layering.** The three modes, their propagation across a dependency
  closure, and the conflict rules are frozen in spec 001 §3.10 and untouched. The only thing said
  about them here is that propagation is not a change of grant.
- **The delivery modes, the consumer set and the pairing rules.** Unchanged. What this spec
  changes about the credential grammar is one thing only — that `delivery` may be omitted — and
  that change is named as an amendment above rather than claimed as untouched.
- **Capability requirement and version selection between plasmids.** The kernel already selects a
  version across providers; how an author writes a version range and how a conflict is explained
  to them is its own question and its own spec.
- **The `[commands]` section's shape.** Reserved in spec 001; an author declaring a command still
  writes it as it stands today.
- **How attached software becomes visible inside a cell.** A plasmid's software is not copied
  into a running cell: attach makes it visible, detach stops making it visible, and the layout
  that gives a later attach somewhere to land is prepared when the cell is created. All of that
  is the kernel's, none of it is the author's to write, and its spec belongs to the isolation
  work under intent 011.
