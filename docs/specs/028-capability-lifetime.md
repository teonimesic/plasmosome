---
id: 028
title: Capability lifetime, and what ends a grant without being asked
status: accepted
intents: [012, 004, 003]
---

## Behavior

Every plasmid that holds a capability grant has a lifetime, and when it runs out the kernel takes
the grants away by itself; the agent inside is never asked. With nothing declared, it is a 15-minute
lease from the attach's prepare; a plasmid that only carries software has none of its own. The
operator or the author may declare a shorter lease or an idle limit, a request may tie it to one
`cell.exec` process, and only the operator may say "until revoked". Status shows the time left.
Only `plasmid.renew`, a recorded operator verb, adds time, within the ceiling set at attach.

Expiry has two steps. At the bound, the cell's supervisor closes every grant of the plasmid at its
host gate, so nothing gets through on any reference, new or already open, with no timer and no
controller; to the workload, a fenced grant looks like a withdrawn one. Then the controller removes
the plasmid through the exact removal `plasmid.remove` uses: drain, then force only what did not
drain. A grant reference still effective after that is named with its owner until it goes. A
removal that stalls or fails is reported that way, never as success.

This serves intent 012: a capability exists only while it is needed, and the agent is the last
thing relied on to give it up. Intent 004: expiry removes exactly the expired plasmid's holdings.
Intent 003: a bound survives a controller crash and a host sleep; a host restart retires the cell
(spec 023). It pulls against intent 011: a short default makes revocation a routine event a
running harness sees as an ordinary error (question 1). Today nothing revokes automatically.

