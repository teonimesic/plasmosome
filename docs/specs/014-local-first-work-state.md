---
id: 014
title: Local-first work state with one authoritative writer
status: accepted
intents: [015]
---

## Behavior

Plasmosome keeps the durable meaning of its work in Markdown and Git, and keeps changing
coordination state in one local-first ledger. Intents, specs and tasks remain readable documents;
their status, ownership, readiness and forge activity stop causing edits to those documents. After
cutover there is one writable authority for each fact, never a Markdown status beside a ledger
status.

Agents read the queue from a clone-local store without using the network. Every result says when
that store last synchronized, which remote generation it last observed, and whether local changes
remain unpublished. Any operation that changes authoritative state synchronizes and publishes
online before it reports success, so an offline or losing session cannot claim work or cause an
external action.

The implementation uses Beads 1.1.2 with its embedded Dolt store and the existing GitHub
repository as the remote. A Plasmosome wrapper owns writes and uses a short project writer lease
plus compare-and-set publication to `refs/dolt/data`; raw concurrent Beads pushes are not part of
the workflow. This is a small coordination ledger for public open-source work, not a project
management suite or a prose editor.

## Contract

### One authority for each fact

Markdown and Git are authoritative for durable content:

- the three-digit document id, title and canonical repository path;
- intent, spec and task prose, including rationale and references;
- the upward `intents:` and `specs:` links;
- a task's plan and `done_when` condition;
- notes, outcomes and evidence worth retaining after the operational event is old; and
- Git history for every content revision.

Beads is authoritative after cutover for volatile operational state:

- intent, spec and task lifecycle status and transition history;
- task priority, owner, agent session, claim and lease;
- dependency, ready and blocked projections;
- pull request, check, review and merge observations;
- operational evidence, idempotency receipts and reconciliation cursors; and
- synchronization, lease and transition timestamps and audit history.

An imported title is a read-only projection of Markdown. Operational reconciliation may not edit
document prose, links, plans, `done_when`, notes or durable outcomes. Durable evidence may cite a
pull request or merge, but the wrapper never reads that prose as current PR, CI, merge or lifecycle
state.

The authority mode is itself an authoritative ledger value. `markdown-shadow` permits the existing
volatile frontmatter and permits only a one-way import from Markdown. `ledger` rejects Markdown
records containing writable volatile fields and rejects any importer that tries to overwrite
ledger state from them. A generated snapshot is allowed only when it is marked as a projection,
is never accepted as mutation input, and cannot be confused with a source document.

### Stable document mapping

Every intent, spec and task has this logical record at the wrapper boundary. The adapter may map
it to Beads fields, labels or relations differently without changing the contract.

```text
document_key       = "intent:015" | "spec:014" | "task:001"
kind               = intent | spec | task
document_id        = exactly three decimal digits
document_path      = canonical Git-relative Markdown path
title              = current Markdown title, read-only in Beads
content_commit_sha = 40-hex Git commit that established the imported content
state_version      = monotonically increasing unsigned integer
intent_ids         = ordered list of three-digit intent ids
spec_ids           = ordered list of three-digit spec ids
```

`document_key` is immutable and unique across the project. The three id namespaces stay separate,
so `intent:014`, `spec:014` and `task:014` are different keys. A rename updates
`document_path` through an audited content import but does not mint a new key. A deleted key is
retired and its three-digit id is never reused.

`content_commit_sha` is the newest commit on the imported authoritative branch that established
the current path and contents. The importer verifies that the file read from that commit has the
same contents as the imported file. An unrelated later commit does not change this value. Every
approval, acceptance and content import records this SHA; a gate refuses a governance transition
whose receipt names a different content revision.

`state_version` starts at 1 on first import and increments on every authoritative
mutation to that record, including content import, lifecycle, priority, ownership and reconciled
forge state. A mutation's `expected_document_version` is this exact public value, not the content
commit. Its receipt records expected and resulting versions. The project remote generation fences
publication, while the per-record version refuses a lifecycle mutation based on stale state.

The link lists are imported from Markdown without renumbering, reordering or replacing them with
Beads-native ids. Each id must resolve to exactly one document of the required kind. The ledger may
also materialize relations for queries, but import and export are compared through the logical
record above. Beads-native row ids and its internal table layout are not caller-visible contracts.

