---
id: 046
title: Install and query the shared local Beads shadow
status: in_review
priority: 1
specs: [014]
intents: [015]
refs:
  [
    AGENTS.md,
    README.md,
    Cargo.toml,
    Cargo.lock,
    .agents/skills/planning-work/SKILL.md,
    .agents/skills/tasks/SKILL.md,
    .agents/skills/pr-review/SKILL.md,
    docs/intents/015-local-first-shared-work-state.md,
    docs/specs/014-local-first-work-state.md,
    tasks/042-beads-transport-foundation.md,
    tasks/045-beads-shadow-import.md,
    tools/work-state,
    tools/work-state-beads-1.1.2.toml,
    crates/plasmosome-work-state/Cargo.toml,
    crates/plasmosome-work-state/AGENTS.md,
    crates/plasmosome-work-state/README.md,
    crates/plasmosome-work-state/src/lib.rs,
    crates/plasmosome-work-state/src/command.rs,
    crates/plasmosome-work-state/src/contract.rs,
    crates/plasmosome-work-state/src/document.rs,
    crates/plasmosome-work-state/src/main.rs,
    crates/plasmosome-work-state/src/pin.rs,
    crates/plasmosome-work-state/src/shadow.rs,
    crates/plasmosome-work-state/tests/cli.rs,
    crates/plasmosome-work-state/tests/command.rs,
    crates/plasmosome-work-state/tests/contract.rs,
    crates/plasmosome-work-state/tests/document.rs,
    crates/plasmosome-work-state/tests/pin.rs,
    crates/plasmosome-work-state/tests/shadow.rs,
  ]
done_when:
  - `./tools/work-state bootstrap --source-ref REF --archive PATH --bd PATH` verifies the supplied pinned Beads 1.1.2 archive and binary, resolves REF without fetching, and atomically installs a complete `markdown-shadow` generation below the repository's absolute Git common directory so every linked worktree finds the same store.
  - Bootstrap installs the running wrapper and copies and re-verifies the pinned binary, commits and validates the imported Dolt state, is unchanged on an identical valid rerun, repairs only the same validated source generation, and leaves the previous current generation readable if preparation is interrupted.
  - `./tools/work-state list`, `show KIND:NNN`, `ready`, and `blocked`, with optional `--json`, need no Cargo process, artifact path or credential; the launcher selects the installed wrapper, which verifies itself and the installed Beads binary, reads a disposable copy of the shared local generation, and returns the exact Spec 014 freshness envelope.
  - The freshness model and real-store contract evidence cover `unknown`, `synchronized_as_of`, `stale`, `unpublished`, `stale_with_unpublished`, and `unknown_with_unpublished`, preserving observation metadata and pending operation ids in structured and human output.
  - `ready` and `blocked` use wrapper semantics rather than Beads' native open status, evaluate task lifecycle, live-owner and dependency blockers, every copied spec and intent link, accepted/approved lifecycle, and exact ordered intent closure, and always report that a local projection does not authorize starting work.
  - Missing or corrupt installed state, a changed snapshot during a read, invalid freshness metadata, unsafe paths, an unavailable local ref, and a source commit different from the installed generation are refused with stable codes without changing the active generation.
  - Ordinary reads construct only local `git rev-parse`, installed-wrapper and Beads verification, and pinned `bd --readonly --sandbox` status/export/KV commands under the isolated environment; they never run Cargo, resolve or fetch a source ref, configure or contact a remote, open a credential, sync implicitly, or modify the source checkout or shared generation.
  - Tests are written and observed failing before each implementation batch, real pinned Beads acceptance passes, coverage and uncovered branches are reviewed, the timed workspace suite and all five root gates exit 0, and the task makes no claim that Spec 014's complete `offline-reads` case passes because `heartbeat observe` and the OS-level no-socket harness remain separate work.
pr: https://github.com/teonimesic/plasmosome/pull/82
evidence:
---

## Why

The Markdown shadow can now reproduce the repository's work records in disposable Beads stores,
but an agent still cannot install that state once for a clone or query it from another worktree.
This task turns the verified shadow import into a durable, clone-local read model without changing
authority: Markdown at the selected Git commit remains the source of every imported field.

After one explicit bootstrap with already-present pinned artifacts, any worktree can run local
`list`, `show`, `ready` and `blocked` commands with no artifact-path knowledge and no network work.
Every answer says exactly what generation it read and what is or is not known about freshness.

## Plan

### Deliverable, in one sentence

Install one verified Beads 1.1.2 Markdown-shadow generation under Git's common directory, then
provide deterministic, read-only `list`, `show`, `ready` and `blocked` commands that all worktrees
share and that always return the complete Spec 014 freshness envelope.

### Acceptance delivered and deferred

This task completes the `freshness` and `combined-freshness` acceptance cases for the six-state
classifier, using the production read path and a real pinned embedded Beads store. It also adds a
`local-reads` contract case for bootstrap, common-directory discovery, `list`, `show`, `ready` and
`blocked` in both output forms. Name it `local-reads`, not `offline-reads`.

Spec 014's named `offline-reads` case is not complete here. That case also requires
`heartbeat observe` and an operating-system harness that disables DNS and network routes and
observes socket creation. Another session owns heartbeat work, and command recording cannot prove
what syscalls a real child made. This task must neither add nor run `heartbeat observe`, weaken
the no-socket requirement, or report the narrower case as the full acceptance case.

### Out of scope

- No `heartbeat observe` or `heartbeat apply`, and no edit to `.agents/skills/heartbeat`, any other
  agent-facing lifecycle instruction, root instruction, task template or selector. Do not run a
  heartbeat while executing this task.
