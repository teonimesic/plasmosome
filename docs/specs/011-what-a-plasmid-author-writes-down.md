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

One file. It carries six things and nothing else:

- **Who it is** — a stable id and a version.
- **What it is for** — a sentence in plain words. This is not decoration. It is what a person
  browsing reaches for and what a model reads before choosing to use the thing at all. A plasmid
  whose purpose can only be recovered by reading its code is unusable by both audiences this goal
  names.
- **What it needs from the outside world** — hosts and ports, workspace paths, credentials,
  commands it may run. Stated in the vocabulary of the outside world: `api.github.com`, not a
  routing rule.
- **What it offers back** — named tools, each with the sentence a caller reads before choosing it.
- **What it needs from, and offers to, other plasmids** — capabilities by name, with a version
  range.
- **How long it may take to stop** — a drain budget.

The grammar in `plasmosome-core::manifest` already carries most of this and this spec does not
re-open it. What that grammar has nowhere to put is the two prose fields: the plasmid's purpose,
and the per-tool sentence. Their absence is the difference between a declaration a kernel can
enforce and one an author or a model can use. Tools today are bare names — `pr.read`,
`pr.comment` — and the intended consumer of that list is a model, which cannot choose from a bare
name. Both fields are required rather than optional, because an optional purpose field is one
nobody fills in and a registry of blanks is worse than none.

### What the author never writes

Naming this list is the whole of "without understanding much about it":

- **How a host becomes reachable.** The author names the host. Brokering, address plans and
  pinning are the kernel's.
- **How the plasmid is torn down.** A plasmid never implements its own revocation, gets no chance
  to refuse one, and runs no code during one. Detach is done to it, not with it.
- **How the cell is supervised**, what happens if the plasmid crashes, and what the kernel has to
  undo afterwards.

**One part of the declaration as it stands does not meet that bar, and this spec fixes it.** The
frozen credential grammar makes an author choose a delivery mode — `handle`, `helper`, `inject`,
`mint` — which is precisely the kernel-internal knowledge intent 010 says an author should not
need. What an author does know is the **consumer**: the thing that will use the credential — a
component, `git`, an HTTP call, a spawned process. In every pairing the frozen validator accepts,
the delivery list follows from the consumer and from whether a path scope is present, so the
author may omit it and the kernel fills it in. Nothing about the pairing rules changes, and an
author who wants to pin the delivery still writes it.

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

That report says what was denied and never what would have been allowed, and it is addressed to
whoever is authoring the plasmid — it appears in the cell's session log and in the plasmid's
status — rather than being handed back to the code that made the call. The denial itself is
unchanged: enforcement is not cooperation, and a better error message is not a second chance.

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
because "the interface a scaffold would generate against is not frozen yet", and for the half it
names that is right: a component skeleton generated against an unfrozen world is generated against
a shape that will change. The declaration is not that half. It is parsed by a grammar that already
ships and it names nothing the world decides. So the scaffold writes the declaration now and still
refuses the component half, saying so.

### What attach promises

- **All or nothing.** Either every capability the declaration names is granted and the plasmid's
  tools are callable, or nothing about the cell changed. A half-attached plasmid is not a state a
  cell can be in.
- **Attached means usable.** The cell reports the plasmid active and every tool its declaration
  names resolves where a caller looks. A plasmid that attached but whose tools nothing can find
  has not attached.
- **Attach widens the cell and does nothing else.** It does not restart the cell, does not
  interrupt what is running in it, and does not change what any already-attached plasmid holds.
- **Attach never rounds up.** If satisfying the declaration would require granting more than it
  names, attach refuses rather than granting the wider thing.
- **Detach takes exactly what attach gave**, needs nothing from the plasmid, and leaves the cell
  as it was.

### The gate, and what this spec does not decide

Intent 010 names a gate between generating a plasmid and it taking effect, and says outright that
its shape is undecided. It stays undecided here. Two things about it are not a matter of policy,
because they follow from the architecture, and stating them is what keeps the open question from
being answered by accident.

**A cell cannot approve its own widening.** The agent that found the gap is the one asking for the
capability, and intent 012 puts the agent inside the boundary last on the list of things to rely
on — it reads untrusted text all day and a poisoned one is following instructions faithfully.
So the request leaves the cell and the decision is made outside it. Whatever the gate turns out to
be, it is not something a cell performs on itself.

**What the gate reads is the declaration.** Because the kernel grants exactly what is declared,
the declaration bounds what the plasmid can reach, and a reviewer who reads only the declaration
knows the worst case in reach. It does **not** bound what the plasmid does with what it reaches: a
plasmid granted a repository can do anything to that repository. Reach is what reading buys, and
saying so is what stops the gate being trusted for more than it gives.

Everything else — where the gate sits, who holds it, whether a hand-crafted plasmid on the
author's own cell passes through the same one — is the owner's to settle and belongs to a sibling
spec. Nothing above prejudges it.

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
  leaves the sentence optional in practice.