For a task naming several specs, its intent closure is an ordered union. Traverse `spec_ids` in the
task's order and each spec's `intent_ids` in that spec's order; append an intent the first time it
appears and omit later duplicates. The task's copied `intent_ids` must equal that complete ordered
union. Ready, planning and start checks evaluate every spec and every intent in the union, never
only the first named spec.

### Pinned local store and remote

The only supported tracker is Beads `1.1.2`, pinned by version and verified release checksum. A
repository pin manifest records each supported artifact filename and SHA-256 copied from the
`checksums.txt` asset on the upstream Beads v1.1.2 GitHub release. The wrapper checks both that
manifest and `bd --version` before opening or changing the store and returns
`unsupported_beads_version` or `beads_checksum_mismatch` on failure; it never performs an automatic
schema upgrade. The pinned release is MIT licensed.

Each clone has one embedded Dolt store shared by its worktrees. Initialization uses
`bd init --stealth` or a verified equivalent that installs no hooks, edits no `AGENTS.md`,
`CLAUDE.md` or skill, and stages no repository file. The wrapper owns the store location and
configures the existing GitHub `origin` as the Dolt remote whose authoritative generation is
`refs/dolt/data`.

Embedded cleanup drops its handles and removes its temporary root; it does not invoke `bd dolt
stop`, because embedded mode starts no Dolt server. The harness reaps only child processes it
actually started.

`dolt.auto-push` is false and no daemon, hook or background job performs a Git-protocol push.
Beads 1.1.2 warns that concurrent automatic pushes can corrupt or strand remote history; all pulls,
commits and pushes used for authoritative mutations occur inside the guarded wrapper protocol.
Ordinary queries never invoke Beads behavior that synchronizes implicitly.

Transport contract tests use the same injected command boundary as the production adapter. They
script the exact `git` and Beads observations for two independent clone-local stores: expected-base
read, winning non-forcing push, stale non-fast-forward rejection, retry before publication and
re-observation after a lost response. They assert the constructed publication command is
non-forcing and that any exceptional compare-and-set ref update uses an explicit
`--force-with-lease` expected SHA; a bare force is always unsafe. This tests Plasmosome's command
construction, classification and idempotency logic without a Git or GitHub emulator.

GitHub's documented rejection of non-fast-forward pushes is the stable platform contract for the
ref update, and Git receive-pack defines the underlying ref-update behavior. No GitHub REST mock,
hosted fixture or credential is involved because Beads synchronization uses Git transport. Not
running a live GitHub proof is not a cutover blocker; an unsafe configured command, a missing
expected base where a lease is required, or an observed result that contradicts the documented
contract is.

### Local reads and honest freshness

`list`, `show`, `ready`, `blocked` and `heartbeat observe` query only the embedded local store.
They work with DNS and all network routes disabled. Their structured and human-readable forms
always include:

```text
last_successful_sync_at = UTC timestamp or unknown
local_generation        = local Dolt commit SHA
remote_generation       = last observed refs/dolt/data SHA or unknown
remote_observed_at      = UTC timestamp or unknown
pending_mutations       = count plus operation ids
freshness               = synchronized_as_of | stale | unknown | unpublished |
                          stale_with_unpublished | unknown_with_unpublished
```

`synchronized_as_of` means only that local and remote generations were equal at
`remote_observed_at`; an offline read may retain that label and timestamp, but never presents it as
current. `stale` means a newer remote generation has been observed but not applied. `unpublished`
means the local store contains a mutation not confirmed on GitHub. A clone with no successful
remote observation, or an explicit synchronization attempt whose failure leaves equality unknown,
is `unknown`. Only an online command that re-reads the remote in the same operation may say it
observed the current remote generation.

Pending publication and remote freshness are independent. When pending mutations coexist with an
observed newer remote, the value is `stale_with_unpublished`; when they coexist with unknown remote
equality, it is `unknown_with_unpublished`; otherwise pending mutations produce `unpublished`.
`pending_mutations` remains populated in all three cases, so recovery never hides either condition.

`ready` and `blocked` are local projections and carry the same freshness envelope. A ready task is
`planned`, has no live task owner or dependency blocker, names accepted specs, and reaches approved
intents. A stale projection may guide reading but never authorizes starting or dispatching work;
the mutation path repeats every check against freshly synchronized state.

### Online, idempotent mutations