- No OS network-isolation or socket-observation harness. The production read path is strictly
  local, but the complete `offline-reads` proof remains for the work that owns heartbeat and the
  operating-system network boundary.
- No online synchronization, ref observation, fetch, pull, push, `refs/dolt/data` publication,
  operation receipt, compare-and-set update, retry recovery or reconciliation.
- No writer lease, task ownership lease, claim, dispatch, branch creation, lifecycle write or
  external effect. The short process-scoped filesystem lock described below serializes only local
  installation; it is not an authority, ownership or work-admission record.
- No changed-source refresh or incremental merge into the active shadow. If the selected ref does
  not resolve to the installed source commit, return `source_commit_mismatch`. Preserving monotonic
  record versions, retired keys, id-reuse refusal and remote observations across a refresh is a
  separate capability; do not reset records to Task 045's initial `state_version = 1`.
- No GitHub polling, GitHub API or API mock, live or hosted fixture repository, local Git server,
  fake forge, broad mock framework, daemon, container or paid synchronization service.
- No backup, restore, rollback, garbage collection, authority freeze, authority epoch, dual-write
  period or `ledger` cutover. Old complete generations and abandoned staging directories are left
  untouched; deleting them safely is later maintenance work.
- No committed repository snapshot or copied Markdown corpus. Real acceptance derives documents
  from the requested ref in the real Git object database and creates any clone/worktree only under
  a temporary directory. Small typed state builders are allowed only for malformed metadata,
  readiness blockers and the six freshness states.
- No change to Markdown lifecycle, priority, PR, evidence, plan or links. Do not create a tracked
  `.beads`, `.plasmosome` or generated state file in any checkout.
- No use of Beads' native `list`, `show`, `ready` or `blocked` projection. All imported native rows
  are intentionally `open`; the Plasmosome wrapper owns document and readiness semantics.
- No raw `bd bootstrap`, SQL server, `bd sql`, Dolt remote or automatic publication. Pinned Beads
  1.1.2 does not support `bd sql` in embedded mode, and `bd vc status` is the supported local
  source of its commit identity.
- No next work-state capability. STOP after this plan's tests, documentation and gates pass.

### Files to read, and nothing else

The executor reads every `refs` entry before editing. The intended implementation changes are
limited to:

- `Cargo.lock` and `crates/plasmosome-work-state/Cargo.toml` for the
  already-workspace-pinned `libc` dependency used by the macOS/Linux nonblocking filesystem lock;
- `crates/plasmosome-work-state/src/pin.rs` for an installed-binary-only checksum and version
  verifier which is separate from install-time archive verification;
- new `crates/plasmosome-work-state/src/store.rs` for Git common-directory discovery, the on-disk
  pointer/manifest, installation lock, staged bootstrap, installed-wrapper selection, atomic
  activation, disposable store copies and fenced snapshot reads;
- new `crates/plasmosome-work-state/src/freshness.rs` for validated observation state, pending
  operation ids and the pure six-state classifier;
- new `crates/plasmosome-work-state/src/read.rs` for local projections and JSON/human rendering;
- `crates/plasmosome-work-state/src/shadow.rs` only to generalize the existing fresh-store adapter,
  add the strict Beads-backed operational sibling for task owners/dependencies, and expose the
  narrow validation/digest operations the durable store reuses;
- `crates/plasmosome-work-state/src/contract.rs` for temporary real-repository clone/worktree
  fixtures and the three new contract cases;
- `crates/plasmosome-work-state/src/main.rs` and `src/lib.rs` for the exact command surface and
  module exports;
- new `crates/plasmosome-work-state/tests/store.rs`, `tests/freshness.rs` and `tests/read.rs`, plus
  focused changes to `tests/pin.rs`, `tests/cli.rs`, `tests/shadow.rs` and `tests/contract.rs`; and
- `tools/work-state` so `bootstrap` uses release locked offline Cargo, `contract-test` retains
  debug locked offline Cargo, and ordinary reads select and execute the installed wrapper without
  starting Cargo; and
- `crates/plasmosome-work-state/AGENTS.md` and `README.md` to replace their disposable-only claim
  with the precise installed-shadow read boundary and its non-goals.

Do not edit any other file. If a required change falls outside this list, or the accepted spec
contradicts this plan, STOP and return to planning instead of expanding scope.

### Command surface and result contract

Preserve every Task 042/045 `contract-test` invocation and add exactly these user commands:

```text
./tools/work-state bootstrap --source-ref REF --archive PATH --bd PATH [--json]
./tools/work-state list [--json]
./tools/work-state show intent:NNN|spec:NNN|task:NNN [--json]
./tools/work-state ready [--json]
./tools/work-state blocked [--json]
```

`bootstrap` is the only public command that accepts artifact paths or a source ref. All flags are
single-use, values are nonblank, `--source-ref`, `--archive` and `--bd` are required, and unknown
or positional extras are `invalid_command`. `show` accepts one exact kind-qualified key; a bare id
is ambiguous and invalid. The read commands never accept a ref, remote, artifact, database path or
credential option.

For `bootstrap`, the tracked launcher runs exactly
`cargo run --release --locked --offline --quiet -p plasmosome-work-state -- ...`; for
`contract-test`, it retains `cargo run --locked --offline --quiet -p plasmosome-work-state -- ...`.
Both must fail locally rather than consult a registry. For `list`, `show`, `ready` and `blocked`, the
launcher runs only local `git rev-parse` with prompts, lazy fetch and optional locks disabled, reads
the single safe `current` basename, and `exec`s the wrapper inside that generation. It must never
start Cargo, rustup or another build tool. Missing, malformed or unsafe pointer/runtime state fails
in the launcher with the same structured or human error contract. The installed wrapper
independently revalidates its common-directory placement, manifest binding and checksum before
serving the request.