**Levels.** Darwin arm64 is the only product host. Each acceptance item names the lowest level that
proves it: the **model** (controller, journal, fake backend, injected clock), the **gate model**
(the supervisor's admission check on an injected clock), or the **pinned runtime** (a booted
repo-built guest with spec 017's adapters). That waits on owner decisions O-1 (the instance root's
location), O-6 (libkrun's library search path) and O-7 (the guest policy mechanism).

## Contract

### 1. What carries a lifetime

A lifetime belongs to an **attachment**: one plasmid in one cell, one change in spec 008's journal.
All its grants share one bound and expire in one exact removal. No single grant has its own:
expiring one of several would leave the plasmid half attached, which spec 011 forbids.

| Limit | Ends the attachment | Enforced by |
| --- | --- | --- |
| `lease_ms` | at a bound fixed at prepare; 1,000 ms to 30 days | the gate |
| `idle_ms` | when no request was admitted for this long; 1,000 ms to 30 days | the gate |
| `exec` | when one named `cell.exec` process of the same cell ends (draft 024) | the controller |
| `until_revoked` | only by removal | nothing |

Every lifetime except `until_revoked` has a lease as its hard ceiling. An idle limit or exec scope,
either of which may accompany `until_revoked`, only ends it earlier, and use never postpones the
lease. An exec scope decides *when* the grants end, not *who* uses them: while it runs, every
process in the cell can reach them (draft 024 §5).

The **bound** B is the moment a lifetime ends. A lease's B is fixed at prepare as two readings
(section 3); time before activation comes out of the lease. An idle limit or exec scope that ends
the lifetime first moves B earlier. D, the drain budget, is the declaration's `drain_ms`, 1 to
30,000 ms, or 5,000 ms when none is named. **H = B + 2 s + D** bounds grant references.

### 2. Who declares it, and the default

- **The operator**, in spec 022's operator declarations file: each plasmid entry gains an optional
  `lifetime`, `{"lease_ms": N}` with optional `"idle_ms"`, or `"until_revoked"`, which no other
  source can say.
- **The author**, in `[lifecycle]` next to `drain_ms`: optional `lease_ms` and `idle_ms`, which
  only shorten, because the author may be the agent inside (intent 010). Spec 011 refuses an
  unknown field with 108 naming it and a `fix`; under section 10's 011 change it also refuses these
  fields and `drain_ms` out of range. The parser does neither yet (plasmosome-pe2), and an ignored
  field cannot lengthen a lifetime.
- **The request.** `plasmid.add` takes optional `lease_ms`, `idle_ms` and `exec_id`; `cell.new` with
  a genome takes the first two. Each is one flag under spec 025. They only shorten, and cover every
  attachment the request creates, providers it pulls in included. An `exec_id` whose exec is not
  `running` is 105 `{from: <state>, to: "running"}`, and an unknown one 101 `exec <id>`; neither
  prepares anything.

The operator's lifetime is the **ceiling**. When the operator names none, an attachment holding a
capability grant gets the **default ceiling, a 900,000 ms (15-minute) lease**, and one that only
carries software (draft 027) gets none. The lease is the smallest of the ceiling and any author or
request lease; under `until_revoked` there is no ceiling, but such a lease still applies. The idle
limit is the smallest any source names. Nothing defaults to `until_revoked`. Resolution happens in
spec 022's single read of the operator file, and the journal records the ceiling. A reload does not
re-resolve or change the lifetime, as spec 022's reload keeps the held mode, and recovery, expiry,
renewal and withdrawal never open the file. So a workspace Mount on the default lease expires under
a running harness; an operator who wants it kept declares a longer lease or renews.

**Closures.** At attach, a requirer's bound is cut down to each provider's; a software-only provider
has no bound to cut it to. When a provider's lifetime ends, for any cause, every attached plasmid
that transitively requires it, software-only ones included, expires in the same removal with cause
`provider`, and so does every provider `plasmid.remove` would take with them (spec 011). A requirer
never outlives its provider, and an expiry strands nothing.

### 3. Two clocks, and a clock tests can drive

A lease bound is two readings. `not_after` is a UTC wall-clock instant. `continuous` is a deadline
on a clock that counts while the host sleeps and that setting the wall clock does not move, tagged
with the host boot session (`kern.bootsessionuuid`, as in spec 023). B has passed once **either**
reading has, so a wall clock stepped back cannot extend a grant, one stepped forward ends it early,
and sleep extends nothing. A continuous reading from another boot session is ignored.

On Darwin it is `clock_gettime(CLOCK_MONOTONIC_RAW)`. This spec's reviewers measured it equal to
`mach_continuous_time` on macOS 26.6.2, which the SDK header documents as "like
mach_absolute_time, but advances during sleep"; the manual page calls `CLOCK_MONOTONIC_RAW`
unaffected by time adjustments. There `CLOCK_MONOTONIC` equalled wall time minus `kern.boottime`, so
it is not used, and Rust's `Instant` reads `CLOCK_UPTIME_RAW`, which stops during sleep. Linux would
use `CLOCK_BOOTTIME`. No sleep or wall-clock step has been measured yet; item 14 measures both.

**The clock is injected.** The controller and supervisor read time only through an injected clock
with three readings that tests set independently: wall, continuous with its boot session, and the
clock their waits use. The controller starts an expiry's removal **within 2 s of B, or of its own
next run** if it was down or the host slept.

### 4. Expiry: the fence, then exact removal

**The fence.** The supervisor gets each grant's bound and idle limit with its apply (spec 001
§4.1), before activation. It takes an earlier bound at any time, and a later one only from a
published DesiredCell (section 8). From B on, the host gate of every grant in the attachment admits
no request, on any reference, opened before B or after. The gate reads the supervisor's clock at
every admission, as spec 001 §4.2's graceful pause does, so the fence needs no timer and no
controller. It refuses before any OS effect, with spec 001's `grant_inactive`. It caps each admitted
request's work at B + D, and when B moves earlier, work already admitted is cut to the new B + D.

For the idle limit, the gate keeps the continuous time of the last request it admitted for the
attachment; a request still in flight is not use, and sleep counts as idle. It fences once that time
plus `idle_ms` has passed, checked at each admission and each observation, and the supervisor's
observation reports the idle deadline. With equal peers, fresh use goes to the smallest admitting
`GrantId` (spec 001), so the other can go idle.

