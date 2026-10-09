---
id: 028
title: Capability lifetime, and what ends a grant without being asked
status: draft
intents: [012, 004, 003]
---

## Behavior

Every plasmid attached to a cell has a lifetime, and when it runs out the kernel takes the
plasmid's grants away by itself: nobody has to remember, and the agent inside is never asked. With
nothing declared, it is a 15-minute lease from the attach's prepare. The operator or the plasmid's
author may declare a shorter lease or an idle limit, a request may tie it to one `cell.exec`
process, and only the operator may say "until revoked". Status always shows each plasmid's
lifetime and time left. Only `plasmid.renew`, an operator verb outside the cell that the session
log records, adds time.

Expiry has two steps. At the bound, the cell's supervisor closes every grant of the plasmid at
its host gate, so nothing new gets through on any reference, new or already open; it needs no
timer and no controller for this. Then the controller removes the plasmid through the exact
removal `plasmid.remove` uses: drain, then force what did not drain. A removal takes away
reachability, not objects a process already holds, so expiry also answers for held references:
each one still effective after the drain is named, with its owner, until the last one goes. A
removal that stalls or fails is reported that way, never as success.

This serves three intents. Intent 012: a capability exists only while it is needed, short and
automatic, and the agent is the last thing relied on to give it up. Intent 004: expiry removes
exactly the expired plasmid's holdings and no equal neighbour. Intent 003: a bound survives a
crash, a host sleep and a restart. Today nothing revokes automatically, in code or in the accepted
specs. The secret grammar's `ttl` is a credential's lifetime, not a grant's.