Default output is human-readable. `--json` emits one JSON object followed by one newline. Every
successful read response has `command`, `authority_mode`, `source_commit`, `freshness`, and the
command's ordered `documents`, `document`, `ready` or `blocked` payload. The nested `freshness`
object has exactly:

```text
last_successful_sync_at
local_generation
remote_generation
remote_observed_at
pending_mutations = { count, operation_ids }
freshness
```

Exit 0 means the requested bootstrap or read completed. Exit 2 is limited to malformed commands,
flags, keys and ref syntax. Exit 1 covers safe operational refusals, including `not_initialized`,
`bootstrap_busy`, `document_not_found`, `source_ref_unavailable`, `installed_beads_missing`,
`beads_checksum_mismatch`, `unsupported_beads_version`, `invalid_store`, `store_changed` and
`source_commit_mismatch`. For the new commands, stdout is empty on failure; stderr is one stable
JSON refusal object under `--json`, or `error[CODE]: MESSAGE` in human mode. Messages may identify
the document key but must not print artifact contents, credentials or an unredacted command
environment. Do not change the established contract-test JSON/exit behavior.

Bootstrap success reports `installed`, `reinstalled` or `unchanged`, the resolved source commit,
local generation, logical counts and export digest. It may report the common state directory, but
not the caller's archive or binary path. Human and JSON forms contain the same values.

### Shared store and atomic activation

Resolve the invoking checkout with only these outer-repository commands, using `CommandRunner`,
the existing cleared environment, `GIT_NO_LAZY_FETCH=1`, disabled prompts and no user/global Git
configuration:

```text
git rev-parse --show-toplevel
git rev-parse --path-format=absolute --git-common-dir
```

Require one absolute, existing, canonical common directory and a non-bare worktree. Refuse empty,
multi-line, relative, nonexistent or symlink-redirection results. Main and linked worktrees must
therefore resolve the same root without relying on upward `.beads` discovery. The only production
state location is:

```text
<git-common-dir>/plasmosome-work-state/
  bootstrap.lock
  current
  generations/
    generation-<unpredictable-safe-suffix>/
      plasmosome-work-state
      bd
      state.json
      repository/.git/
      repository/.beads/
      runtime/{home,xdg_config,xdg_cache,xdg_data,tmp,git_config_global}
```

`current` contains exactly one `generation-<safe-suffix>` basename and a trailing newline. The
generation's strict `state.json` contains schema version 1, `markdown-shadow`, source commit,
logical export SHA-256, a canonical document-plus-operational projection SHA-256, full
embedded-Dolt local generation, host target, wrapper and Beads binary SHA-256 values,
remote-observation state and pending operation ids. Owner and task-dependency facts do not belong
in this file. Deny unknown fields, absolute or traversing generation names,
duplicate operation ids, symlinked state components and inconsistent cross-field values.

Open `bootstrap.lock` without following a symlink and take a nonblocking exclusive `flock` for the
duration of bootstrap. Contention is `bootstrap_busy`; process exit releases the OS lock. Reads do
not wait on this lock: the launcher loads one complete `current` pointer and executes the immutable
generation it names. This lock carries no actor, token, expiry, lifecycle or authority meaning.

Create staging and final generation directories on the same filesystem below `generations/`.
Build only in a `.staging-<unpredictable-safe-suffix>` directory. After both binaries, repository,
state manifest and real snapshot all validate, sync written files, rename the directory to the
corresponding `generation-...` name, write a same-directory temporary pointer, sync it, and
atomically rename it over `current`. That last rename is the only activation point. An
interruption before it leaves the previous pointer intact; an interruption after it can expose
only a fully validated generation. Never edit an active generation's binaries or private Beads
repository, expose a public state-manifest writer, or remove an old or abandoned generation in
this task.

### Bootstrap and installed runtime

Bootstrap first loads the pin manifest from the invoking worktree and runs the existing full
archive-plus-binary verifier against the supplied ordinary files. This happens before creating or
opening clone state. It then takes the installation lock and resolves `REF^{commit}` once through
Task 045's local Git source loader. Preserve its local-only `rev-parse`, NUL-delimited tree walk,
literal-path log and exact `git show` rules; never fetch a missing object or fall back to the
working tree.

Copy the verified executable into the staging generation as `bd`, set only owner-readable and
executable permissions needed by the current user, sync it, then verify the copied bytes and exact
`bd version 1.1.2 (...)` output again. Extend `pin.rs` with a separate installed-runtime verifier
that selects the host target, rejects a missing, non-regular or symlinked executable, hashes it
against `binary_sha256`, and runs only `bd --version` under the isolated environment. Every read
uses that verifier before its first command whose cwd is the private Beads repository; reads never
need the original archive or extraction directory.

Also copy the currently running `plasmosome-work-state` executable into staging, reject a symlink
or non-regular source, sync it and record its SHA-256 in `state.json`. The installed executable
derives its generation from its own canonical path below the invoking clone's common directory and
requires that path and checksum to match the strict state manifest. The tracked launcher selects
this generation executable for every ordinary read, so neither Cargo nor the build tree is in the
read path. Reinstallation may replace the wrapper only by activating another complete generation.

Initialize `repository/` as a private Git repository, set an isolated fixture identity and
`dolt.auto-push=false`, then run the already-proven command shape:

```text
bd --sandbox init --stealth --skip-agents --skip-hooks --non-interactive
```