The fence is permanent: no restored pause, late drain reply, renewal, publication or use reopens
it. `grant.select` treats a fenced grant as closed, so an equal peer of another attachment serves
fresh lookups (spec 017). The fence withdraws and finishes nothing, so it is not a removal path.
To the workload a fenced grant is a withdrawn one: draft 024 §9's table (at `c92e557`) says what
each grant kind sees, and when, and this spec restates none of it. The one difference: a fenced
Mount with no peer stays in `mountinfo` until removal finishes, with no bound while the controller
is down.

**Exact removal** starts when B passes on the controller's clock, when observation shows an idle
deadline passed or a gate fenced before the journal's B (section 8), or when a scoping exec ends:
`exec.status` reports `exited` or `signaled`, or answers 101, `-32603` or `no_reply`, so an unknown
exec fails closed. The controller observes a cell with an idle limit, and asks about a scoping exec,
at least once a second. For an exec or `provider` cause it first lowers the attachment's bound at
the supervisor, which fences it.

It is the transaction `plasmid.remove` performs (spec 001 §3.11, spec 008): a prepare with a null
replacement for the plasmid and every attachment section 2 takes with it; exact withdrawal of each
recorded effect, LIFO, with a graceful `DrainSpec` of D; a finish only after fresh observation
proves them absent. It enters spec 023's one-at-a-time order as a mutation that arrived at B, and
withdraws only this attachment's holdings; spec 017 keeps every equal peer.

**Escalation (question 2).** By default the bound authorizes Force for exactly the holdings whose
graceful withdrawal returned `DrainTimedOut`. The prepare records it as `force: {"expiry": {plugin,
cause, bound}}`, not an operator assertion, and spec 008's audited `force` event comes before Force
is used. The supervisor accepts an expiry Force only for a grant its own gate fenced and whose drain
timed out, so a controller fault cannot make it a general Force. A bound never discharges an
external assertion: an attachment with one outstanding is not prepared, and stays fenced as
`external_pending` until an operator removes it with Force (spec 001 §3.11). Only records carried
forward can hold one, since spec 008's writer refuses new external effects.

**What is recorded.** The journal records the transaction as for `plasmid.remove`. The session log
gets one `expiry` event per attachment: cell, plugin, generation, cause (`lease`, `idle`, `exec` or
`provider`), both readings of the bound, when the fence was first observed and removal started, and
each `GrantId`'s outcome (`drained`, `escalated` with its deadline, or `incomplete` with its
detail), never a count. The event stays after the finish, when the plasmid leaves the list.

### 5. Held references, and their bound

A revoke removes reachability, not objects already held, so what a reference holds decides how it
ends.

