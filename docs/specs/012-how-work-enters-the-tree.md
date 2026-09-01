---
id: 012
title: How work enters the tree — the chain, the gates, and what a pull request must carry
status: accepted
intents: [008]
---

## Behavior

**Somebody should be able to open this repository and find out, without asking anybody, whether a
change in flight was wanted, whether it was allowed to start, and whether anyone read it before it
merged.** All three answers exist today. Each of them can also come back yes when the truth is no.
They are spread across six documents — three skills, the root `AGENTS.md`, and the READMEs of two
folders — one of which already asks two incompatible things of the same small pull request. They are
read out of records that can stop being records with nothing saying so. And the rules that produce
them sit where the author of the change they would refuse can widen them.

The rules governing how work enters the tree are a layer of this repository like any other: the
chain from an intent down to a task, the gates on what may be started, the conditions a pull
request meets before it merges, and the records all of that is read out of. **This spec is the
contract that layer answers to.** It is not the rules. A rule written for the next agent to read
lives in `.agents/skills/`; what this says is which properties such a rule must have, so that one
added tomorrow can be checked against something instead of argued about.

### Every rule here is paired with something that can come back negative

`docs/intents/008-an-ai-native-way-of-building-plasmosome.md` asks one thing of any loop: a check
that can answer no, whose answer does not come from the agents being checked. A process rule
nobody can fail is a preference, and this is the layer where preferences are cheapest to add and
most expensive to keep.

So **a rule in this layer names which of three things can refuse it**, and a rule naming none of
them does not go in:

- **A sweep over the files** — mechanical, runnable by anybody, printing faults and nothing else.
- **A person's judgement, recorded where it is visible** — not remembered, and not self-attested by
  the agent whose work it gates. An agent may *record* a judgement the person actually made, and
  relaying one is ordinary work; what it may never do is originate the judgement and then cite it
  as the check on its own change.
- **A named failure with a count somebody can take** — concrete enough that a reader can go and see
  whether it stopped happening.

The third is the weakest of the three and is still a check, because it can be contradicted: "you
can tell whether this is working by whether an open pull request without a task survives a
heartbeat" is a claim the tree can refute. What is not a check is an agent's own report that it
followed the rule, or an approval it originated and then relied on. That is the answer coming from
inside the loop, which is the one shape intent 008 rules out by name — and it is a different thing
from carrying somebody else's answer, which is how approval reaches the tree at all.

### The chain is total, and the list of shapes carrying no task is closed

Every change that reaches `main` is reachable upward: the pull request names a task, the task names
a spec, the spec names an intent. Read that way it answers "was this wanted", which is the whole
point of the links. A change the walk cannot follow to a top has no answer to give.

**The walk ends at an intent, with one named exception that ends it a step earlier.**
`docs/specs/001-control-protocol.md` is `accepted` carrying `intents: []` and keeps its place —
`docs/specs/README.md` states that amnesty and `.agents/skills/tasks` says why. A change under that
one spec therefore walks up to the spec and stops, and its task carrying `intents: []` is a correct
field rather than an unfilled one. **The amnesty is one file and it is closed**: any *other*
accepted spec with an empty `intents:` skipped the gate rather than predating it, because nothing
distinguishes an old file from a new one claiming to be old. This spec adds nothing to that set,
and no clause below reopens it.

**Exactly two shapes carry no task, and both are structural.** They are not the same shape and
flattening them is the error to avoid. A pull request filing an **intent** has nothing above it at
all: the intent is the top of the chain. A pull request filing a **spec** does have something above
it — its `intents:` names the goal it serves, which `docs/specs/README.md` requires of every new
spec, and this spec's own frontmatter reads `intents: [008]` — but it has no *task*, because the
task rides the work branch that follows and cannot exist until the spec is accepted. What the two
share is the missing task, not a missing parent. Neither can name a task governing it without one
being invented for the purpose, which is precisely the move the gates exist to refuse.