Reuse Task 045's typed loader and importer for the complete selected corpus. Add a strict sibling
`metadata.plasmosome_operational` object to each task row with schema version 1, no active owner and
an empty ordered task-dependency list; intents and specs must not carry it. Keep this data in Beads,
not in the pointer or freshness manifest. Verify the export and the
`plasmosome.authority-mode=markdown-shadow` and `plasmosome.source-commit=<resolved-sha>` KV values,
then make one explicit local commit:

```text
bd --sandbox dolt commit -m "bootstrap markdown-shadow <resolved-sha>"
```

Do not configure a Dolt remote. Query the committed identity with
`bd --readonly --sandbox --json vc status`; do not infer it from the source Git SHA or export hash.
Initialize remote observation to unknown and pending mutations to empty. Run the production fenced
snapshot reader against staging before it can be activated.

On a rerun, load the existing state manifest and use the supplied verified binary against a
disposable copy when the installed runtime is unusable. Validate the existing fenced snapshot,
including Beads-backed operational metadata. If target, pin, wrapper checksum, source commit,
export digest and documents equal the requested input, return `unchanged` without import, pointer
replacement, timestamp change or shared-store write. If only the installed wrapper or Beads
runtime is missing, corrupt or outdated, build another generation by copying the already-validated
repository, install both valid binaries, preserve its Dolt commit and the complete freshness and
operational state, and return `reinstalled` after atomic activation. Never reimport Markdown to
repair runtime files.

If the requested ref resolves to a different source commit, return `source_commit_mismatch` even
when its current logical export happens to compare equal. Do not reset state versions, forget
retired keys or erase the last remote observation. Malformed state, an unreadable/corrupt Beads
repository or a snapshot/state mismatch is a refusal, not a reason to reconstruct over possible
operational evidence.

### Fenced local snapshot and command safety

The pinned embedded engine opens writable lock/journal files and changes storage mtimes even with
`--readonly`; therefore it must never run directly against the shared active generation. After the
launcher selects a generation, the installed wrapper validates and hashes itself and the shared
installed Beads file, recursively copies only regular files/directories from that generation's
private repository and Beads binary into a new per-read `TempDir`, rejects symlinks and special
files, and hashes the copied binary again. It runs `bd --version` on the copy before opening the
copied repository. Every read then executes this exact disposable-repository sequence through
`CommandRunner`:

```text
bd --readonly --sandbox --json vc status
bd --readonly --sandbox export
bd --readonly --sandbox --json kv list
bd --readonly --sandbox --json vc status
```

Require both status responses to have schema version 1, branch `main`, the same full nonblank Dolt
commit, and that commit to equal `state.json.local_generation`. Decode the complete export with
Task 045's strict typed decoder; revalidate unique keys, typed targets and canonical ordering;
require its logical SHA-256, canonical document-plus-operational projection SHA-256 and source
commit to match the manifest; and require the KV result to contain exactly the expected Plasmosome
authority/source values. A mismatch is `store_changed` or `invalid_store`, never a partial answer.
Do not pass an output path to export.

Decode `metadata.plasmosome_operational` from every task row in the same export. Require exactly
schema version 1, either no active owner or one record containing nonblank `actor`, `session_id`,
`ownership_token`, `claim_operation_id` and canonical UTC `acquired_at`/`expires_at`, and unique
ordered `task_dependencies` keys whose targets exist; reject the sibling on intent/spec rows.
Presence means the owner is live for this offline projection. Do not use the workstation clock to
expire it. This task initializes only empty operational values; small typed builders may exercise
populated read cases, but no public command writes them.

Compute the operational projection digest from the strictly typed document plus its operational
sibling, serialized in canonical document order with fixed field order. It includes every owner
field and dependency position, and excludes only Beads' unrelated presentation fields. This makes
an uncommitted raw owner/dependency edit disagree with `state.json` even when `bd vc status` still
reports the prior commit.

Use one environment-cleared constructor for the common isolation values. It retains only the
minimum executable search path plus per-operation HOME/XDG/TMP/global-Git paths and the existing
`BD_DISABLE_METRICS=1`, `BD_DISABLE_EVENT_FLUSH=1`, `GIT_TERMINAL_PROMPT=0`,
`GIT_NO_LAZY_FETCH=1`, `GIT_OPTIONAL_LOCKS=0` and no-auto-push settings. Do not forward proxy, SSH
agent, credential, GitHub, cloud or user configuration variables. Give bootstrap and ordinary
reads separate pre-dispatch validators. The read validator accepts only the exact outer
`git rev-parse` forms, copied `bd --version` and the four read forms above in the current read's
temporary cwd. It rejects `ls-remote`, ref resolution beyond the locator, fetch, pull, push, sync,
remote management, Beads native projections, an output file, missing `--readonly`, missing
`--sandbox`, a shared-store cwd or another binary before the runner sees the plan. The bootstrap
validator separately admits only the exact local Git init/config and Task 045 source-read commands
plus pinned Beads init/import/KV/export/status/commit commands. A bootstrap write shape must be
accepted there and rejected by the read validator.

Snapshot the source worktree's tracked status, index, hooks and local configuration and the shared
generation's path/content/mode/mtime tree around real reads. Successful reads may create files only
inside their private temporary root; they may create no persistent file, queue, staged entry,
config change or background child and may not modify the source checkout or shared generation.
Explicitly close the `TempDir` and fail as `temporary_cleanup_failed` if complete removal fails;
reap every child first. No read resolves the stored source ref or checks whether the source
checkout's current HEAD moved; freshness comes only from the stored envelope.

### Freshness state and classifier