The supported writer is the small Plasmosome work-state wrapper. Agents may use raw `bd` commands
for diagnostics against a disposable copy, but direct `bd` writes or pushes to the project store
are unsupported and are rejected as missing a wrapper operation receipt.

Every mutation supplies a project id, actor, session id, semantic operation id and
`expected_document_version`. The operation id is stable across retries. A completed result or a
terminal refusal validated against fresh authoritative state is published as an operation receipt;
repeating it returns that result. A transport or process failure before any authoritative receipt
exists does not consume the operation id, so the same operation resumes after recovery. A published
version conflict is terminal for that operation; the caller refreshes and uses a new operation id
and expected version for a different request.

An authoritative mutation requires a working GitHub connection. The wrapper synchronizes,
acquires admission, validates document versions and lifecycle gates, mutates locally, and publishes.
It reports success only after GitHub accepts a descendant of the expected remote generation and a
fresh read of `refs/dolt/data` contains the operation receipt. A local commit, a Beads success
message or acquisition of the writer lease is not success.

If the process stops after a local mutation, reads expose it as unpublished. Recovery fetches the
remote generation and uses the operation receipt to distinguish "published but response lost"
from "never published". It either returns the already-published result or discards and replays the
mutation after revalidation; it never merges an unvalidated local outcome into authority.

### Writer admission and publication fencing

The project writer lease is a short-lived, renewable ledger record containing a random fencing
token, actor, session, semantic operation id, acquisition generation, acquisition time and expiry.
Expiry is evaluated against GitHub-observed time, not only a workstation clock. Expiry makes a
takeover eligible; publication order on GitHub decides which contender owns the next generation.

Lease acquisition is a special wrapper mutation. A contender pulls an observed
`refs/dolt/data` generation, appends an acquisition or expired-takeover record, and publishes a
candidate whose parent is exactly that observed generation. The remote update is non-forcing and
compare-and-set: one fast-forward update wins. A rejected contender refreshes, removes its local
candidate, reports `writer_conflict`, and performs no lifecycle change or external side effect.

The lease is admission serialization; the authoritative fence is successful compare-and-set
publication from the expected `refs/dolt/data` base. Before publishing an operation, a holder
renews or revalidates its token and base. A takeover advances the same remote history, so a paused
former holder's candidate descends from an old base and GitHub rejects it as non-fast-forward.
Release is recorded in the operation commit or a later idempotent release commit. A lost release
response is recovered from history or by expiry.

The cutover test must establish that the wrapper constructs and classifies this non-forcing
expected-base behavior for `refs/dolt/data`, including a stale holder after takeover. A publication
plan containing an unleased force, an exceptional leased update without the exact observed base,
or a recorded accepted stale result is `cutover_blocked`. The absence of a live hosted test is not.
Retrying a stale `bd dolt push` or weakening exactly-one publication is not an alternative.

### Ownership before effects

A task claim is separate from the short project writer lease. It records the task, owner actor,
session, renewable ownership token, lease times and claim operation id in Beads. The claim must be
published and confirmed remotely before that session creates a branch, changes task status,
dispatches an executor or causes any other external effect for the task.

Every external coordination action has a deterministic action id and is authorized in the ledger
before invocation. Branch creation uses create-if-absent against its expected base. A dispatcher or
other effect adapter must deduplicate that action id or refuse a stale ownership token. Recovery
completes the same authorized action; it does not authorize a second one. An effect that offers
neither compare-and-set nor idempotency is a cutover blocker for that action.

An ownership lease may be renewed only by its current token. After expiry, a new owner takes over
through the guarded mutation protocol and the audit records the former token, new token, actor,
time and reason. Once takeover publishes, every later mutation or effect from the former token is
refused. A losing claim racer refreshes and performs no branch creation, dispatch, status change or
other effect.

### Lifecycle gates and audit

The ledger preserves the current lifecycle vocabularies: intents are `draft` or `approved`; specs
are `draft`, `accepted` or `superseded`; tasks are `todo`, `planned`, `in_progress`, `in_review` or
`done`; task priority is 1, 2 or 3. The wrapper, not Beads' built-in ready or claim behavior,
defines these meanings and rejects every transition not listed here:

- intent: `draft -> approved`; `approved -> approved` only to bind an explicit new owner approval
  to a changed content commit;