**That list is closed here rather than in the skill, and the placement is the point.** An exemption
written where the author of a change can widen it gets widened. The previous one said "under about
twenty lines" and was in practice applied to a whole category at any size: three merged pull
requests carrying no task changed 173, 65 and 51 lines, all three of them edits to the skills. That
was not cheating. The rule sat in a file the exempted change was also editing, so it had no author
but the person it was refusing. Adding a third shape means editing this file, in a pull request of
its own, which is a different act from writing a paragraph into a skill the exempted change is
already open in.

**That is a claim about where a change is made, not about this file being hard to change.** A spec
records what was delivered, and one whose design turns out wrong is edited — ordinarily, by whoever
finds it, with the reason — exactly as `.agents/skills/pr-review` already requires when built
behavior and its spec disagree. Nothing here is frozen, and no clause in this spec draws authority
from being unchangeable. What the closed list buys is that widening it is one visible edit somebody
reviews on its own, instead of a sentence added to the document the widening change was editing
anyway.

**The tempting third shape, refused here by name: "no spec describes my area yet."** That is not
above the chain. It is the ordinary case of work needing a new spec, and the answer is to write one
under the intent that wants it — or, where no intent wants it, to put the question to the owner or
to drop the change and say why. Absence of a governing document is the chain not yet reaching
something, never permission to step around it. Read loosely, this readmits every change the size
exemption used to let through, which is why it is written as a refusal rather than left to be
inferred.

### The gates bind starting, not writing

Anything may be written down at any time, and working ahead is meant to happen: a draft spec under
a draft intent is a proposal on the record and costs one document to throw away. **What waits is
commitment.**

There are two gates on starting, and this spec adds none:

- A spec may not become `accepted` until the intent it names is `approved`.
- A task may not be started until the spec it names is `accepted`.

**That set is closed the same way the exemptions are, for the mirror-image reason.** A gate is
cheapest to add in the document describing the work it would delay, and a process layer that
accumulates gates stops being read at all. A third gate is an edit to this file, on the same terms.

### One gate belongs to a person, and it is a state rather than a courtesy

Approval is the owner's, and it is the only judgement in this layer that cannot be derived from the
tree — every other answer is a field somebody can read. An approved intent multiplies into specs
and each of those into tasks, so it is the point where one yes becomes work nobody has counted.

**Anything waiting on the owner waits in the open, as a draft pull request.** That is a state and
not a memory: `gh pr list` is the queue in front of them, and it does not depend on an agent
remembering to mention what it is waiting for. It carries down the chain unchanged — a change whose
chain reaches a goal nobody has approved is waiting on the same person, in the same place, for the
same reason.

**The wait is ended by the owner and by nobody else.** That is the contract this spec states. The
rule agents read — that an agent does not mark such a pull request ready, and never originates an
approval — is written in `docs/intents/README.md`, and
`docs/decisions/008-approving-an-intent-is-an-instruction.md` records why nothing mechanical holds
it, what the residual risk is, and what would reopen it. Neither is restated here and neither is
changed here.

### What a merged pull request has to carry

Three things. They are stated together because each has failed on its own:

- **A link upward** — a task, or being one of the two shapes that carry none.
- **A review that read the commit being merged** — not a review of an earlier head, and not a
  signal whose real meaning is queued, rate limited, skipped, or not started yet.
- **An answer to every thread**, by a fix or by a disagreement written down.

**The middle one carries a requirement about evidence, and that is the part worth stating
precisely.** A signal reading the same whether or not a review happened is not evidence of a
review, whatever its colour. The evidence has to bind to the commit that merges, and it has to tell
a review that found nothing from a review that never ran — the two states that look identical to
anything counting reviews rather than reading them. Which reviewer, which endpoint and which
command obtain that is the skill's business and changes when the tool does. That it binds to the
merged commit is this spec's, and does not.