Keep observation state in atomically replaceable generation `state.json`, not Beads KV: recording
an observation inside Dolt would itself change the local generation it describes. Besides the
public envelope fields,
the strict manifest stores `remote_relation = equivalent | ahead | unknown` and
`observed_local_generation`. These record the local/remote pair established by a future online
operation even when the embedded Dolt hash and `refs/dolt/data` Git SHA use different encodings.
This task reads and validates those facts but exposes no public command that changes them.

Use canonical UTC `YYYY-MM-DDTHH:MM:SSZ` strings and validate real calendar/time ranges for both
`last_successful_sync_at` and `remote_observed_at` whenever present. An
`equivalent` or `ahead` relation requires remote generation, observation time and observed local
generation. `equivalent` also requires `last_successful_sync_at` equal to its remote observation
time and, without pending work, is valid only when the observed and current local generations
match. `ahead` without pending work also requires the current generation to remain the observed
local base. `unknown` may have no observation or may preserve the last remote generation, timestamp
and local comparison after a failed synchronization; paired values may not be split.
Remote generations are lower-case 40-hex `refs/dolt/data` observations; local and observed-local
generations must match the full nonblank commit form emitted by the pinned `bd vc status` parser.
Pending operation ids are nonblank, unique and preserved in recorded order.

Classify from one validated snapshot:

| Pending ids | Remote relation | Freshness |
| --- | --- | --- |
| empty | `equivalent` | `synchronized_as_of` |
| empty | `ahead` | `stale` |
| empty | `unknown` | `unknown` |
| nonempty | `equivalent` | `unpublished` |
| nonempty | `ahead` | `stale_with_unpublished` |
| nonempty | `unknown` | `unknown_with_unpublished` |

Never derive recency from the workstation clock, current source branch, Git remote, lexical hash
order or absence of an error. Human output for equality is exactly “synchronized as of <UTC>” and
neither form uses “current” or “up to date” for synchronized, stale or unknown state. Combined
states retain every last-known remote field and list every pending operation id.

### List, show, ready and blocked

`list` returns every decoded document in canonical intent/spec/task then numeric-id order. Each
summary contains document key, kind, three-digit id, title, lifecycle and optional priority.
`show` returns the complete stored logical record and Markdown-shadow fields for one exact key.
Namespace collisions such as `intent:014`, `spec:014` and `task:014` remain distinct. A valid key
which is absent is `document_not_found` and names that key only.

Compute readiness from the decoded document and Beads-backed operational metadata; never invoke
`bd ready` or `bd blocked`. A decoded active owner is live for an offline read: do not expire it
from the workstation clock. A dependency key blocks until its task lifecycle is `done`. Reject
malformed metadata, references to non-task keys and duplicate dependencies as invalid store state.

For every `todo` or `planned` task, collect deterministic blockers in this order:

1. `task_not_planned` for `todo`;
2. `live_owner` when an active owner is recorded;
3. one `dependency_not_done` per ordered unfinished dependency;
4. `missing_spec_links` when the copied spec list is empty;
5. one `spec_not_accepted` per copied spec whose lifecycle is not `accepted`;
6. `intent_closure_mismatch` when copied task intents differ in content, duplication or order from
   the exact first-seen ordered union of those specs' intent ids;
7. `missing_intent_links` when the expected union is empty; and
8. one `intent_not_approved` per first-seen intent named by either the expected union or copied
   task list whose lifecycle is not `approved`.

Task 045 already refuses missing typed targets while decoding, so a missing referenced record is
`invalid_store`, not a misleading readiness blocker. `ready` contains planned tasks with no
blockers. `blocked` contains `todo`/`planned` tasks with their full blocker list; `in_progress`,
`in_review` and `done` are absent from both rather than called blocked. Both preserve canonical
task order and include `authorizes_start: false` in JSON and the human sentence “local projection;
does not authorize start.” Freshness never changes membership, and no freshness value authorizes a
claim, start or dispatch.

### Contract cases

Extend the existing runner without weakening Task 042/045 cases:

```text
./tools/work-state contract-test local-reads --source-ref REF --archive PATH --bd PATH
./tools/work-state contract-test freshness --source-ref REF --archive PATH --bd PATH
./tools/work-state contract-test combined-freshness --source-ref REF --archive PATH --bd PATH
./tools/work-state contract-test all --source-ref REF --archive PATH --bd PATH
```

The harness resolves `REF` in the real repository, creates an untracked temporary mirror clone of
that real object database with `git clone --mirror --no-local`, then adds two detached linked
worktrees at the resolved commit. It starts no local Git server and copies no Markdown fixture.
Bootstrap from one worktree and prove both worktrees resolve the mirror's same absolute common
directory and current generation; a separately initialized mirror has a distinct store.

Because the selected document ref can predate this task's launcher, the harness supplies the
currently compiled CLI executable to the production bootstrap API and places the exact
repository-under-test `tools/work-state` bytes at the two temporary launcher paths before taking
the no-mutation snapshots. Those two runtime tooling files are not a copied work-record corpus or
a substitute store. This lets pre-commit integration tests exercise the real installed executable
and shell read path without writing state into the developer's actual Git common directory.

`local-reads` runs the production bootstrap twice, proves `installed` then `unchanged`, and runs
all four read commands in both human and JSON form from both worktrees through the actual tracked
shell entry point and installed wrapper. It asserts that ordinary entry points start no Cargo,
rustup or registry process and make no source/build/shared-generation write. It also checks exact
canonical document sets and namespace identity against the Task 045 source model, exact source
and local generation, readiness blocker order, no start authorization, complete temporary cleanup,
no metrics/event footprint, and only the allowlisted local command plans. Missing and
checksum-changed installed binaries refuse before any disposable-repository command. A fault
injected at each staging/rename boundary leaves either the old complete generation or the new
complete generation, never a partial pointer. Contending bootstrap processes produce one active
complete generation or one stable `bootstrap_busy` refusal.