**Levels.** Darwin arm64 is the only product host. Each acceptance item names the lowest level
that proves it: the **model** (controller, journal, fake backend, injected clock), the **gate
model** (the supervisor's admission check on an injected clock), or the **pinned runtime** (a
booted repo-built guest with spec 017's adapters, which needs O-7).

## Contract

### 1. What carries a lifetime

A lifetime belongs to an **attachment**: one plasmid in one cell, which is one change in spec
008's journal. All the attachment's grants share one bound and expire together, as one exact
removal. A single grant has no lifetime of its own. Nothing needs one yet, and expiring one grant
of several would leave the plasmid half attached, which spec 011 forbids.

| Limit | Ends the attachment | Enforced by |
| --- | --- | --- |
| `lease_ms` | at a fixed bound after prepare; 1,000 ms to 30 days | the gate |
| `idle_ms` | after this long with no IO admitted or in flight; 1,000 ms to the lease | the gate |
| `exec` | when one named `cell.exec` process of the same cell ends (spec 024) | the controller |
| `until_revoked` | only by removal | nothing |

Every lifetime except `until_revoked` has a lease, and the lease is its hard ceiling. An idle limit
or an exec scope can only end an attachment earlier, and either may also accompany `until_revoked`.
Use can postpone an idle limit but never the lease, so an agent cannot keep a capability past its
lease by touching it. An exec scope decides *when* the grants end, not *who* may use them: while the
exec runs, every process in the cell can reach them (spec 024 §9).

The **bound** B is the moment a lifetime ends. A lease's B is fixed at prepare, before any grant
exists, and is recorded as two readings (section 3). Time spent between prepare and activation
comes out of the lease; it never adds to it. When an idle limit or an exec scope ends the lifetime
first, B moves earlier, to that moment. D is the drain budget: the declaration's `drain_ms`,
capped at 30,000 ms, or 5,000 ms if the declaration names none. **H = B + 2 s + D** is the bound
on held references.

### 2. Who declares it, and the default

Up to three sources declare a lifetime, and the narrowest one applies.

- **The operator**, in spec 022's operator declarations file. Each plasmid entry gains an optional
  `lifetime`: `{"lease_ms": N}` with an optional `"idle_ms"`, or the string `"until_revoked"`. No
  other source can say `until_revoked`.
- **The author**, in the declaration's `[lifecycle]` section, next to `drain_ms`: an optional
  `lease_ms` and `idle_ms`. The author may be the agent inside the cell (intent 010), so the
  declaration can only shorten the lifetime. There is no field for `until_revoked`; spec 011
  refuses any unknown `[lifecycle]` field.
- **The request**: `plasmid.add`, and `cell.new` with a genome, take an optional `lifetime` with
  `lease_ms`, `idle_ms` and `exec_id`. It can only shorten the lifetime. `exec_id` comes only from
  a request. An exec that is not `running` refuses with 105 `{from: <state>, to: "running"}`. An
  exec the cell does not know refuses with 101 `exec <id>`. Either way, nothing is prepared.

The operator's lifetime is the **ceiling**; when the operator names none, the ceiling is the
**default: 900,000 ms (15 minutes)**. The lease is the smallest of the ceiling and any lease the
author or the request names, so neither can lengthen the default. When the operator says
`until_revoked`, there is no ceiling: an author or request lease still applies, and without one the
lifetime is `until_revoked`. The idle limit is the smallest any source names; if none is named,
there is none. Nothing defaults to `until_revoked`. Resolution happens in spec 022's single read of
the operator file for each resolving request. Recovery, expiry, renewal and withdrawal never open
that file.

**Closures.** At attach, a requirer's bound is cut down to each provider's, so status shows the
bound that applies; a renewal that would pass a provider's bound is refused instead (section 8).
If an idle or exec limit ends a provider first, every attached plasmid that transitively requires
it expires in the same removal, with cause `provider`. Spec 011 refuses a detach that would strand
a requirer; expiry cannot refuse, so it takes the requirers too, which only narrows the cell.

**A reload never extends.** `plasmid.reload` resolves the lifetime again (spec 022 §8). Its new
holdings take the earlier of two bounds: the attachment's current bound, and the newly resolved
bound measured from the reload's prepare. A reload is never a renewal.

### 3. Two clocks, and a clock tests can drive

A lease bound is recorded as two readings. `not_after` is a UTC wall-clock instant. `continuous` is
a deadline on a monotonic clock that keeps counting while the host sleeps. It is tagged with the
host boot session (`kern.bootsessionuuid` on Darwin, as in spec 023). B has passed once **either**
reading has passed, so every clock fault fails closed. A wall clock stepped back cannot extend a
grant, because the continuous reading still reaches its deadline. A wall clock stepped forward
ends it early. Sleep extends nothing, because both clocks count it. A continuous reading from
another boot session is ignored: that guest is gone, and spec 023 §8 retires the cell.

On Darwin, the continuous clock is `clock_gettime(CLOCK_MONOTONIC)`, which Apple documents as
counting during sleep. A clock that stops during sleep must never measure a bound. Rust's
`std::time::Instant` reads such a clock on Darwin, and so does every timer built on it. A Linux
host would use `CLOCK_BOOTTIME`. Acceptance item 13 measures the chosen clock across a real sleep
on the pinned Mac rather than assuming it.

**The clock is injected.** The controller and the supervisor read time only through an injected
clock with three readings: wall, continuous with its boot session, and the clock their waits use.
Tests set each independently, and no test sleeps for the duration it tests. A wait for B on a
clock that stops during sleep fires late by the sleep's length, so the controller never relies on
one long timer: while running, it re-checks every bound at least once a second and starts an
expiry's removal **within 2 s of B**.

### 4. Expiry: the fence, then exact removal

**The fence.** The supervisor receives each grant's bound and idle limit with the grant's apply
(spec 001 §4.1), before activation. From the lease bound on, the host gate of every grant in the
attachment admits no new IO, on any reference, whether opened before the bound or after. The gate
reads the supervisor's clock at every admission, as spec 001 §4.2's graceful pause already does.
So the fence needs no timer and no controller. The same gate measures idle time, from the last IO
the attachment admitted or still had in flight, and fences when the idle limit is reached.

The fence is permanent: no restored graceful pause after `DrainTimedOut`, late drain reply,
renewal or later use reopens it. Host selection (`grant.select`) treats a fenced grant as closed,
so an equal peer from another attachment serves fresh lookups (spec 017). The fence withdraws
nothing, releases nothing and finishes no journal record, so it is not a removal path. The holding
stays standing, reported as fenced, until exact removal withdraws it.

**Exact removal.** The controller starts removal in three cases: B passes, the supervisor reports
an idle fence, or a scoping exec is reported `exited` or `signaled`. The removal is the
transaction `plasmid.remove` performs (spec 001 §3.11, spec 008):

1. A prepare gives the plugin a null replacement. It does the same for every requirer that
   section 2 takes with it.
2. Each recorded effect is withdrawn exactly, in LIFO order, with a graceful `DrainSpec` of D.
3. The finish is written only after fresh observation proves those effects absent.

The transaction enters the cell's one-at-a-time mutation order (spec 023 §6) as a mutation that
arrived at B. It withdraws only holdings this attachment owns. Spec 017 keeps every equal peer. Its
owner and address checks, and the drain behaviour PR #122 implements, apply unchanged. An exec scope
has no gate fence: the controller polls spec 024's `exec.status` at least once a second. So while
the controller runs, the grants stay reachable for at most 2 s after the exec ends. While it is
down, they stay reachable until the lease bound.

**Escalation.** Every expiry prepare carries standing authority to Force, which comes from the bound
itself and is recorded as an expiry authority, never an operator assertion (section 10). When a
graceful withdrawal returns `DrainTimedOut`, the controller forces exactly the holdings that timed
out, and nothing else. Before using Force it appends spec 008's audited session-log `force` event,
naming that authority. Outstanding external effects block a safe `plasmid.remove` but not expiry:
Force discharges them, and the expiry record names each one.

**What is recorded.** The journal records the transaction exactly as for `plasmid.remove`. The
session log gets one `expiry` event per attachment, naming the cell, plugin, generation and cause
(`lease`, `idle`, `exec` or `provider`); both readings of the bound; when the fence was first
observed and when removal started; each external assertion discharged; and each `GrantId`'s
outcome: `drained`, `escalated` with its deadline, or `incomplete` with its detail. Every forced
holding is named; a count is not enough.

### 5. Held references, and their bound

A revoke removes reachability, not objects a process already holds. Measured in a booted libkrun
guest, with references held: a lazy unmount succeeds, an open file descriptor stays readable, and
a mapping keeps executing. Revocation by name therefore cannot meet this spec.

What bounds a held reference is spec 017's mediation. Every managed file, mount, stream and flow
passes a host check on every request, and managed files cannot be mapped (spec 017, O-7). So after
the fence, IO on a reference opened before B is refused at the gate, with the error spec 024 §9
names. The contract term is **until the last reference goes**:

- IO admitted before B may finish within D. Escalation forces what remains by H.
- IO that the operating system cannot interrupt stays an `IncompleteEffect` until it ends. It is
  named with its owner `{cell, plugin}` and its `GrantId`.
- **Residue** is a reference with no owner, or a reference still effective after H.
- Bytes already delivered into the cell are not a capability; nothing claims them back (spec 011).

One part can be checked while the expiry runs, and it fails the expiry: a surviving reference with
no owner fails it, as it fails a detach (spec 011). The other part is a later obligation: a
reference still effective after H is reported against its owner after the expiry returned, and again
until it is gone. The journal's finish waits for the observed exact absence spec 008 requires; until
then the journal holds the pending removal, and the session log the expiry event and each later
report.

### 6. When removal fails or stalls

Status (section 9) shows an unfinished expiry in one of these states:

| State | Meaning |
| --- | --- |
| `fenced` | B has passed and removal has not started: the controller is down, or the cell is busy |
| `draining` | graceful withdrawal is in progress |
| `escalated` | the named holdings timed out and are being forced |
| `incomplete` | Force left an `IncompleteEffect`; the holdings and their owners are named |
| `failed` | the transaction was refused, or its audit failed (spec 008); the refusal is named |

None of these is success. The plasmid's state is `draining`, never `active`. A retry reuses the
prepared operation, never a new one with fresh IDs, like spec 008's resumed withdrawal. The fence
stays closed throughout. While any expiry is unfinished past its H, `plasmosome.status` reports
`ready: false`.

**A known gap, which expiry must not inherit.** Today a forced removal whose residue diff is empty
renders `Empty`, like a clean one: `ResidueReport::from_diff` takes only the diff and the external
assertions. `plasmid.remove` and `cell.kill` tell them apart only by `drained`, and a replay count
cannot say which holding was forced. Expiry records name each forced `GrantId` instead, so forced
and clean expiries differ wherever either is reported. Closing the gap for the existing verbs is
outside this spec; the PR lists it.

### 7. Restarts, and a missing supervisor

- **Controller down.** The fence still holds at B: spec 001 §4.2's gates and streams belong to the
  supervisor and outlive the controller. Removal waits. Once startup succeeds, the controller
  expires every attachment past its B before any other mutation of that cell, reading the bound from
  the journal, never from the operator file or counted from the restart. The expiry event records
  how late removal started.
- **Supervisor gone.** Spec 023 never relaunches a supervisor. The host side of every gate dies
  with it, so nothing is admitted, and the guest closes its own admissions when it sees the
  association is lost (spec 001 §4.2). The cell is unaccounted (spec 023 §8) and never mutated,
  so its expiries cannot run. `plasmosome.recovery` shows each holding with its recorded bound.
- **Host reboot.** The guest is gone; spec 023 retires the cell and reports its holdings as residue.

### 8. Renewal

`plasmid.renew` takes `kernel`, `cell`, `plasmid`, exactly one of `lease_ms` (measured from now,
in section 1's range) or `until_revoked: true`, and `operator` and `reason`, both required and
nonblank as for Force. The new bound may be earlier or later than the current one.

A renewal is a spec 008 transaction whose replacement keeps every effect and `GrantId` and changes
only the lifetime. Its prepare, and a session-log `renew` event naming the operator, reason, and
old and new bounds, become durable first. It must then take effect in the journal, as the
committed lifetime, and at the gates, where the supervisor raises every grant's bound in one step,
or none if any is already fenced. The bound in force is the earlier of the two, so a failure in
either place leaves the earlier bound in force, the reply says so, and no failure extends a grant.
A renewal not in force at the gates before B is refused with 105 `{from: "expired", to:
"renewed"}`, and the fence stays. A renewal renews no provider; one that would pass a provider's
bound is refused with 105 naming it. Any idle limit or exec scope is kept.

Renewal is never self-service. No method on the guest's 4090 or 4091 lanes changes a bound, and
spec 023 §9 keeps the workload away from the control socket (gated on O-2). An agent wanting more
time has no verb; it asks through whatever it already uses to reach its operator, and that request
carries no authority. A channel for such requests is RESERVED. The control socket proves a UID,
not a person (spec 001), so the recorded operator is an assertion, as for Force.

### 9. What status shows

Each per-plasmid object in `cell.status` (spec 001 §3.6) and `plasmid.list` (§3.9) gains
`lifetime`, and it is always present. Inside it, an empty field is left out (spec 001 §1). The
shape below is provisional. The change that serves it corrects the example rather than being
bound by it.

```json
{"plasmid": "github-pr", "state": "active",
 "lifetime": {"kind": "lease", "lease_ms": 900000, "idle_ms": 120000,
              "not_after": "2026-10-08T14:15:00Z", "remaining_ms": 412000,
              "source": "operator", "renewals": 1}}
{"plasmid": "model-provider", "state": "draining",
 "lifetime": {"kind": "lease", "lease_ms": 900000, "remaining_ms": 0, "source": "default",
              "expiry": {"state": "escalated", "cause": "lease", "escalated": ["<GrantId>"]}}}
```

An `until_revoked` plasmid shows `"kind": "until_revoked"`, so forever never looks like a missing
field. `remaining_ms` is computed at reply time from the reading with less time left, never below
0. `source` is where the lease in force came from: `default`, `operator`, `declaration`, `request`
or `renewal`. An exec-scoped attachment shows its `exec_id`. After the finish the plasmid is gone
from the list and the session log keeps its `expiry` event. `plasmosome.status` labels are
unchanged, and spec 025's own rule adds a `plasmid renew` row once the verb is served.

### 10. Changes proposed to other specs

These changes are proposed, not made here. The PR that accepts this spec carries their exact text;
without it, this spec stays draft.

- **Spec 022 (draft):** §4 gains the optional `lifetime` key in version 1, before that version
  ships; §2 resolves the lifetime in its single read; §8 adds that a reload never moves a bound
  later; §9 gains the lifetime refusals.
- **Spec 011:** the sixth kind of declaration becomes "how long the plasmid may hold its grants,
  and how long it may take to stop". `[lifecycle]` gains `lease_ms` and `idle_ms`, which can only
  shorten a lifetime. No seventh kind is added.
- **Spec 001:** §3.6 and §3.9 gain `lifetime`; §3.5 and §3.10 gain the request's `lifetime`; §3
  gains `plasmid.renew`, with no new error code. In §4.1 an effect apply carries the bound and
  idle limit, a renewal raises them, and observation reports fenced grants. In §4.2 admission
  refuses at and after the bound, independently of the graceful pause. In §3.3 readiness is false
  while an expiry is unfinished past H.
- **Spec 008 (format 1, not yet written):** each `Replacement` gains a required `lifetime` record
  (the bound's two readings, the limits, where each came from, the renewal count); `force` admits
  a third form, `{"expiry": {plugin, cause, bound}}`, which is not an operator assertion; the
  session log gains `expiry` and `renew` events; after startup, overdue expiries come first in
  each cell's mutation order.
- **Spec 017:** one sentence where a graceful timeout restores admission: restoring it never lifts
  a lifetime fence. Exact removal is otherwise used as it stands.
- **Specs 018, 023, 024 and 025:** no change. Spec 023 §6's arrival order is kept, with an expiry
  arriving at its B.

## Open questions

These need the owner. Where this spec cannot work without an answer, it uses the default named.

1. **The default.** Is it right to use a 15-minute lease, no idle limit and a 5 s drain? A short
   default means more renewals, and renewal fatigue pushes operators toward `until_revoked`.
2. **Force at expiry.** Is a bound standing authority to Force at H (the default), or should an
   expired attachment stay fenced until an operator forces it? The fence holds either way; this
   decides only whether work in flight is cut off and cleanup finishes unaided.
3. **Who renews.** Is any control-socket caller with a recorded operator and reason enough (the
   default)?
4. **Asking for more time.** Should a cell get a channel to request a renewal, carrying a request
   but never a grant?
5. **Task scope.** The product has no task; spec 021's delegated operation may become one. Task
   scope is RESERVED until something needs it.
6. **Exec binding.** `plasmid.add` names an already running exec (the default), so the capability
   arrives after the process starts. Should `cell.exec` attach plasmids instead?
7. **The caps.** Are 30 days for a lease and 30 s for a drain budget right?

## Acceptance

Each item names the broken implementation it catches. Every lease, idle and exec item runs on the
injected clock, and none sleeps for the duration it tests.

1. **The default, at the boundary (model).** No lifetime is declared anywhere, and the fake clock
   advances 5 s between prepare and commit. The bound is prepare + 900,000 ms on both readings; at
   B − 1 ms no removal has started, and by B + 2 s one has. Catches: no default, an
   `until_revoked` default, and a bound measured from commit.
2. **The narrowest wins (model).** Operator `until_revoked` with author 600 s gives 600 s, source
   `declaration`. Under an operator 300 s, author 1,200 s gives 300 s, a request for 120 s gives 120
   s (source `request`), and one for 3,600 s gives 300 s. With no operator lifetime, author 3,600 s
   gives 900 s, source `default`. Catches: last writer wins, and an author or request lengthening a
   lifetime or the default.
3. **Only the operator says `until_revoked` (model).** A declaration writing it is refused as a
   named unknown field; a request sending it gets `-32602`; a lease under 1,000 ms or over 30 days
   is refused from every source; none of these prepares anything. From the operator file it shows
   in status. Catches: a missing lifetime read as forever, and an author or caller granting itself
   forever.
4. **Expiry removes exactly the plasmid (model).** X (60 s) and Y (600 s) hold equal capabilities
   in one cell. At 62 s the journal holds the prepare `plasmid.remove X` would write, X's holdings
   are gone, and Y's stand with their original IDs; at 59 s both stand. Catches: removal by
   resource description, a path that skips the journal, and expiring the whole cell.
5. **Closures (model).** R (600 s) requires P (300 s); R's recorded bound equals P's, and at it
   one transaction removes R then P, with no strand refusal. With P `until_revoked`, R keeps
   600 s. An idle fence on P removes R too, cause `provider`. Renewing R past P's bound is 105
   naming P. Catches: a stranded requirer, and an expiry that refuses.
6. **Clean and forced expiries differ (model).** Two expiries are staged identically, except that
   one holding stalls its drain (`stall_graceful_drains_for_owner`); no `drain_ms` is declared.
   The clean record names every `GrantId` `drained`. The stalled one names exactly the stalled
   `GrantId` `escalated` at 5,000 ms and not its drained peer, and the records compare unequal.
   Catches: forced and clean rendering alike, an unnamed forced holding, and attribution by count.
7. **Force is audited first (model).** Kill the controller between the expiry prepare and the
   `force` event, then restart: the event is appended before any forced withdrawal. With the
   audit write made to fail, no forced withdrawal or finish happens, and status shows `failed`.
   Catches: Force used before its audit.
8. **Failure is not success (model).** Force leaves an `IncompleteEffect`: status shows
   `draining` and `incomplete` with the `GrantId` and owner, no finish is written, readiness is
   false after H, and retries reuse the prepared operation. The same staging with Force succeeding
   writes the finish and restores readiness. Catches: success reported over an incomplete effect,
   and a retry under fresh IDs.
9. **The fence (gate model).** With bound B, an admission at B − 1 ms passes and one at B is
   refused. A graceful pause timed out and restored after B admits nothing; the same restore
   before B readmits. IO on a handle opened before B is refused after B. An equal peer with a later
   bound serves a fresh lookup after B. Catches: a fence built as the reversible pause, a check
   only at open, and an expiry that takes the peer.
10. **Idle (gate model).** Idle 60 s, lease 600 s. Use at 0, 50 and 100 s, then none: fenced at
    160 s, and use at 161 s is refused and revives nothing. One more use at 155 s keeps it open at
    160 s. Use every 10 s still ends it at 600 s. Catches: use extending the lease, revival after
    the fence, and idle measured from attach.
11. **Exec scope (model with a 4090 test double; gate model).** A 600 s lease scoped to `e-1`:
    when the double reports `e-1` exited at 30 s, removal has started by 32 s; with `e-1` still
    running, nothing starts. Scoping an unknown or exited exec refuses and prepares nothing. With
    the controller stopped, the gate still fences at 600 s. Catches: a scope that never fires, and
    a scope with no ceiling.
12. **Controller restart (model; gate model).** Stop the controller before B, advance past B, and
    restart. The gate refused from B while it was down. After startup succeeds, the cell's first
    new journal record is the expiry prepare, with the original bound and the delay named. Editing
    the operator file meanwhile changes nothing. Catches: missed expiries never firing, a bound
    reset at restart, and reading the operator file at recovery.
13. **Clocks (model, then pinned runtime).** Advance wall and continuous by 11 minutes with the
    wait clock still: a 10-minute lease's removal starts within one 1 s check, and without the
    advance nothing starts. Wall stepped back an hour: expiry still fires at the continuous
    deadline. Wall stepped forward past B: it fires early. A foreign boot session's reading is
    ignored. Later obligation: on the pinned Mac, sleep the host past a 2-minute lease and wake it;
    the first admission is refused and removal starts within 2 s. Catches: bounds and timers built
    on `Instant`, and a check of only one clock.
14. **Renewal (model; gate model).** A renewal before B with operator and reason moves the bound,
    appends `renew` with both bounds, and increments `renewals`; without either it is `-32602`;
    after the fence it is 105 `expired` and admission stays refused. When the supervisor's raise
    fails, the reply is an error and status and gate keep the earlier bound. A reload at 500 s of a
    600 s lease keeps the original B. Catches: a renewal reopening a fence, an unrecorded renewal,
    and a reload used as a renewal.
15. **No renewal from inside (model).** No 4090 or 4091 method changes a bound; a guest request
    naming one gets `-32601` and the bound is unchanged. Catches: a convenience extension verb.
16. **Status (model).** A leased and an `until_revoked` plasmid side by side both show
    `lifetime`, and `remaining_ms` falls with the fake clock. During expiry the plasmid is
    `draining` with its expiry state; after the finish it is gone and the session log keeps the
    event. Catches: omitting `until_revoked`, so forever reads as missing, and a stale remaining
    time.
17. **Held references, in-band and later (model, then pinned runtime).** In the model, an expiry
    whose surviving reference has no owner fails naming it, while the same staging with the owner
    recorded succeeds; a reference still effective after H is reported against its owner after the
    expiry returned. On the pinned runtime, with a Mount and a ProxyMap, a descriptor and a working
    directory opened before B and an established TCP flow are each refused or carry no byte after
    B, while the same uses succeed at B − 1 s. A mutant expiring by lazy detach alone fails, and
    killing `membraned` leaves no grant reachable. Catches: revocation by name only, which leaves a
    descriptor and a mapping working as measured, and a check that runs only after the expiry.

## Out of scope

- How long a cell lives (spec 023), and revoking bytes or secrets already delivered into a cell.
- A credential's own `ttl`. Custody is reserved (spec 022); when it lands, a minted credential must
  not outlive the attachment carrying it.
- Delegation edges (draft intent 016). An edge is not a spec 017 class; if it becomes a grant, it
  may use these lifetimes under its own removal rules, which let admitted work finish.
- Per-grant lifetimes and task scope, until something needs them.