- spec: `draft -> accepted`; `accepted -> accepted` only to bind reviewed changed content;
  `accepted -> superseded` with a reason; and
- task: `todo -> planned`, `planned -> in_progress`, `in_progress -> in_review`,
  `in_review -> done`, plus `in_progress -> planned` or `in_review -> planned` only through the
  recovery rules below.

The wrapper validates the edge, expected state version, gates and ownership before any mutation or
external effect. In particular, `todo -> done`, `planned -> done`, skipping `in_review`, and every
transition out of `done` are refusals.

A task may move from `todo` to `planned` only when its Markdown revision has a non-empty plan and
`done_when`, its `spec_ids` name accepted specs, and its copied `intent_ids` match the intents those
specs name. A task may be claimed or start only from `planned`, and start rechecks that every named
spec is accepted and every reached intent is approved. A task without those links is blocked, not
ready.

A spec may move from `draft` to `accepted` only when every intent it names is `approved` for the
current content commit. Acceptance records the spec content commit. An accepted spec may move to
`superseded` with a reason. Task recovery from `in_progress` or `in_review` to `planned`, and every
other non-forward transition, requires explicit operational evidence and an audit reason rather
than being inferred from a missing branch.

Intent approval remains an owner decision. The wrapper never infers it from a merged PR, GitHub
actor, review, comment, elapsed time or agent request. An explicit owner approval records actor,
`draft -> approved`, the exact intent content commit, UTC timestamp, reason or relayed instruction,
operation id and resulting remote generation. The wrapper can enforce every machine-readable gate
that consumes this receipt; it does not claim it can decide that the owner approved.

The repository currently gives agents the owner's GitHub identity, so the actor field is audit and
not authentication. The wrapper requires an explicit owner directive rather than an actor string,
but instruction remains the control on who may supply it, as recorded in
`docs/decisions/008-approving-an-intent-is-an-instruction.md`. Cryptographic distinction between the
owner and an agent requires separate identities and is not invented by this work-state migration.

Every lifecycle transition records previous state, new state, actor, session, document key,
content commit, operation id, timestamp, reason and remote generation. Histories are append-only;
conflict recovery may add a compensating event but may not erase a losing or expired claim.

### Forge reconciliation and heartbeat

GitHub PR, check, review and merge observations are facts keyed by their stable GitHub ids and
updated timestamps. Replaying an observation is an upsert with no second transition. Older or
partial observations cannot move a record backward. A merge is reconciled to `done` only after the
configured merge conditions and evidence are present.

Webhook-like delivery is an optional latency improvement, not a correctness dependency. A bounded
poller enumerates changed and still-open PRs, checks, reviews and merges from the last cursor, and
periodically performs a full repair scan. Its cursor advances in the same authoritative mutation
as the facts it covers, so interruption repeats work safely.

Heartbeat has two commands. `heartbeat observe` is a concurrent local-only read that reports
freshness and proposed actions. `heartbeat apply` sends each proposed mutation through the writer
lease, ownership and publication fence. Observation never creates a branch or dispatches; two
sessions proposing the same reconciliation or dispatch share one deterministic operation/action
id, so only the published winner may perform it.

### Migration, backup and rollback

Migration starts in `markdown-shadow`. A one-way importer reads every current numeric intent, spec
and task from one Git commit, including all 39 task records present when this spec was written.
Later runs discover all matching records dynamically rather than keeping 39 as a configured limit.
Markdown remains authoritative, and agents do not write both representations.

Shadow mode compares the logical document mapping, lifecycle state, priority, links, ready/blocked
projection and PR/merge evidence against the existing Markdown and GitHub reconstruction. Cutover
requires parity plus the race, interruption, stale-holder, expiry, conflict, backup and restore
tests below on two processes and two independent clones.

Before cutover, the migration makes a restorable Dolt backup, a logical export and a Git tag or
commit identifying the Markdown snapshot. Cutover then blocks work-state mutations, performs the
final import and reconciliation, and merges one Git change that removes writable volatile fields
and updates every agent-facing lifecycle rule, template, selector and status-flip instruction to
use the wrapper. Only after that Git commit is observed does the guarded mutation write and publish
the `ledger` authority epoch. CI and the wrapper reject dual authority from that epoch onward. The
maintenance interval has no writable operational authority; it never enables both.