`freshness` uses a narrow typed manifest builder and real committed Beads generations to exercise
an unobserved bootstrap, a known equivalent offline observation, an observed-ahead remote and a
committed unpublished local change. Production reads report `unknown`, `synchronized_as_of`,
`stale` and `unpublished` with exact fields and human wording. `combined-freshness` combines a real
committed local change with `ahead`, then with lost equality knowledge, and proves
`stale_with_unpublished` and `unknown_with_unpublished` preserve the remote observation and
pending id. No public mutation or synchronization command is added to create these fixtures.

Recording tests cover deterministic refusal paths at the existing narrow process seam. Successful
acceptance uses the supplied checksum-verified Beads binary, real Git commands and embedded Dolt;
it must not substitute recorded output for the installed store. `all` runs the existing cases and
these three. Do not add an `offline-reads` alias and do not mark a case skipped.

### TDD test table

Add the named test before its implementation, run the narrow command, and record the expected red
failure. Implement only after that failure is observed, then rerun the same command to green.
Several tests may be added together only when one production change makes that coherent batch
pass; never edit a test and its implementation in the same step.

| Test added first | Required red observation and eventual proof |
| --- | --- |
| `installed_binary_verification_needs_no_archive` (`pin`) | Installed verifier is absent; copied exact bytes/version pass, while missing, symlinked, checksum-changed and wrong-version binaries fail before store commands |
| `linked_worktrees_resolve_one_common_store` (`store`) | Locator/store model is absent; main and linked worktrees agree, another clone differs, and bare/relative/multiline/symlink cases refuse |
| `bootstrap_installs_a_complete_shadow_generation` (`store`) | Bootstrap is absent; real source documents, installed wrapper and Beads runtime, committed Dolt generation, export digest and authority/source KV all validate below common-dir only |
| `bootstrap_same_source_is_a_write_free_noop` (`store`) | Existing path rebuilds; rerun returns unchanged with the same pointer/tree/mtime and no import or commit command against shared state |
| `bootstrap_reinstalls_runtime_without_reimporting_state` (`store`) | Runtime repair loses ledger facts; a supplied valid runtime copies the validated repository into a new generation, preserves Dolt/freshness/owner/dependency state and never imports Markdown |
| `bootstrap_activation_survives_every_interruption_boundary` (`store`) | Direct writes expose partial state; staged faults and a contending lock leave one complete readable pointer and never overwrite old evidence |
| `bootstrap_refuses_a_different_source_commit` (`store`) | A changed ref resets records to state 1; every different resolved commit returns `source_commit_mismatch` without staging, import, pointer or observation change |
| `snapshot_reads_one_unchanged_committed_generation` (`store`) | Reader/plan validator is absent; a copied status/export/KV/status agrees, and changed generations, digest/KV/source/schema/path/pin mismatches refuse before output |
| `ordinary_read_uses_and_removes_a_disposable_store_copy` (`store`) | Direct Beads reads change active Dolt mtimes; only the TempDir copy changes and complete cleanup leaves source/shared path-content-mode-mtime snapshots equal |
| `ordinary_read_plans_are_local_and_write_free` (`command`/`store`) | Unsafe shapes reach the runner; exact temporary cwd/environment/flags pass and every ref-resolution, network, native-ready, output-file, shared-store cwd or write shape is refused before dispatch |
| `bootstrap_and_read_validators_are_distinct` (`command`/`store`) | One allowlist rejects required bootstrap writes or admits them to reads; exact init/import/KV/commit forms pass only bootstrap and fail read validation |
| `freshness_classifies_the_six_spec_states` (`freshness`) | Classifier is absent; the six table rows return exactly the accepted enum values and pending ids |
| `invalid_or_partial_observation_state_is_refused` (`freshness`) | Malformed metadata is accepted; bad UTC, split remote fields, invalid relation/base, duplicate/blank pending ids and impossible clean equality fail |
| `list_and_show_preserve_canonical_namespaces` (`read`) | Projections are absent; all records sort canonically, kind-colliding ids remain distinct, exact show returns complete fields and absent keys are stable |
| `ready_requires_every_governance_gate` (`read`) | Wrapper readiness is absent; planned accepted/approved exact-closure task passes locally and todo, owner, dependency, empty-link, nonaccepted, unapproved, duplicate, missing and reordered closure cases report exact blockers |
| `active_and_terminal_tasks_are_not_called_blocked` (`read`) | Naive not-ready logic overreports; only todo/planned tasks participate and every ready/blocked item says it cannot authorize start |
| `human_and_json_reads_carry_the_same_envelope` (`read`) | Renderer omits fields/uses “current”; all four commands carry six exact fields, combined pending ids and “synchronized as of” wording without a recency claim |
| `public_reads_need_no_artifact_arguments` (`cli`) | Parser accepts only contract-test; bootstrap alone requires artifacts/ref, reads reject them, show requires a qualified key, and exit/stdout/stderr behavior is exact |
| `ordinary_launcher_executes_installed_wrapper_without_cargo` (`cli`) | The script always runs Cargo; reads choose the safe pointer/runtime and no Cargo/rustup/build command, while bootstrap uses release locked/offline Cargo and contracts retain debug locked/offline Cargo |
| `bootstrap_launcher_uses_release_locked_offline_cargo` (`cli`) | Bootstrap lacks the release profile; fake Cargo records exactly `run --release --locked --offline --quiet -p plasmosome-work-state --` and forwarded bootstrap arguments, while `contract-test` retains the debug locked/offline argv and ordinary reads remain Cargo-free |
| `real_local_reads_share_one_clone_store` (`contract`) | `local-reads` case is absent; real mirror worktrees install once and return matching shell-entry-point results through disposable copies without checkout/build/shared-store mutation or a disallowed command |
| `real_freshness_cases_preserve_remote_and_pending_facts` (`contract`) | Cases are absent; real committed stores drive all six states through production readers and renderers without a public state-writing command |
| `all_includes_local_reads_and_both_freshness_cases` (`contract`) | Aggregate omits new evidence; one supplied real artifact run executes every old and new case without skip or hosted fixture |