**Grant references** (spec 017's five classes) are bounded by spec 017's mediation: every managed
file, mount, stream and flow passes a host check on every request, and managed files cannot be
mapped (O-7), so after the fence every request on a reference opened before B is refused. The
contract term is **until the last reference goes**: IO admitted before B may finish within D, and
escalation forces what remains by H. IO the operating system cannot interrupt stays an
`IncompleteEffect` until it ends, named with its owner `{cell, plugin}` and its `GrantId`. A grant
reference effective after H is **residue**: the expiry returns `incomplete` naming it, the journal
keeps the removal unfinished until fresh observation proves it absent (spec 008), and the cell's
observation names it until it goes.

**Software-tree references** (draft 027) hold code bytes, not authority. The tree's detach is
complete when its names are gone. Surviving references are reported in draft 027's shape, a list of
`{path, pid, comm, exec_id?}`, in the expiry event and then in the cell's observation while the
guest's scan finds any, stopping when it finds none or the cell stops (draft 027 §4). A descriptor
in flight over `SCM_RIGHTS` is in no table and is not named. These references are not residue, hold
nothing unfinished, and leave readiness alone.

**What the fence does not bound.** It bounds IO through the data channel. A Broker is a host process
acting for the cell: work it accepted before B goes on until removal ends it (spec 017), or never
while the controller is down; an equal peer can keep the same process alive, and it can outlive a
SIGKILLed supervisor (spec 023, O-10). Spec 022's version 1 declares no Broker; the spec that gives
Broker a source must bound its accepted work by H.

### 6. When removal fails or stalls

Status (section 9) shows an unfinished expiry in one of these states. The names are provisional;
what is fixed is that none reads as success, and every forced or surviving `GrantId` is named.

| State | Meaning |
| --- | --- |
| `fenced` | B has passed and removal has not started: the controller is down, or the cell is busy |
| `external_pending` | an operator assertion is outstanding; nothing is prepared until Force |
| `draining` | graceful withdrawal is in progress |
| `escalated` | the named holdings timed out and are being forced |
| `incomplete` | Force left an `IncompleteEffect`, a reference survives H, or a software name could not be removed (draft 027); each is named, a name by its path |
| `failed` | the transaction was refused, or its audit failed (spec 008); the refusal is named |

The plasmid's state is `draining`. Failure never keeps or restores a grant, and its gates stay
fenced throughout. Before a removal commits (`fenced`, `external_pending`, a refusal, or a failed
`force` audit), the committed desired state still holds the attachment's `lifetime` record (spec
008's `Replacement.lifetime`, section 10), and every publication carries it: for a lease its bound
has passed, and for another cause the permanent fence holds and observation shows it. A failed audit
leaves a prepare without commit, which startup aborts (spec 008) and section 7 expires again. A
removal stuck after commit stays committed without finish (spec 008, publication step 5) until a
retry or Force resumes it. While any expiry is unfinished past its H, `plasmosome.status` reports
`ready: false`. A `cell.kill` resumes an unfinished expiry before its own withdrawal (section 10).
Forced and clean removals by the existing verbs still render alike, outside this spec; expiry
records differ.

### 7. Restarts, and a missing supervisor

- **Controller down.** The fence holds at B, and an idle fence at the next admission: the gates
  belong to the supervisor (spec 001 §4.2). Admitted work stops by B + D; a Broker's does not, and
  an exec-scoped grant stays reachable until its lease bound. After startup the controller
  reconciles each cell's DesiredCell under spec 008, which carries the journal's bounds, then
  expires every attachment past its B, with the bound from the journal, before any other mutation
  of that cell. The expiry event records the delay.
- **Supervisor gone.** Spec 023 never relaunches one. Every gate's host side dies with it, and the
  guest closes its own admissions when it sees the loss (spec 001 §4.2). The cell is unaccounted
  (spec 023 §8) and never mutated, so its expiries cannot run; `plasmosome.recovery` shows each
  holding's recorded bound. After a host reboot, spec 023 retires the cell.

### 8. Renewal

`plasmid.renew` takes `kernel`, `cell`, `plasmid`, exactly one of `not_after` (an RFC 3339 UTC
instant) or `until_revoked: true`, and `operator` and `reason`, nonblank as for Force. A renewal
only moves the bound later, and never past the ceiling recorded at attach: `not_after` is at most
that ceiling from now, and `until_revoked` needs an `until_revoked` ceiling. Anything else, or a
bound earlier than the one in force, is `-32602` naming the limit. A requirer is never renewed past
a provider's bound: that is 103, with the provider in `plasmid` and the capability it provides in
`capability`. A `not_after` equal to the bound in force writes nothing and replies as before, so a
repeat after spec 025's exit 3 is safe.

A renewal is a spec 008 transaction whose replacement changes only the lease; an idle limit or exec
scope stays. After its prepare is durable, a session-log `renew` event records operator, reason,
and old and new bounds; like `force`, it records authorization, not completion. Then come commit,
finish and publication. The supervisor raises a gate only from a published DesiredCell, so a raise
always follows a durable commit and no gate is later than the journal. If B passes before the
commit, the controller appends abort and replies 105 `{from: "draining", to: "active"}`. If a gate
fences between commit and publication, the fence stays, and the controller expires the attachment
with cause `lease` and replies the same 105. Renewal and expiry share spec 023's arrival order: a
renewal arriving after B gets that 105 before prepare, and one still running at B fails.

Renewal is never self-service. No guest lane (4090 or 4091) changes a bound, and spec 023 §9 keeps
the workload off the control socket (O-2). That holds only while no cell reaches a same-UID deputy
of that socket: a grant whose upstream is the socket, a local service that drives the command line,
or a host agent running it (spec 025 allows one) on output read from the cell. The socket proves a
UID, not a person, so `operator` is an assertion. The ceiling bounds each renewal, but repeated
renewal has no total limit (question 3). A channel for an agent to ask for time is RESERVED.

### 9. What status shows

Each per-plasmid object in `cell.status` (spec 001 §3.6) and `plasmid.list` (§3.9) gains `lifetime`,
always present. It holds `kind` (`lease`, `until_revoked`, or `none` for software with nothing
declared, so forever never looks like a missing field); the limits, ceiling and `not_after`;
`remaining_ms` from the reading with less time left, never below 0; `source` (`default`, `operator`,
`declaration`, `request` or `renewal`); the renewal count; any `exec_id`; and during expiry, its
state, cause and named `GrantId`s. The names are provisional.

### 10. Changes proposed to other specs

These are proposed text. The PR that accepts this spec edits no other accepted document. Each
amendment is applied by a later reviewed change to the document it amends, before any task that
relies on it. Where an item names a section of this spec, that section's sentences are its text.

- **Spec 001.** §3.6 and §3.9 gain section 9's `lifetime`; §3.5 and §3.10 section 2's request
  fields; §3 section 8's `plasmid.renew`, with no new error code, and 103 also refuses a renewal
  past a provider's bound. §4.1: an apply carries the bound and idle limit; a published DesiredCell
  carries each attachment's `lifetime`; the controller may lower a bound at any time; and
  `membrane.cell.observe` reports each grant's bound, idle deadline and fence. That is host-gate
  state the supervisor observes itself, never a `GuestObservation` or `GuestBinding` field. §4.2:
  section 4's fence, work cap and expiry Force check. §3.3: readiness is false while an expiry is
  unfinished past H.
- **Spec 008 (format 1, not yet written).** `Replacement` gains a required `lifetime` record (both
  readings, limits, ceiling, sources, renewal count); the session log gains `expiry` and `renew`;
  overdue expiries come first after startup. Under question 2's default only, `force` gains the
  form `{"expiry": {plugin, cause, bound}}`.
- **Spec 011, declaration.** Replace "**How long it may take to stop** — a drain budget." with
  "**How long it may hold its grants, and how long it may take to stop** — a lifetime and a drain
  budget (spec 028)." `[lifecycle]` gains `lease_ms` and `idle_ms`, refused outside section 1's
  ranges, and `drain_ms` is refused outside 1 to 30,000 ms. After "the drain window its declaration
  asks for", add ", up to 30 s".
- **Spec 011, detach.** This is the one text for these lines. It includes draft 024's proposed
  insertion verbatim, so the two land as one change, and draft 027 cites it. In the Behavior bullet
  (`011:195-207`), replace everything from "What the contract holds instead is the bound" to the
  bullet's end with:
  > What the contract holds instead depends on what the reference holds. A reference into a grant
  > always has an owner, the cell and plugin holding the grant and its `GrantId` (spec 028), and is
  > named against that owner until it goes. One held past its bound is not a detach failure — the
  > bound expires after the detach has returned — but an obligation spec 017's exact removal keeps
  > afterwards. A reference whose backing has been revoked, so that every use of it fails with an
  > ordinary error, refused by the guest kernel when the grant had its own connection and by the
  > guest file server when a peer shares it, reaches nothing and is not a surviving reference (spec
  > 024). A reference into attached software holds code bytes, not authority: it never fails or
  > delays a detach, and each one the guest's scan finds is reported with its path, holding
  > process, command name and any `exec_id` (spec 027). A descriptor in flight over `SCM_RIGHTS`
  > is in no table, so no scan names it.

  In the Contract bullet (`011:359-363`), replace the three sentences from "Detach returns when no
  new reference can be obtained" to "named at detach." with:
  > Detach returns when no new reference can be obtained; a reference taken before it keeps its
  > object alive. A reference into a grant is named against its owner, `{cell, plugin}` and its
  > `GrantId`, until the last one goes; one held past that bound is not a detach failure — the
  > bound expires after the detach returns — but an obligation spec 017's removal keeps. A
  > reference into attached software never fails or delays a detach, and is reported while the
  > guest's scan finds it; a descriptor in flight over `SCM_RIGHTS` is not.

  In the acceptance (`011:434-437`), replace the bullet "A detach whose surviving object has no
  owner **fails**" with:
  > - A reference into a grant taken before a detach is refused on its next use after the detach,
  >   and the same use succeeds before it; the cases differ only in the detach. One still effective
  >   past its bound leaves the removal unfinished, named with `{cell, plugin}` and its `GrantId`;
  >   the same staging with it closed before the bound finishes.
  > - A reference into attached software that survives a detach leaves the detach complete and is
  >   reported with its path and holding process; the same staging with it closed reports none.
- **Spec 017.** Restoring admission after a graceful timeout never lifts a lifetime fence. Under
  question 2's default, "Spec008 requires its durable operator assertion before forced cell
  cleanup" gains "or, for a holding whose drain timed out after its lifetime's bound, the recorded
  expiry authority (spec 028)".
- **Spec 022.** §4's operator entry gains section 2's optional `lifetime` in version 1, before it
  ships; §2's single read resolves it; §8: a reload does not re-resolve it; the refusals cover its
  ranges. Once UdsSocket has a source, refuse an upstream that is the instance's control socket.
- **Spec 023.** In §7 step 2, after "in one removal transaction under spec 008", add ", after first
  resuming any unfinished transaction of the cell, such as a spec 028 expiry, since spec 008 starts
  no later transaction until it finishes".
- **Spec 025.** The command table gains `plasmid renew` (`plasmid.renew`, changes state, not
  served). §2 gains "A boolean parameter is a bare flag, such as `--until-revoked`."

**Requests to draft specs.** Draft 024 (#127): apply its proposed 011 insertion as part of the 011
detach text above, not separately. Its §9 table at `c92e557` answers everything else 028 needs.

## Open questions

Each has the default this spec uses until the owner answers.

1. **The default.** Default: a 15-minute lease, no idle limit, a 5 s drain, and caps of 30 days and
   30 s. A short default makes revocation routine: a workspace Mount expires under a running
   harness, which gets an ordinary error intent 011 would rather it never saw, and renewal fatigue
   pushes operators toward `until_revoked`. Are these right?
2. **Force at expiry.** (a) Default: a bound forces only holdings whose drain timed out, and
   external assertions wait for an operator; this needs section 10's 008 and 017 changes. (b) A
   bound also discharges external assertions: also add "or, at a lifetime's bound, the recorded
   expiry authority (spec 028)" after spec 001 §3.11's "force requires the operator/reason pair
   exactly as `plasmosome.stop`" and spec 011's "force requires the operator/reason pair", and "and
   its external assertions" to (a)'s 017 sentence. (c) No expiry Force: a timed-out holding stays
   fenced until an operator forces it; no 008 or 017 change.
3. **Who renews.** Default: any control-socket caller with an operator and reason, each renewal
   capped at the attach ceiling. A host agent running the command line on cell output can then
   renew again and again, since repeated renewal has no total limit.
4. **Asking for more time.** Should a cell get a channel to request a renewal, never a grant?
   Default: no channel (RESERVED).
5. **Software-only plasmids.** Should they expire too? Default: no; they hold no authority, though
   one that requires a leased provider expires with it.

## Acceptance

Each item names the broken implementation it catches. Lease, idle and exec items run on the
injected clock, and none sleeps for the duration it tests.

1. **The default, at the boundary (model).** One grant, nothing declared, 5 s between prepare and
   commit: both readings are prepare + 900,000 ms, in the journal and in the gate's observed bound,
   and `remaining_ms` falls with the clock. At B − 1 ms no removal has started; with the wait clock
   at B + 2 s, one has, and status shows `draining`. A software-only plasmid shows `kind: "none"`
   and never expires on its own. Catches: no default, a forever default, a bound from commit or
   from the supervisor's apply, a default on software.
2. **The narrowest wins (model).** Operator `until_revoked` with author 600 s gives 600 s, source
   `declaration`. Under operator 300 s, author 1,200 s gives 300 s, a request for 120 s gives 120 s
   (`request`), and one for 3,600 s gives 300 s. With no operator lifetime, author 3,600 s gives 900
   s (`default`). Catches: last writer wins, and an author or request lengthening a lifetime.
3. **Only the operator says forever (model).** A request with `until_revoked`, or a lease under
   1,000 ms or over 30 days, is `-32602` and prepares nothing; the operator file's refusals are
   spec 022's, and its `until_revoked` shows as that `kind`. Once plasmosome-pe2 and section 10's
   011 change land, `[lifecycle]` with `until_revoked`, an unknown field, `drain_ms` of 0 or
   30,001, or `lease_ms` of 999 is 108 naming it. Catches: an author or caller granting itself
   forever, and a field typed but never range-checked.
4. **Expiry removes exactly the plasmid (model).** X (60 s) and Y (600 s) hold equal capabilities in
   one cell. At 59 s both stand. At 62 s the journal's prepare holds exactly the changes
   `plasmid.remove X` would, X's holdings are gone, and Y's stand with their original IDs. After the
   finish X leaves `plasmid.list`, and its `expiry` event stays. Catches: removal by resource
   description, a path that skips the journal, a wider expiry.
5. **Closures (model).** R (600 s) requires P (300 s): R's bound is P's, and one transaction removes
   both. With P `until_revoked`, R keeps 600 s. An idle or exec end of P removes R, cause
   `provider`, and so does P's lease end for a software-only R. R's expiry removes a P attached
   only for R. A 120 s lease on `plasmid.add R` gives the P it pulls in 120 s. Renewing R past P is
   103 naming P. A reload of P keeps its bound. Catches: a stranded requirer, an orphaned provider,
   a reload moving a bound.
6. **Clean and forced differ (model).** Two expiries staged identically, except one holding of the
   plasmid stalled with `mark_stuck(handle)`. The clean record names every `GrantId` `drained`; the
   other names exactly the stuck one `escalated` at 5,000 ms and its peer of the same plasmid
   `drained`. Catches: forced and clean alike, attribution by count, forcing every holding.
7. **Force is audited and checked (model; gate model).** One holding is stalled with
   `mark_stuck(handle)`. Kill the controller between the expiry's prepare and its `force` event,
   then restart: startup aborts that generation, and the next expiry generation's `force` event
   comes before its forced withdrawal of the stuck holding. With the audit write failing, the stuck
   holding stays standing and fenced, nothing is finished, and status shows `failed`. The gate
   refuses an expiry Force for a grant it has not fenced, or whose drain did not time out. At B, an
   attachment with a forensic external assertion stays fenced and `external_pending`, unprepared,
   until `plasmid.remove` with an operator's Force. Catches: Force before its audit, the expiry
   form as a general Force, a bound discharging an operator's assertion.
8. **Failure is not success (model).** Force leaves an `IncompleteEffect`: status names it with
   `GrantId` and owner, no finish is written, readiness is false after H, and retries resume the
   committed generation; with Force succeeding, it finishes and readiness returns. A `cell.kill`
   queued meanwhile resumes the expiry first. Catches: success over an incomplete effect, a retry
   under fresh IDs, a kill stuck behind an expiry.
9. **The fence (gate model).** With bound B, an admission at B − 1 ms on both readings passes; one
   with only the wall reading at B is refused, and so is one with only the continuous reading at B
   and the wall an hour back. A graceful pause restored after B admits nothing; restored before B,
   it readmits. IO on a handle opened before B is refused after B before any OS effect. Work
   admitted at B − 1 s is capped at B + D. An equal peer with a later bound serves a fresh lookup.
   Catches: one reading checked, a reversible fence, a check only at open, uncapped work.
10. **Idle (model; gate model).** Idle 60 s, lease 600 s. Use at 0, 50 and 100 s, then no admission
    attempted: removal has started by 162 s and status shows `draining`. Use at 161 s is refused and
    revives nothing; a use at 155 s keeps it open at 160 s. A `stream.read` admitted at 100 s and
    still blocked is not use, and ends by 160 s + D. Use every 10 s still ends it at 600 s. Catches:
    idleness checked only on IO, in-flight work as use or cut only at the lease, revival, use
    extending the lease.
11. **Exec scope (model with a 4090 test double; gate model).** A 600 s lease scoped to `e-1`: when
    the double reports it `exited` at 30 s, removal has started within 2 s, and the same for
    `signaled` and for `exec.status` answering 101, `-32603` or `no_reply`; while it runs nothing
    starts. An unknown or exited exec refuses and prepares nothing; `exec_id` on `cell.new` is
    `-32602`. With the controller stopped, the gate fences at 600 s. Catches: a scope that never
    fires or fails open, a scope with no ceiling.
12. **Controller restart (model; gate model).** Stop the controller before B, advance past B,
    restart. The gate refused from B. The cell's first new record is the expiry's prepare with the
    original bound, and its event names the delay; an edited operator file changes nothing. Catches:
    missed expiries, a bound reset at restart.
13. **Renewal (model; gate model).** Before B with an operator and reason, a renewal moves the
    bound, appends `renew` with both bounds and counts it; the same request again writes nothing.
    Without operator or reason, past the ceiling, or `until_revoked` under a leased ceiling:
    `-32602`; after the fence: 105, admission still refused. Kill the controller between the
    renewal's prepare and commit and keep it down past the old B: the gate refuses from the old B,
    and startup aborts the renewal and expires the plasmid. Lose the publication's reply and kill
    the controller: after restart each grant's observed bound equals the new journal readings.
    Catches: a raise before commit, a partial or stale raise, a reopened fence, renewal to forever,
    a repeat renewing twice.
14. **Clocks (model, then pinned runtime).** Advance wall and continuous 11 minutes with the wait
    clock still, then the wait clock 1 s: a 10-minute lease's removal starts; without the 11 minutes
    nothing does. Wall back an hour: it fires at the continuous deadline; wall forward past B:
    early. A bound from another boot session whose continuous deadline has passed and wall reading
    has not does not fire. Later obligation, pinned Mac, controller stopped: sleep past a 2-minute
    lease and wake; the first admission is refused, and the continuous reading advanced by at least
    the sleep's wall-clock length. Step the wall clock back an hour under a 10-minute lease; the
    gate refuses at the continuous deadline. Catches: `Instant`, a continuous reading derived from
    wall time, one reading checked.
15. **Held references (model, then pinned runtime).** In the model, a grant reference effective
    after H keeps the removal unfinished, is named with `{cell, plugin}` and `GrantId`, and makes
    readiness false; closed before H, the expiry finishes. A surviving software-tree reference is
    reported in draft 027's shape until the scan finds none, and the expiry finishes with readiness
    true. On the pinned runtime, controller stopped, with a Mount and a ProxyMap: a descriptor and a
    working directory opened before B, and a TCP flow moving bytes both ways, succeed at B − 1 s.
    After B an operation started fails at once, and one in flight ends by B + D, each with draft 024
    §9's error for its kind once 024's task records it. With no peer, the Mount stays in
    `mountinfo` until removal finishes. Killing `membraned` leaves no grant reachable. Catches:
    revocation by name only, a fence that only removal provides, a hung call, tree references as
    residue.

## Out of scope

- How long a cell lives (spec 023), and revoking bytes or secrets already delivered into a cell.
- A credential's own `ttl`: when custody lands (spec 022), a minted credential must not outlive the
  attachment carrying it.
- Per-grant lifetimes, and task scope, which is RESERVED: the product has no task yet.