This is not a rule about carelessness. Every signal named above fails in the same direction, toward
merging, and an agent doing exactly what it was told is the one that ships the unread change.

### A record nobody can read has left the queue

The record-validation rules in this section are enforced by reading files — not every rule above
is. Approval is a person's and is not derivable from the tree; the waiting state is GitHub's draft
flag; the review evidence is an answer the forge gives about a commit. Reading those off the files
would be reading them off the wrong thing. What files do carry is the records, and every list this
repository builds finds them by matching a line — `status:`, `specs:`, `intents:`. **A selector
like that fails open.** A
record written `status:todo`, or with a trailing space, or saved with CRLF endings, does not turn up
late in its queue. It leaves the queue, silently, and the count meant to notice is the thing that
stopped seeing it. That is the opposite direction from a gate, where a mismatch is a refusal.

So the layer carries two requirements about its own records.

**Each record declares its state once, from a set of values written down in one place.** One field,
one line, anchored at the start of it, one value from a closed set. A set written down twice
eventually disagrees with itself, and a sweep built on the wrong copy is worse than no sweep: it
clears the files that are wrong and reports the ones that are right.

**Each folder holding those records is swept for whether its state lines are well formed at all.**
A sweep is not a queue. It prints faults, silence is the only passing answer, and an empty input set
refuses rather than passes — a run that found no files to read is indistinguishable from a run over
a clean tree, which is the same shape as a green that reviewed nothing.
`docs/specs/009-how-much-of-an-intent-is-built.md` requires that floor of its own check for that
reason; this is the general form of it.

**Two of the three folders are swept today and the third is not.** `docs/intents/` and
`docs/specs/` are. `tasks/` holds 28 records, more than either, and every list built over it is a
selector with nothing behind it.

**The vocabulary is settled before the sweep is written, never by it.** When this spec was drafted
`.agents/skills/tasks` listed five task statuses and the tree carried four — no task file held
`in_progress` — so a sweep written then would have had to pick a side, and from then on the
disagreement would have been enforced rather than discussed. The owner has since ruled that
`in_progress` stays, so the set is the five the table lists and this particular question is closed.
What generalises is the shape: whether an executor records claiming a task changes what an agent
must write down mid-flight, which is a decision, and a check is the worst place to make one.

### One statement per rule

A rule in this layer is written in exactly one file, and every other mention points at it.
`AGENTS.md` already says as much — "a second copy of a rule is a copy that will disagree" — and the
tree does not hold it. The rule that a pull request waiting on the owner stays a draft is written
out in `AGENTS.md`, in `docs/intents/README.md` and in `.agents/skills/pr-review` step 2. The gate
on a spec becoming `accepted` is written out in `.agents/skills/tasks`, in `docs/specs/README.md`
and in `docs/intents/README.md`. The values an intent's `status:` may take are written out twice.

**None of those copies disagrees today, and that is the difficulty rather than the reassurance.** A
copy costs nothing until the day one of them is edited and the others are not, and it does not
announce itself before then; the copy a reader happens to open is the one they follow. So this
clause stands on that reasoning and not on an instance. Nothing on this tree has yet been measured
drifting between two copies of one rule, and saying otherwise would be the kind of claim this spec
exists to make contradictable.

What has been measured is the same reading failure by a shorter route. `.agents/skills/pr-review`
requires the commit being merged to be one a review read, and its own rounds table gives a change
under 100 lines a single round and says not to re-trigger; addressing a finding makes a new commit,
so a small pull request that finds anything cannot honour both. Those are two distinct rules, each
written once, in one file — not copies that drifted. They are cited here for what a reader meets,
which is two sentences that cannot both be followed, and copies are the commoner way to arrive at
it.

## Contract