- **A secret ref's `delivery` is optional and derived from its `consumer`** when absent:
  `wasm` derives `["handle"]`; `git` derives `["helper", "mint"]`; `http` and `process` derive
  `["inject"]` when the ref carries a `path_scope` and `["mint"]` when it does not. An explicit
  `delivery` is still accepted and still validated exactly as it is today. **Deriving never
  widens:** every derived list is one the frozen pairing validator already accepts, and `inject`
  is derived only where the path scope it requires is present. The four modes, the consumer set
  and the pairing rules frozen in spec 001 §3.10 are unchanged.
- **Every refusal an author can cause carries three things**: the plasmid id, the field path in
  the declaration, and a `fix` sentence holding the line the author would write. The closed error
  code table in spec 001 §1 is unchanged; `fix` is one added structured field on the refusals an
  author can reach.
- **A capability denied at the boundary is reported as a missing declaration.** The report names
  what was denied and the declaration field it belongs in, contains nothing about what would have
  been allowed, and is written to the cell's session log and the plasmid's status rather than
  returned to the caller. The denial's own behavior is unchanged.
- **Attach is all-or-nothing.** On any failure the cell holds no record of the plasmid, no tool of
  its own, and no grant made on its behalf.
- **A plasmid is reported active only when every tool its declaration names resolves.**
- **Attach grants nothing the declaration does not name**, and refuses rather than granting more.
- **Detach requires nothing from the plasmid** and leaves the cell holding what it held before
  attach.
- **`plasmid new <name>` writes exactly one file, the declaration**, grants nothing, attaches
  nothing, and prints the sections the author must add. It writes no component and says that it
  will not until the plasmid interface is frozen.
- **The generated path and the hand path produce the same artifact.** No field, section or value
  is reachable from only one of them, and the kernel reads no field recording which produced it.
- **Nothing here decides the gate.** Two properties bound it and no more: the approval is not the
  cell's to give, and the declaration is what it reads.

## Acceptance

- A declaration with no `description` is refused, and the refusal names `description`.
- A `[provides]` binding naming a tool with no sentence is refused, and the refusal names the
  tool.
- The list form of `tools` is refused, and no manifest anywhere in the tree still uses it.
- A secret ref with a `consumer` and no `delivery` parses, and its derived list is asserted for
  all four consumers — including both `http`/`process` branches, one ref carrying a `path_scope`
  and one not.
- Every derived list is passed through the frozen pairing validator in the same test and accepted,
  rather than being compared to a table by eye.
- An explicit `delivery` still parses; an illegal explicit pairing is still the same named error
  it is today.
- Each of these refusals carries the plasmid id, the field, and a non-empty `fix`: missing
  `description`; a tool with no sentence; an illegal delivery pairing; `inject` with no path
  scope; a relative path-scope entry; a network section with no hosts; a declaration naming no
  capability at all; a command secret naming no subject.
- A plasmid denied a host it did not declare produces a report naming that host and the hosts
  field, and that report contains none of the hosts the plasmid *was* granted — asserted by
  searching the report for the allowed host and finding it absent.
- The denial report reaches the session log and the plasmid's status, and the calling code inside
  the plasmid sees the denial unchanged.
- An attach failed partway leaves no trace: the cell lists no such plasmid, the tool registry
  holds none of its tools, and the ledger records no grant for it.
- A plasmid one of whose declared tools does not resolve is not reported active.
- `plasmid new <name>` writes exactly one file, creates nothing else, exits non-zero, and prints
  the names of the sections the author must add.
- The file `plasmid new` writes is refused by the parser, and the refusal names the same sections
  the command printed. The existing test asserting that `plasmid new` creates nothing at all is
  replaced by this pair rather than kept alongside it.
- A declaration typed by hand and one produced by the scaffold for the same inputs are the same
  file, and no field in the grammar is settable only through the scaffold.
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
  properties named above are the only ones this spec asserts about it, and they were derived
  rather than chosen.
- **The registry.** Finding a plasmid somebody else wrote and publishing your own is intent 014
  and a different goal. This spec says what the thing in a registry would be, not where it lives.
- **The skill an agent inside a cell invokes to generate one.** Intent 010 names that path as the
  common case; what the skill is made of is downstream of this contract rather than part of it.
- **Mock modes and their layering.** Frozen in spec 001 §3.10 and untouched.
- **The credential grammar itself.** The four delivery modes, the consumer set and the pairing
  rules stay exactly as spec 001 froze them. This spec only lets an author leave the choice out.
- **Capability requirement and version selection between plasmids.** The kernel already selects a
  version across providers; turning that into an author-facing surface — how a requirement is
  written, how a conflict is explained — is its own question and its own spec.
- **The `[commands]` section's shape.** Reserved in spec 001; an author declaring a command still
  writes it as it stands today.