Rollback is an explicit authority transition, not a concurrent fallback. A published write freeze
leaves `ledger` as the sole authority while refusing every ordinary mutation; only the idempotent
administrative rollback operation is admitted through the writer lease and expected-base fence. It
backs up the last remote generation and audit, restores a tested snapshot, and stages regenerated
Markdown operational fields from that one generation without accepting writes from them.

The administrative operation publishes `ledger -> markdown-shadow`, the exact target Git
revision, reason and invalidation of older ledger tokens before Markdown writes are enabled. A
failure before that publication leaves the published mode `ledger`; a failure after it leaves
`markdown-shadow`. There may be a no-writer interval while the target Git revision is activated,
but never zero or two published authorities, and a stale ledger writer cannot publish after the
mode transition.

Beads, embedded Dolt and the existing GitHub repository are the complete required infrastructure.
Public open-source use requires no paid runtime or synchronization service. Community web or TUI
clients may read the logical export as optional projections; their schemas and availability are
not part of this contract.

## Acceptance

The implementation supplies a hermetic runner, `./tools/work-state contract-test <case> --archive
PATH --bd PATH`, and CI runs `./tools/work-state contract-test all --archive PATH --bd PATH`. It
initializes two independent temporary clone-local
stores with the pinned Beads binary, then scripts the exact external Git/Beads observations at the
existing command seam. It uses no server, hosted repository, credential, GitHub API mock, fake
forge or shared store in place of the two clients.

- `shadow-parity --source-ref origin/main` dynamically imports every numeric intent, spec and task
  present when it runs and reports no missing, extra or different lifecycle, priority, link, PR or
  evidence value while Markdown is authoritative. The fixed historical fixture
  `13c0f68c13743f4db2fb123fef560f3fa12734d1` separately asserts 39 task records.
- `document-mapping --source-ref origin/main` exports and reimports every logical record and
  compares the exact three-digit id, kind, key, path, title, content commit and ordered upward
  links. Duplicate ids, missing targets, a changed order and a content/SHA mismatch each make the
  command exit non-zero with the offending key.
- `offline-reads` disables DNS and network routes, then runs `list`, `show`, `ready`, `blocked` and
  `heartbeat observe` in both structured and human-readable forms. Every result contains
  `last_successful_sync_at`, `local_generation`, `remote_generation`, `remote_observed_at`,
  `pending_mutations` and `freshness`, and the harness verifies that no socket was opened.
- `freshness` starts with an unsynchronized clone, an offline clone synchronized at a known old
  generation, a clone that has observed a newer remote generation, and a clone with an unpublished
  local mutation. They report `unknown`, `synchronized_as_of`, `stale` and `unpublished`
  respectively; the human form says "synchronized as of" with its timestamp, and neither output
  mode presents `synchronized_as_of`, stale or unknown data as current.
- `combined-freshness` leaves a mutation unpublished, then observes a newer remote generation and
  separately loses remote equality knowledge. The reads report `stale_with_unpublished` and
  `unknown_with_unpublished`, preserve the remote metadata and list the pending operation in both
  structured and human-readable output.
- `claim-race --processes 2 --clones 2` releases two contenders from a barrier against one planned
  task and asserts one published ownership token, one `writer_conflict` or `claim_conflict`, and one
  append-only winning transition on `refs/dolt/data`.
- `loser-has-no-effects` instruments branch creation, dispatcher calls and status writes during the
  same race and asserts exactly one branch/action/status effect, all carrying the winner's token,
  and zero calls from the losing process.
- `interrupted-mutation` kills the writer after lease publication, after local mutation, and after
  remote publication before the response. Each restart exposes pending state correctly, returns or
  completes one idempotent result, and leaves one transition rather than losing or duplicating it.
- `mutation-retries` gives two sessions the same expected state version and proves the first
  publication increments it while the second receives a terminal version conflict without a state
  change. It also interrupts transport before any receipt, then retries the same operation id after
  recovery and publishes exactly one result.
- `expired-lease-recovery` stops a holder, advances trusted time past expiry, publishes takeover,
  and proves the new holder can finish. The former holder's later publication and external effect
  are refused, and both expiry and takeover remain in audit history.
- `stale-base-fence` scripts a renewed holder and an expired former holder against one recorded
  `refs/dolt/data` history. The production command is non-forcing, the stale non-fast-forward result
  is terminal, and a later observation still names the winner. An unleased force, missing expected
  base or accepted stale result is a mandatory cutover blocker.