- **The subject** is how work enters the tree: the chain from intent to spec to task, the gates on
  what may be started, the conditions a pull request meets before it merges, and the form of the
  records all of that is read out of. It does not reach the content of any individual rule — how
  many review rounds a diff size earns, which reviewer, which endpoint answers which question.
  Those change without this spec changing.
- **Every change reaching `main` is reachable upward to an intent**: pull request → task → spec →
  intent, or by the shorter route the two shapes below provide.
- **One named spec ends that walk a step early.** `docs/specs/001-control-protocol.md` is `accepted`
  with `intents: []` under the amnesty `docs/specs/README.md` states. A change beneath it reaches
  that spec and stops, and its task's `intents: []` satisfies this contract rather than breaching
  it. **The amnesty is one file and closed** — any other accepted spec with an empty `intents:`
  skipped the gate rather than predating it — and nothing in this spec adds to it or removes it.
- **Exactly two shapes carry no task**: a pull request filing an intent, which has nothing above it
  at all, and one filing a spec, which names the intent it serves but has no task until the work
  branch that follows it. **The list is closed, and adding to it means editing this file** in a pull
  request of its own — not a judgement available to the author of the change that would use it. An
  area no spec covers is not one of the two and does not become a third.
- **The gates on starting are two and no more**: a spec may not become `accepted` until the intent
  it names is `approved`; a task may not be started until the spec it names is `accepted`. Adding a
  gate means editing this file too. **Nothing gates what may be written down.**
- **Approval is the owner's**, and is the only judgement in this layer not derivable from the tree.
- **Anything waiting on the owner waits as a draft pull request**, so the waiting is a state anyone
  can list rather than something an agent has to say, and **the wait is ended by the owner, never by
  an agent**.
- **A merged pull request carries** a link upward or is one of the two shapes; a review that read
  the commit being merged; and an answer to every thread.
- **Evidence that a review happened binds to the commit being merged** and distinguishes a review
  that found nothing from one that never ran. A signal reading identically in both cases does not
  satisfy this.
- **Each record in the chain declares its state in exactly one field**, on one line, anchored at
  the start of the line, whose value comes from a closed set.
- **Each of `docs/intents/`, `docs/specs/` and `tasks/` is swept** for records whose state line is
  not well formed. A sweep prints one line per fault, prints nothing on a clean tree, and treats an
  empty input set as a refusal rather than a pass.
- **A sweep never settles what the legal values are.** Where the written set and the tree disagree,
  that is resolved before the sweep is written.
- **A rule in this layer names what can refuse it** — a sweep, a person's judgement recorded
  visibly, or a named failure someone can go and count. An agent's report that it followed the rule
  is none of the three.
- **A rule in this layer is written in one file, and each closed set of values is written in one
  place; every other mention is a pointer. This clause binds what a change adds or edits**, never
  what is already on the tree. Three rules carry copies today — the wait-as-a-draft rule in three
  files, the gate on a spec becoming `accepted` in three, an intent's `status:` values in two — and
  collapsing them is a backfill this spec names rather than schedules: a copy goes when the rule it
  copies is next edited, removed by whoever edits it. **A new copy is refused now.**
- **This spec is edited when it turns out wrong**, like any other, and no clause here relies on it
  being unchangeable. The closed lists above bind *where* a change is made, never whether one may
  be made.
- **Callers may rely on** this being a floor. Nothing here makes the chain unbypassable: every
  clause is a rule agents read, and
  `docs/decisions/008-approving-an-intent-is-an-instruction.md` records why the mechanical forms
  were turned down and what would reopen them.

## Acceptance

- `.agents/skills/tasks` states that every pull request is reachable upward to an intent, names the
  two shapes that carry no task, distinguishes them — an intent's pull request has nothing above it,
  a spec's names an intent and lacks only a task — and says the list is closed by this spec rather
  than by the skill.
- `.agents/skills/tasks` states that an area no spec covers is not one of those shapes, and says
  what to do instead.