### Execution order

1. Confirm the task worktree is still based on the planned `origin/main`, read every ref and run the
   existing crate tests before editing. If unrelated changes or a moved accepted spec alter this
   contract, STOP.
2. Add the installed-runtime tests in `tests/pin.rs`; run
   `cargo test -p plasmosome-work-state --test pin installed_` and observe red. Implement only the
   verifier split in `pin.rs`, rerun the focused test, then the full `--test pin`.
3. Add locator, layout, manifest and bootstrap tests in new `tests/store.rs` in the batches shown
   above. Before `store.rs` exists, run each named filter with
   `cargo test -p plasmosome-work-state --test store TEST_NAME` and preserve the red output. Add
   `libc.workspace = true`, accept the resulting `Cargo.lock` package-dependency update, add the
   module export and the minimum store implementation only after the corresponding batch is red.
   Rerun each filter and then the full `--test store`.
4. Add `tests/freshness.rs`, observe the classifier and validation tests red with
   `cargo test -p plasmosome-work-state --test freshness`, then implement `freshness.rs` and make
   that target green. Keep classification pure; no process or clock call belongs in it.
5. Add the fenced-snapshot and exact-command-plan tests to `tests/store.rs` and
   `tests/command.rs`; run their exact filters red. Generalize only the required shadow helpers and
   implement the reader after the tests fail. Run `--test store`, `--test command` and
   `--test shadow` to green before projection work.
6. Add list/show tests to new `tests/read.rs`, run those filters red, then implement and green them.
   In a separate test-only change add readiness cases, observe red, then implement readiness and
   rerun the full `--test read`. Add renderer tests last, observe red, implement, and green.
7. Add CLI parser/process tests in `tests/cli.rs`, run
   `cargo test -p plasmosome-work-state --test cli` red, then extend `main.rs` and
   `tools/work-state` without changing old contract behavior and rerun it green. Exercise the real
   script and prove ordinary reads do not start Cargo, bootstrap is release locked/offline, and
   contract development commands remain debug locked/offline.
8. Add one contract case at a time in `tests/contract.rs`: `local-reads`, `freshness`, then
   `combined-freshness`. Observe each exact filter red before changing `contract.rs`; implement and
   run that filter with the supplied real pinned artifact before adding the next case. Add the
   aggregate assertion last, observe it red, then extend `all`.
9. Update only the crate AGENTS/README boundary after behavior is green. They must say Markdown
   remains authoritative, bootstrap is the sole local installer/runtime-reinstaller, ordinary
   queries are offline projections that never authorize work, and full offline/heartbeat
   acceptance remains incomplete.
10. Run the crate test target, the three real contract commands, coverage review, refactor pass,
    timed workspace suite and root gate below. STOP; do not implement a deferred capability.

### Coverage, refactor pass and definition of done

Use an already-installed `cargo-llvm-cov`; never install a tool, accept an installer prompt or
change toolchain configuration in this task. Run:

```text
cargo llvm-cov --version
cargo llvm-cov --workspace --summary-only
rustup toolchain list
rustup component list --installed --toolchain nightly
cargo +nightly llvm-cov --workspace --branch --summary-only
```

The two `rustup` commands are local preflight only: require an already-listed nightly toolchain and
an already-installed `llvm-tools`/`llvm-tools-preview` component before invoking branch coverage.
Do not run `rustup toolchain install`, `rustup component add` or accept cargo-llvm-cov's proposed
installation. If either prerequisite is absent, stop branch coverage and record the exact blocker;
do not retry a command that can prompt.

Inspect uncovered branches in `pin.rs`, `store.rs`, `freshness.rs`, `read.rs`, CLI parsing and the
contract additions. Add tests for meaningful safety, state-validation, atomicity, command-plan,
ordering and rendering branches. Generated process exits and impossible I/O races may remain
uncovered only when the task Notes name them and real contract evidence exercises the successful
boundary. A missing coverage tool is a blocker to report, not permission to install or omit the
review.

After coverage, make a separate behavior-preserving refactor pass. Look for repeated manifest
validation, command construction, envelope rendering, task lookup and fixture setup. Run the
focused tests after every refactor; do not combine a behavior change with cleanup. Confirm the
production code still has one real process seam and no test-only branch, broad mock layer or
second hypothetical adapter.

Run real acceptance with caller-supplied local artifact paths and the real repository ref:

```text
./tools/work-state contract-test local-reads --source-ref origin/main --archive PATH --bd PATH
./tools/work-state contract-test freshness --source-ref origin/main --archive PATH --bd PATH
./tools/work-state contract-test combined-freshness --source-ref origin/main --archive PATH --bd PATH
./tools/work-state contract-test all --source-ref origin/main --archive PATH --bd PATH
```