- `push-conflict-recovery` creates divergent local Dolt commits in two clones, resolves by guarded
  pull, replay and push, and compares every operation id and transition before and after. No history
  entry is lost, overwritten or silently force-pushed.
- `approval-audit` supplies an explicit owner directive and asserts actor, content commit,
  `draft -> approved`, UTC timestamp, reason, operation id and remote generation. A missing
  directive, inferred GitHub event and mismatched content SHA are each refused; the test does not
  claim authentication the repository's shared GitHub identity cannot provide.
- `gate-refusals` tries to plan a task without plan/`done_when`, claim or start an unplanned task,
  start through a draft or missing spec, accept a spec under a draft or missing intent, and reuse an
  approval for changed intent content. Multi-spec fixtures cover two accepted specs with distinct
  intents and two with an overlapping intent: the exact first-seen ordered union passes when every
  intent is approved, while a missing second-spec intent, a duplicate, wrong order and an
  unapproved intent each refuse. Every refusal exits non-zero before any remote state or external
  effect changes.
- `transition-graph` exercises every listed intent, spec and task edge, then tries every other pair
  of lifecycle values. It proves unlisted edges, including `todo -> done`, `planned -> done`, a
  skipped review and every transition out of `done`, fail before state or external effects change.
- `merge-reconciliation --replay 3` omits the initial merge observation, repairs it by polling, and
  replays the same PR, check, review and merge facts three times. It produces one final lifecycle
  transition and one set of facts with the merge commit preserved.
- `backup-restore` backs up a ledger with claims, expiries, approvals and merge facts, restores it
  into a fresh clone, and compares logical records, remote generation lineage and complete audit
  history before allowing writes.
- `dual-authority` proves shadow mode rejects ledger-originated operational writes, then switches a
  fixture to ledger mode and proves a hand-written Markdown `status:`, task `priority:` or `pr:` and
  a Markdown-to-ledger state overwrite each fail before publication. It also exercises the ordered
  rollback, injects failure before and after its fenced administrative transition, and finds
  exactly one published authority after each failure. Markdown writes start only after
  `markdown-shadow` publishes, and stale ledger tokens remain refused.
- `cutover-instructions` scans the ledger-mode fixture's skills, intent/spec/task READMEs, templates,
  hooks and heartbeat entry points. It fails if an agent is still told to write or reconstruct
  volatile Markdown fields, and proves those entry points use the wrapper and its lifecycle values
  after the authority epoch.
- `cutover-freeze` starts the ordered cutover, then attempts Markdown-originated and
  wrapper-originated operational mutations before the ledger epoch; both are refused. Publishing
  the epoch is also refused until the wrapper observes the exact Git commit that removed the
  volatile fields and updated the agent-facing instructions, after which ledger writes alone work.
- `version-pin` passes with a checksum-verified Beads 1.1.2 binary and refuses a lower, higher or
  unparsable `bd --version`, a missing platform entry in the repository pin manifest, and a binary
  reporting 1.1.2 whose SHA-256 differs from the pinned upstream release checksum. Every refusal
  occurs before store migration, sync or mutation.
- `stealth-init` initializes a clean fixture and proves agent instructions, hooks and tracked files
  are unchanged, embedded Dolt uses the configured `refs/dolt/data` remote, and automatic or
  concurrent Git-protocol pushes remain disabled.
- `network-required-mutations` disables the network and proves claim, transition, approval,
  acceptance, reconciliation and release commands fail without changing authoritative state or
  reporting success; local reads continue to work with `unknown` freshness.

The repository gate in the root `AGENTS.md` is green for the implementation, and the contract
runner records its Beads version, two clone labels, redacted production commands, scripted remote
observations and final operation ids so a reviewer can reproduce every decision without recording
secret or machine-specific paths.

## Out of scope

- General project-management features such as sprints, estimates, roadmaps, dashboards and custom
  workflows.
- Collaborative editing or synchronization of intent, spec or task prose.
- Agent messaging, inboxes or chat transport.
- A paid hosted database, paid synchronization service or required commercial UI.
- Treating a community UI as an authority or compatibility contract.
- Product capability or process-rule changes beyond the work-state wrapper, migration and the
  existing lifecycle gates described here.