- No skill file states a size, a kind or a category of change that is exempt from the chain. The
  two shapes above are not one: having no task to name is a different claim from being excused from
  naming what you do have, and the skill text has to make that difference visible.
  `grep -riE 'trivial|20 lines' .agents/skills/` finds the candidates; a hit is something to read,
  not a fault on its own.
- `.agents/skills/pr-review` states the three things a merged pull request carries, and states that
  the review evidence binds to the commit being merged rather than to the pull request.
- `.agents/skills/pr-review` step 2 states that a change whose chain reaches an unapproved intent
  stays a draft, and `.agents/skills/heartbeat` step 1 states that such a draft is not stalled
  work waiting on an agent.
- `.agents/skills/heartbeat` step 4 sweeps `tasks/*.md` for a well-formed `status:` line, alongside
  the two folders it already sweeps.
- The statuses that sweep accepts are exactly the set the `## Lifecycle` table in
  `.agents/skills/tasks` lists — five, `in_progress` among them, which the owner has ruled stays.
  Should the table and the tree disagree again, that is settled before the sweep is written and not
  by it, with the change saying where the answer came from. That provenance is a sentence a reviewer
  reads and not a line a script checks, exactly as an intent's approval is.
- The amnesty this spec names and the one `.agents/skills/tasks` names are the same one file,
  `docs/specs/001-control-protocol.md`, described as closed in both. No task under this spec widens
  it, and a task whose `specs:` names only 001 carries `intents: []` without that reading as an
  unfilled field.
- Each sweep prints nothing on the tree as it stands; prints the offending file for a record whose
  state line is absent, empty, duplicated, malformed, or outside the set, injected one at a time
  into a scratch copy; and refuses in its own words, non-zero, on an empty input set. **Both shapes
  of empty are tested** — the folder missing, and the folder present but holding nothing the sweep
  would read. The second is the one a missing-folder test walks straight past, and it is the shape a
  file match that has quietly stopped matching produces, which is the failure the refusal exists to
  catch. Verified under both `bash` and `zsh`, because a glob matching nothing is fatal in one and
  empty in the other.
- Every rule a task under this spec adds to a skill file names, in its own text, what can refuse
  it.
- No rule a task under this spec adds or edits is stated in more than one file; every other mention
  is a pointer. Where such a task edits a rule that already carries copies, the copies go in the
  same change.

## Out of scope

- **Writing the rules.** This spec is the contract; the skill text that agents read is the work
  under it, and lands in its own pull requests. A spec that also wrote the rules would be
  reviewed as one change with the thing it governs, which is the ordering the two-pull-request rule
  exists to keep apart.
- **The content of any one rule.** How many rounds a diff size earns, which reviewer runs, how a
  review is triggered on an unchanged commit, and whether `in_progress` survives are all decided in
  the skills and by the owner. This spec says what a rule must have, never what it must say.
- **A mechanical gate on the chain.** Nothing here asks CI to refuse a pull request that names no
  task. Both mechanical forms refuse real work — a spec's pull request touches no task file by
  design, and a body line lives somewhere the hooks cannot read — and a guard that refuses real
  work teaches everybody to reach for the bypass. The conditions are read by agents and checked
  again by the heartbeat.
- **The layers above the chain, and `docs/decisions/`.** Vision and architecture are not work
  entering the tree, and a decision is not a link in the chain — a task may cite one and it never
  stands in for the spec the task names.
- **How much of a goal has been built.** `docs/specs/009-how-much-of-an-intent-is-built.md` owns
  that question and the field that answers it. This spec is about work entering; that one is about
  what has landed.
- **A decision record.** What is argued down here — a size threshold, an exemption the author of a
  change can widen, a sweep that settles its own vocabulary — is argued in the prose above. The one
  that will be raised again is the exemption, because every process change meets it and every
  author has a reason. **If a third shape is proposed a second time, that is the trigger to write
  the record** rather than to re-run the argument from here.