Before PR, measure ordinary installed-wrapper latency outside the contract assertion: create a
fresh disposable clone/generation with the current tracked launcher and source, run the actual
release locked/offline bootstrap with the caller-supplied pinned artifacts and real source commit,
warm one artifact-free `list --json` read, then time five sequential
`env -i PATH=/usr/bin:/bin ./tools/work-state list --json` reads. Each response must exit 0 and
preserve the 69-document Markdown-shadow projection. This is a manual evidence gate, not a CI
assertion: its release median must be at most one third of a comparable debug median and at most
4.0 seconds on the current 13.33-second baseline machine. If it misses, STOP and return profiling;
do not weaken installed/copied hash or version verification, or the disposable-copy boundary.

Then time the ordinary suite and run all five root gates exactly:

```text
/usr/bin/time -p cargo test --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
./.githooks/provenance-guard
./.githooks/attribution-guard
```

Record in dated Notes the red/green sequence, exact artifact target and checksums, bootstrap and
contract evidence, the five-read release/debug timing sets and median ratio, six freshness outcomes,
coverage line/function/region and branch findings, refactor decision, timed suite result and all
gate results. Do not add an author or co-author other than the repository owner. The PR description
leads with the user-visible outcome: one bootstrap gives every worktree artifact-free, network-free
local queries, while Markdown remains authority. Completion means an agent in any linked worktree
can use the installed runtime to obtain honest local projections immediately, while every deferred
Spec 014 capability remains explicitly unclaimed.

## Notes

### 2026-09-02 — release-bootstrap timing remediation

Added `bootstrap_launcher_uses_release_locked_offline_cargo` before implementation. Its exact
focused command was red because fake Cargo recorded bootstrap without `--release`; after the
launcher-only split, the exact test was green and `cargo test -p plasmosome-work-state --test cli`
was green (9 tests). The test also records that `contract-test` keeps its debug locked/offline argv
and the existing ordinary-read test remains Cargo-free.

The fresh disposable real bootstrap used Beads 1.1.2 for `aarch64-apple-darwin`, verified archive
SHA-256 `9b0137a83a2afd343e2abd2a506be72ea032721000f76669c2cf81729e78501d` and binary SHA-256
`621b7b6c20c38db27ef4120398eb46dc35ba5b3e6c3611e19e14d33de10ce351`, and exited 0 in 22.01s.
It installed source `66583edc75de0fddcfe441273541850d4631b52d`, generation
`1ir7q489cu3geen0gmhi5mdu0ebqv9l8`, with 14 intent, 13 spec and 42 task documents. After one warm
read, five artifact-free ordinary release reads each exited 0 with the same 69-document
Markdown-shadow projection: 3.15, 3.22, 3.17, 3.25 and 3.19 seconds (median 3.19s). A comparable
five-read debug set also returned 69 documents each: 13.53, 13.21, 13.30, 13.30 and 13.34 seconds
(median 13.30s). The release/debug median ratio is 0.240, below one third and the 4.0s manual
threshold. This timing evidence does not claim the separate OS no-socket or full
`offline-reads` acceptance.

### 2026-09-02 — final execution evidence

The implementation followed the named test-first batches: installed runtime/pin verification,
common-directory locator and strict manifest/layout, fenced disposable snapshots, the six-state
freshness classifier, read projections and rendering, launcher behavior, and real contract
dispatch. Later safety reds showed a symlinked state root being classified as `not_initialized`
instead of `invalid_store`, and whitespace-padded manifest/local commit values being accepted;
the corresponding implementation changes made their focused tests green without weakening the
assertions. The complete crate suite was green at 119 tests.

Separate real pinned-artifact commands all exited 0 against `origin/main` resolved as
`66583edc75de0fddcfe441273541850d4631b52d`: `local-reads` (generation
`1jb0pvodonvfmggh6kov13asm6netv1m`), `freshness` (generation
`na9u1qd5go3edv1odc0lpfek0kcm6ted`, pending `operation-contract-046`), and
`combined-freshness` (generation `ihpdldllk21d71lp95s9rik4db9pkvc7`, two pending ids). The final
aggregate `all` also exited 0 with the same 14 intent, 13 spec and 42 task documents, logical
digest `645de42eecf42b8e00d6eed81bf3f5a5077127cc7bb7afdbc09466cbb9ea74fb`, and all three new
scenarios. Together, the real production reads cover `unknown`, `synchronized_as_of`, `stale`,
`unpublished`, `stale_with_unpublished`, and `unknown_with_unpublished`.

Coverage preflight found `cargo-llvm-cov 0.6.21`, an already-installed nightly and
`llvm-tools`; no toolchain change was made. Final stable coverage was 75.61% regions, 71.25%
functions and 77.31% lines. Final nightly branch coverage was 25.63% regions, 26.39% functions,
26.93% lines and 42.26% branches; it emitted the pre-existing deprecated `fetch_update` warning in
`plasmosome-membrane`. Safety/validation, atomicity, local command-plan, readiness, rendering and
new launcher branches are covered. Remaining lower store/contract coverage is primarily failed
filesystem/subprocess cleanup and interruption I/O that cannot be deterministically induced
without a broad mock layer; real contracts exercise the successful process boundary.

The separately run behavior-preserving cleanup simplified the renderer empty-details branch,
removed needless decoder borrows and returned staged closure results directly; focused read,
shadow and store tests stayed green. Final gates all exited 0: `/usr/bin/time -p cargo test
--workspace` (real 6.74s, user 2.55s, sys 3.52s), a separate `cargo test --workspace`, Clippy with
warnings denied, format check, provenance guard, attribution guard and `git diff --check`.

This evidence covers clone-local local reads and freshness only. It does not claim `heartbeat
observe`, an OS-level no-socket harness, or full Spec 014 `offline-reads`/cutover acceptance.
