---
id: 047
title: Synchronize the installed Beads shadow online
status: in_progress
priority: 1
specs: [014]
intents: [015]
refs: [AGENTS.md, README.md, Cargo.toml, Cargo.lock, .agents/skills/planning-work/SKILL.md, .agents/skills/tasks/SKILL.md, .agents/skills/pr-review/SKILL.md, docs/intents/015-local-first-shared-work-state.md, docs/specs/014-local-first-work-state.md, tasks/042-beads-transport-foundation.md, tasks/045-beads-shadow-import.md, tasks/046-beads-local-reads.md, tools/work-state, tools/work-state-beads-1.1.2.toml, crates/plasmosome-work-state/Cargo.toml, crates/plasmosome-work-state/AGENTS.md, crates/plasmosome-work-state/README.md, crates/plasmosome-work-state/src/lib.rs, crates/plasmosome-work-state/src/command.rs, crates/plasmosome-work-state/src/contract.rs, crates/plasmosome-work-state/src/freshness.rs, crates/plasmosome-work-state/src/main.rs, crates/plasmosome-work-state/src/pin.rs, crates/plasmosome-work-state/src/read.rs, crates/plasmosome-work-state/src/shadow.rs, crates/plasmosome-work-state/src/store.rs, crates/plasmosome-work-state/tests/cli.rs, crates/plasmosome-work-state/tests/command.rs, crates/plasmosome-work-state/tests/contract.rs, crates/plasmosome-work-state/tests/freshness.rs, crates/plasmosome-work-state/tests/read.rs, crates/plasmosome-work-state/tests/shadow.rs, crates/plasmosome-work-state/tests/store.rs]
done_when:
  - After the current wrapper is installed with the existing checksum-verified bootstrap, ./tools/work-state sync [--json] runs that installed wrapper without Cargo, artifact paths, caller ref, remote, or credential.
  - A compiled strict project binding fixes project plasmosome, remote origin, observation URL https://github.com/teonimesic/plasmosome.git, Dolt URL git+https://github.com/teonimesic/plasmosome.git, and refs/dolt/data; any unbound configuration refuses before network.
  - Sync verifies installed wrapper and pinned Beads, observes before and after one fresh staged remote clone, and atomically activates only exact same-authority, source, logical, and operational Markdown-shadow parity.
  - Missing ref, transport loss, moving ref, pending operations, and remote mismatch never write remote or expose cloned data, and retain strongest justified freshness without erasing prior sync or pending ids.
  - Bootstrap and sync share one nonblocking activation lock; readers stay lock-free; interruption exposes an old or fully validated new generation only.
  - Sync's allowlist has no push, force, ref update, fetch, arbitrary URL, shell, or ambient credential path; Task 042 helpers remain contract scaffolding.
  - Strict test-first reds, real pinned local-store plus recorded-transport online-sync contract, coverage and branch review, timed suite, and five root gates pass; no writer, lifecycle, claim, heartbeat, reconciliation, backup, or cutover acceptance is claimed.
pr: 84
evidence:
---

## Why

Task 046 made the shared offline projection usable, but later writers cannot safely start until production binds the right project remote, observes it, stages a fresh clone, validates it, and records freshness without touching active state.

This task adds one explicit installed-wrapper sync command that observes the pinned data ref, clones only into fresh staging, atomically activates an exact compatible Markdown shadow, and otherwise retains local data with honest freshness. Markdown remains authoritative for every imported fact.

## Plan

### Deliverable, in one sentence

Add an explicit zero-remote-write installed-wrapper synchronization command that observes the compiled project remote, clones only into fresh staged Beads storage, validates exact Markdown-shadow parity, atomically activates a complete compatible generation, and reports the complete freshness envelope.

### Authority boundary and out of scope

The current store is markdown-shadow. Spec 014 permits that mode only as one-way Markdown import, and existing shadow validation requires state_version equals 1. Task 047 therefore provides production online remote observation and synchronization of the installed shadow with zero remote writes. Do not add a library-only speculative writer, successful lease, receipt, version, lifecycle command, or ledger mode.

- No remote push, force, update-ref, refspec, or creation of refs/dolt/data.
- No writer lease, fence, admission, renewal, release, expiry, takeover, typed receipt, fingerprint, expected document version, state-version increment, or ledger mode.
- No lifecycle, priority, PR, evidence, dependency, owner, claim, approval, acceptance, task, branch, dispatch, or external-effect mutation.
- No GitHub API, reconciliation, heartbeat, freeze, epoch, dual authority, cutover, backup, restore, rollback, garbage collection, hosted/live remote fixture, GitHub mock, local Git server, broad mock, daemon, container, paid service, implicit read sync, source-ref resolution, Markdown reimport, dependency, or toolchain installation. Do not run or edit heartbeat.

Task 048 owns typed project lease and receipt schemas, acquire/renew/release/takeover, guarded first creation/publication when the data ref is absent, non-forcing expected-base publication, request fingerprint/idempotency, retries, stale conflict, lost-response recovery, and ledger-gated work. Task 049 owns expected document version, increments, typed lifecycle receipts, and transition graph/gates. Claims, effects, and cutover remain later work. STOP when this plan passes; do not begin Task 048 or Task 049.

### Files to read and exact edit boundary

Read only this task and the files in refs. Do not explore unrelated crates, tasks, branches, or the concurrent work for Tasks 030, 043, and 044.

Create or edit only new tools/work-state-project.toml, src/project.rs, tests/project.rs, src/sync.rs, and tests/sync.rs; focused src/lib.rs, src/main.rs, src/store.rs, src/freshness.rs, and src/contract.rs; focused tests/cli.rs, tests/store.rs, tests/freshness.rs, and tests/contract.rs; tools/work-state; the work-state crate AGENTS.md and README.md; and this task for lifecycle/dated Notes. Do not edit document.rs, shadow.rs, read.rs, pin.rs, Cargo manifests/lockfile, root instructions, Spec 014, existing intent/spec/task content, or any other file. A required change outside this list is a STOP.

### Compiled project binding and public command

Track tools/work-state-project.toml exactly:

    schema_version = 1
    project_id = "plasmosome"
    remote_name = "origin"
    git_observation_url = "https://github.com/teonimesic/plasmosome.git"
    dolt_remote_url = "git+https://github.com/teonimesic/plasmosome.git"
    data_ref = "refs/dolt/data"

ProjectConfig denies unknown and duplicate fields and validates the exact schema, project id, remote name, credential-free plain HTTPS Git-observation URL, canonical git+https Dolt URL, and ref. Reject userinfo, query, fragment, controls, whitespace, alternate scheme, host, repository, URL pairing, or ref. Load it only with include_str! in the checksum-bound installed wrapper, never a mutable checkout. The wrapper hash binds it; do not bump the manifest schema. Same-source new-wrapper bootstrap reinstalls safely while preserving source, freshness, pending ids, owners, dependencies, and digests.

Add only ./tools/work-state sync [--json] and ./tools/work-state contract-test online-sync --source-ref REF --archive PATH --bd PATH; include online-sync exactly once in all. Sync accepts only one optional --json and no project/ref/URL/remote/artifact/store/token/actor/session/operation/credential/timestamp flags. Syntax refusal is exit 2 invalid_command; ordinary launcher execution uses the installed wrapper, never Cargo, rustup, build, or registry.

Exit-0 output carries command=sync, project_id=plasmosome, outcome=synchronized, authority_mode=markdown-shadow, source_commit, and the exact Task 046 freshness envelope. Human output says synchronized as of, never current. Exit-1 codes include not_initialized, invalid_project_config, sync_busy, store_changed, installed_beads_missing, beads_checksum_mismatch, unsupported_beads_version, remote_uninitialized, remote_transport, invalid_remote_observation, remote_configuration_mismatch, remote_changed, pending_mutations, remote_shadow_mismatch, and temporary_cleanup_failed. A refusal has empty stdout and one stable stderr. Cleanup/activation failure wins; a safe observation-only activation says state_changed=true, otherwise false.

### Mandatory pinned-Beads discovery gate

Before implementation, use supplied pinned Beads 1.1.2 and an isolated initialized store to capture machine-readable bd --sandbox --json dolt remote list, prove that bd --sandbox init --remote git+https://github.com/teonimesic/plasmosome.git --stealth --skip-agents --skip-hooks --non-interactive works in embedded mode, and re-list its exact binding. STOP and return to planning if v1.1.2 lacks deterministic name-and-URL output, init needs server mode, the canonical Dolt URL cannot be represented, or the existing data ref cannot be cloned without a writer. Do not add a permissive text parser, alternate backend, server, or broad seam.

### Sync sequence and fence

Preflight before network locates with the sealed local locator, binds canonical current_exe generation, verifies wrapper/manifest/installed binary/authority/complete fenced snapshot/compiled config, and uses active source plus exact logical/operational projection as comparison truth. Sync never reads Markdown or resolves a source ref.

Only these new staging commands are allowed after a valid R0 observation:

    git ls-remote --exit-code https://github.com/teonimesic/plasmosome.git refs/dolt/data
    <staged-bd> --sandbox init --remote git+https://github.com/teonimesic/plasmosome.git --stealth --skip-agents --skip-hooks --non-interactive
    <staged-bd> --sandbox --json dolt remote list

Existing staged-runtime version and readonly fenced snapshot commands remain allowed for the fresh staged repository. Redact URL argv. A sync validator independently binds compiled config, executing generation, staging root, staged binary, cwd, runtime dirs, full environment, and phase before the system runner. Its cleared environment has only PATH, exact private BEADS_DIR, staged HOME/XDG/TMP/global-Git, BD_BACKUP_ENABLED=false, current telemetry/prompt suppression, and Git safety flags; no token/helper/askpass/SSH-agent/proxy/user HOME/global Git/Beads/Dolt credentials. Refuse before dispatch remote-add, pull, bootstrap, push, fetch, force, refspec, update-ref, shell, wrong binary/root/cwd/URL/ref/remote/environment, shared active cwd, mutable config load, replay, and out-of-order commands.

Every bd --version invocation uses an explicitly bound private runtime-owner cwd; cwd=None, invoking-checkout, shared-active-repository, and outside-root forms refuse before dispatch. No synthetic actor is supplied; only the exact private-repository git config user.name fallback is an admitted local read.

Use physical bootstrap.lock, generalized as generation-activation lock, so bootstrap and sync contend on the same inode. Bootstrap returns bootstrap_busy; sync returns sync_busy; reads remain lock-free. Under the lock, reread current, prepare only a bound staging root and runtime without a repository, and observe R0 with the plain HTTPS Git URL from that bound staging cwd before any Beads command. An empty exit-2 no-match returns remote_uninitialized without a Beads remote command. If a pre-R0 refusal has changed valid observation facts requiring an atomic metadata-only Unknown generation, copy the immutable active repository only under the existing regular/no-symlink/mode/fsync rules; an identical failure envelope is write-free. For valid R0 with no pending ids, create a same-filesystem fresh staged repository; never copy or pull active Dolt history. Initialize that fresh repository exactly once with the canonical Dolt URL, require its exact JSON remote-list binding, observe R1, require R1 equals R0, then run status/export/KV/status parity. Pending ids stop before init/remote clone and may only activate justified observation metadata. Complete staged manifests, recursively fsync, rename, and atomically replace current. TempDir cleanup wins; abandoned staging remains retained.

The candidate must be markdown-shadow, source-equal, and exact in document key/kind/id/path/title/content SHA/state version/ordered links/lifecycle/priority/PR/evidence, task owner and dependency, logical and operational digest. No extra/missing row, unknown Plasmosome KV, receipt, lease, or writer metadata is allowed. Internal Dolt generation may differ only with exact projection parity. Any difference is remote_shadow_mismatch, without reimport, activation, or remote overwrite. One status/export/KV/status fence stays unchanged after observation.

Remote observation accepts exactly one lowercase 40-hex SHA paired with refs/dolt/data. Successful malformed/uppercase/duplicate/extra/wrong-ref/empty output is terminal invalid_remote_observation; empty no-match from ls-remote --exit-code is remote_uninitialized; other nonzero is remote_transport; never parse unconstrained stderr.

### Freshness and atomicity

No successful observation is Unknown while preserving prior complete observation, last_successful_sync_at, local generation, and pending ids without inventing time. Failure before R0 preserves the prior complete historical observation or none without a new timestamp; failure after R0 may record complete Unknown R0/time. R0 different from R1 records R1/time Unknown, returns remote_changed, and exposes no candidate. Stable clone/parity failure records stable remote as Unknown plus remote_shadow_mismatch. Pending at the same synchronized base may be Unpublished; pending at changed/unknown base is UnknownWithUnpublished, with ids preserved in order. Stable valid clone samples post-R1 observation and successful-sync times as SynchronizedAsOf. Equivalent reobservation may postdate a successful sync, canonical UTC never moves backward, the six classifiers remain, and an identical Unknown retry is write-free.

Use narrow crate-private store helpers, not a general manifest writer or duplicated filesystem logic. The lock spans current revalidation, staging, remote work, failure-state preparation, and activation; a selected reader remains on its old generation.

### Strict test-first matrix

| Test | Proves |
| --- | --- |
| project_config_accepts_only_the_compiled_plasmosome_remote_pair | Exact compiled observation/Dolt URL pair only; malformed or alternate forms refuse. |
| compiled_project_config_is_bound_by_the_installed_wrapper | Wrapper reinstall preserves facts; no checkout config. |
| sync_cli_accepts_only_optional_json | Every forbidden option refuses. |
| sync_launcher_executes_only_the_installed_wrapper | No Cargo/rustup/build route; old routes remain. |
| sync_refuses_when_the_selected_generation_is_no_longer_current | Pointer flip creates no network/staging mutation. |
| bootstrap_and_sync_contend_on_one_generation_lock | Same lock gives distinct deterministic busy codes. |
| sync_runner_binds_every_command_before_dispatch | Wrong command/root/env/order/replay reaches no runner. |
| sync_runner_rejects_every_remote_write_shape | remote-add/pull/bootstrap/push/fetch/force/refspec/update-ref/shell pre-dispatch refuse. |
| remote_list_accepts_only_the_exact_canonical_git_transport_binding | The staged JSON remote list has exactly origin, canonical git+https URL/sql_url, and status ok. |
| remote_observation_is_one_exact_lowercase_data_ref | Strict parser and terminal malformed state. |
| remote_no_match_stops_before_any_beads_remote_command | Empty exit-2 no-match is remote_uninitialized without staging/init/list. |
| remote_no_match_and_transport_are_distinct | Output/status distinguish no-match and transport. |
| existing_remote_is_cloned_into_fresh_staging_not_pulled_into_local_history | Valid R0 initializes fresh private staging; no active history copy, remote-add, or pull. |
| failed_sync_records_unknown_without_erasing_history | Failure preserves history/pending and invents no time. |
| equivalent_reobservation_may_postdate_successful_sync | Valid pending reobserve does not regress. |
| pending_mutations_are_observed_but_never_cloned_over | Pending ids yield honest state and stop before remote clone. |
| moving_remote_never_activates_the_cloned_candidate | R0 different from R1 exposes no candidate. |
| stable_sync_requires_complete_shadow_parity | Every authority/source/logical/operational/row/KV mismatch refuses. |
| stable_compatible_sync_activates_one_complete_generation | Stable candidate activates one complete generation. |
| sync_failure_activation_is_idempotent | Changed metadata activates once; repeat is write-free. |
| sync_activation_survives_every_interruption_boundary | Only old or complete new state is exposed. |
| sync_cleanup_failure_precedes_remote_refusal | Cleanup wins and cannot replace pointer. |
| sync_human_and_json_results_carry_the_same_freshness | Equivalent forms do not leak data. |
| online_sync_contract_uses_real_local_stores_and_recorded_transport | Real-local and recorded-remote boundaries stay distinct. |
| all_includes_online_sync_once | Aggregate includes the case exactly once. |

Each named test is a test-only edit that must show a meaningful red before implementation; never weaken assertions, create aliases that cannot discriminate, or edit test and implementation together.

### Execution order, coverage, and gates

0. Confirm exact base, read refs, and run cargo test -p plasmosome-work-state.
1. Run pinned CLI discovery gate and STOP on mismatch.
2. Project test/red/implementation/green; freshness test/red/minimum implementation/full green.
3. Coherent store lock/bootstrap/staging/activation/cleanup batches, each red before implementation and full store green after each.
4. Sync parsing/fence, failure/pending, parity/success, and rendering batches, each red first and full sync green.
5. CLI/launcher tests red before main/launcher changes; contract test red before contract change, real pinned online-sync, then all assertion last/red.
6. Docs after behavior; coverage; separate behavior-preserving refactor; acceptance; timed gates; independent review.

The contract uses real source object database and caller-pinned artifacts, a real temporary shared shadow/current wrapper, Git observation plus network-clone boundary, a real prebuilt pinned local candidate afterward, fenced snapshots/parity/staging/activation/linked-worktree/cleanup/lock, and RecordingCommandRunner only for remote observation/clone outcomes. Cover stable equality, identical-projection ahead candidate, missing ref, transport before/after R0, R0/R1 moving, pending, all parity mismatch classes, unknown KV/extra row, and all activation interruptions. Prove zero remote-add/pull/bootstrap/push/force/update/fetch/credential/effect/lifecycle/Markdown import, and label real-local versus recorded-remote evidence separately.

Never install or prompt. Record a missing prerequisite instead of changing it:

    cargo llvm-cov --version
    cargo llvm-cov --workspace --summary-only
    rustup toolchain list
    rustup component list --installed --toolchain nightly
    cargo +nightly llvm-cov --workspace --branch --summary-only
    ./tools/work-state contract-test online-sync --source-ref origin/main --archive PATH --bd PATH
    ./tools/work-state contract-test all --source-ref origin/main --archive PATH --bd PATH
    /usr/bin/time -p cargo test --workspace
    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --all -- --check
    git diff --check
    ./.githooks/provenance-guard
    ./.githooks/attribution-guard

Inspect project/sync/freshness/store activation/CLI/cleanup branches; do not build a broad mock for coverage. Dated Notes record every red/green, target/archive/binary checksums, remote CLI output shape, all freshness outcomes, lock/interruption/cleanup/worktree evidence, real-local/recorded-remote distinction, coverage metrics/branch analysis, refactor, timed suite/gates, and zero push/writer/lease/receipt/mutation/heartbeat/hosted/server/credential/cutover claim.

STOP if base/Spec014/Task046 moves incompatibly, an edit leaves this allowlist, a dependency or tool install is needed, active state would mutate, exact parity cannot be established, the existing data ref cannot be cloned without a writer, remote transport needs credential/server/mock, failure freshness cannot be represented atomically, an existing read gains network behavior, a test only turns green by weakening, or implementation approaches Task048/049. First absent-ref publication is deferred to Task048 expected-absent writer work.

## Notes

### 2026-09-02 — mandatory pinned-Beads discovery and normative correction

Using the caller-supplied verified Beads 1.1.2 archive and binary (darwin arm64; archive SHA-256 `9b0137a83a2afd343e2abd2a506be72ea032721000f76669c2cf81729e78501d`, binary SHA-256 `621b7b6c20c38db27ef4120398eb46dc35ba5b3e6c3611e19e14d33de10ce351`), an isolated embedded store returned `[]` from `bd --sandbox --json dolt remote list`. The exact plain-HTTPS remote-add probe was accepted but re-listed as a single deterministic object with `url` and `sql_url` `git+https://github.com/teonimesic/plasmosome.git` and `status` `ok`; it cannot satisfy an exact plain-HTTPS remote-list binding. `git ls-remote --exit-code https://github.com/teonimesic/plasmosome.git refs/dolt/data` returned exit 2 with empty stdout, while `bd --sandbox dolt pull --remote origin` failed in embedded mode because it attempted `origin/main` and found no branches. No product or test edit occurred before this result.

Planner correction: bind separate exact observation and Dolt transport URLs; do not add or pull a remote. An absent data ref is `remote_uninitialized` and issues no Beads remote command. A valid observed data ref may only enter fresh private staging through exact canonical `bd --sandbox init --remote ...`; first absent-ref publication remains Task048. This correction is task-only and precedes first TDD red.

The corrected isolated probe then ran exact `bd --sandbox init --remote git+https://github.com/teonimesic/plasmosome.git --stealth --skip-agents --skip-hooks --non-interactive` with exit 0 in embedded mode. Its output said `Remote has no Dolt data yet; initialized a fresh local database`, configured `origin` to the canonical URL, and reported Dolt embedded mode. The subsequent JSON remote list exited 0 with exactly one object: `name` `origin`, both URL fields the canonical git+https URL, and `status` `ok`. This proves the strict canonical init/list form, while confirming that the R0 absent-ref check must precede every Beads remote command.

A second isolated probe started in an empty non-Git directory. The same exact init command exited 0 and reported `Initialized git repository`, so the pinned embedded client creates its private Git repository itself; no additional production `git init` command is required or allowed. It again reported absent Dolt data and initialized a fresh local database, which remains prohibited from activation by the R0 existing-ref guard.

### 2026-09-02 — TDD: compiled project binding

Test-only `project_config_accepts_only_the_compiled_plasmosome_remote_pair` first failed to compile because `plasmosome_work_state::project` did not exist. The implementation then added the checksum-bound `include_str!` configuration and exact parser; the focused command passed with 1 test. The parser accepts only the compiled schema/project/name/observation URL/Dolt URL/ref tuple and rejects alternate, duplicate, or unknown fields as `invalid_project_config`.

### 2026-09-02 — TDD: online freshness facts

Test-only `equivalent_reobservation_may_postdate_successful_sync` first failed with `invalid_freshness`; the old validator required equality between successful-sync and observation timestamps. Test-only `failed_sync_records_unknown_without_erasing_history` then failed to compile because `record_failed_sync_observation` did not exist. The minimum freshness implementation preserves prior successful-sync/local/pending facts, records a complete post-R0 Unknown observation, and permits a later equivalent re-observation without moving the successful-sync timestamp. `cargo test -p plasmosome-work-state --test freshness` then passed all 7 tests. Post-hoc boundary coverage `failed_sync_observation_refuses_a_regressing_timestamp` passed, confirming that a new failure observation cannot move canonical UTC time backward.

### 2026-09-02 — TDD: shared generation-activation lock

Test-only `bootstrap_and_sync_contend_on_one_generation_lock` first failed to compile because `GenerationActivationLock` did not exist. The implementation extracted the existing nonblocking physical `bootstrap.lock` into that shared lock, retaining the bootstrap adapter and mapping contention to `bootstrap_busy` or `sync_busy` by caller. The focused new and existing bootstrap-contended tests passed, followed by `cargo test -p plasmosome-work-state --test store` with 25 passing tests.

### 2026-09-02 — TDD: sealed observation and fresh-clone command fence

Test-only `sync_runner_binds_every_command_before_dispatch` first failed because the sync module did not exist; the narrow ordered runner then bound only R0 Git observation, explicit fresh-clone authorization, exact canonical Beads init, exact JSON remote-list, and R1. Its focused green covers program/argv/cwd/root/URL/ref/environment/order/replay refusal before dispatch. `sync_runner_rejects_every_remote_write_shape`, `remote_no_match_and_transport_are_distinct`, and `remote_list_accepts_only_the_exact_canonical_git_transport_binding` were added as post-implementation boundary coverage and started green. `remote_observation_is_one_exact_lowercase_data_ref` exposed a real red: malformed status-0 R0 returned `invalid_remote_observation` but left the runner retryable. The fix terminally rejects malformed observations and malformed remote-list output before a later command can dispatch. This runner admits no remote-add, pull, bootstrap, push, fetch, force, refspec, update-ref, or shell form.

### 2026-09-02 — TDD: pending observation stops before clone

Test-only `pending_mutations_are_observed_but_never_cloned_over` first failed to compile because clone authorization accepted no pending-operation input. The sync fence now accepts the ordered pending IDs at that decision point, returns `pending_mutations` for a nonempty set, and remains in the pre-init phase; the attempted Beads init is refused before dispatch. The full sync test target then passed 6 tests. Freshness persistence for the outer sync operation remains a later, separately tested store/sync batch.

### 2026-09-02 — TDD: stable observation and result rendering

Test-only `moving_remote_never_activates_the_cloned_candidate` first failed because no R0/R1 stability gate existed. The pure runner state now returns `remote_changed` when a completed R1 differs or disappears, and `remote_transport` for R1 transport; equal exact observations return their shared generation. Test-only `sync_human_and_json_results_carry_the_same_freshness` first failed because the sync result and renderer did not exist. The success result now serializes the exact freshness envelope and the human rendering says `synchronized as of`, never `current`. Focused tests for both are green; no staging, filesystem, network, or activation behavior was added in this batch.

### 2026-09-02 — TDD: lock-bound sync staging and initial orchestration

The store façade tests were added before their implementation. `sync_staging_contains_only_verified_runtime_before_r0` first failed because `prepare_sync_staging` did not exist; `fresh_sync_repository_is_empty_and_created_exactly_once` then failed because the opaque staging type had no repository transition; `sync_candidate_finalizes_only_after_exact_readonly_fence_and_shadow_parity` failed because candidate finalization did not exist; `metadata_only_unknown_generation_copies_active_repository_only_for_a_changed_valid_observation` failed because no metadata transition existed; and `sync_activation_survives_every_interruption_boundary` failed because only an opaque validated candidate may activate. The green façade retains a same-filesystem stage containing only verified mode-0700 wrapper, pinned `bd`, and sealed runtime before R0; it creates a repository exactly once only after R0, verifies copied `bd --version` before either candidate or metadata readonly fencing, derives manifests internally, and makes unchanged Unknown observations write-free. `sync_staging_requires_the_lock_for_the_same_unchanged_current` was added after the lock/current binding was already present and began green; it is honestly post-hoc boundary coverage for wrong-location locks and pointer flips.

Test-only `stable_compatible_sync_activates_one_complete_generation` first failed to compile because `SyncClock` and `synchronize_with_clock` did not exist. Its green recorded sequence is R0, one fresh empty repository, exact `init --remote`, exact JSON remote list, R1, copied-binary version, and readonly status/export/KV/status before one opaque activation. The system clock remains internal; callers cannot supply a timestamp. `remote_no_match_stops_before_any_beads_remote_command`, `pending_mutations_are_observed_but_never_cloned_over`, and `sync_refuses_when_the_selected_generation_is_no_longer_current` were added after that orchestration and began green, so they are post-hoc evidence: no-match dispatches only Git and preserves current; pending work records a complete Unknown observation while preserving ordered ids and never dispatches init; a changed pointer refuses `store_changed` before staging or network. This batch makes no remote write, does not copy active history into a success candidate, and does not read Markdown.

### 2026-09-02 — TDD: installed sync route and contract syntax

Test-only `sync_cli_accepts_only_optional_json` first produced a meaningful red: valid `sync` exited 2 `invalid_command` instead of the initialized-store preflight refusal. The initial duplicate-`--json` formatting expectation was corrected to the established JSON-envelope behavior without changing the syntax assertion; then the narrow `main.rs` route/parsing implementation made the focused test green. Test-only `sync_launcher_executes_only_the_installed_wrapper` first exited 2 because the launcher omitted `sync`; adding it solely to the existing installed-wrapper route made it green and proved no Cargo invocation. `online-sync` contract syntax was then added to the existing source-ref parser after its test-only admission first failed `invalid_command`. After a private immutable synchronization-context refactor required by the argument-count lint, `cargo clippy -p plasmosome-work-state --all-targets -- -D warnings`, library tests (34), sync tests (10), and CLI tests (15) all passed. No remote, credential, writer, lease, receipt, lifecycle, heartbeat, or cutover behavior was added.

### 2026-09-03 — TDD: online-sync contract selection and retained local diagnostic

Test-only `all_includes_online_sync_once` first failed to compile because `online_sync_contract_cases` did not exist; the contract implementation then made the individual and aggregate selectors enumerate exactly one `online-sync` case, and the focused contract target passed all 40 tests. The `requires_shadow_round_trip` assertion was also added test-first: it initially failed because `online-sync` was omitted, then passed once the real source/shadow-round-trip prerequisite was included. The contract uses the real temporary shared shadow/current wrapper, installed runtime, fenced snapshots, parity, staging, activation, linked worktrees, and cleanup; only the Git observation and remote-clone boundary use the ordered recording runner. After that recorded boundary, a real prebuilt local candidate is placed in the fresh staged repository so version/fence/parity and activation remain physical. Its emitted evidence labels this distinction and explicitly records no remote add, pull, bootstrap, push, fetch, force, or update-ref.

The retained pinned-artifact `contract-test online-sync` diagnostic completed with exit 0, empty stderr, and `/usr/bin/time -p` `real 178.29`, `user 145.22`, `sys 16.31`. Its JSON named `outcome: passed`, `authority_mode: markdown-shadow`, source commit `c5aa18db2f9fa62065ac181e78962712420e8140`, and 70 documents (14 intents, 13 specs, 43 tasks); the count is the exact base-tree count and excludes the uncommitted Task047 plan. It reported the separate real-local/recorded-remote/no-write labels above. This retained run is diagnostic evidence only; final acceptance still requires the complete final-head contract matrix, aggregate case, coverage, and root gates. `cargo test -p plasmosome-work-state` then passed 34 library, 15 CLI, 40 contract, 25 store, 10 sync, and all remaining focused targets; clippy, fmt check, and diff check were green.

### 2026-09-03 — completion remediation: pending facts, orchestration, and strict installed-wrapper binding

The completion audit returned this task from `in_review` to `in_progress` before the following batches. Test-only `pending_at_the_last_equivalent_generation_remains_unpublished` and `pending_at_a_different_or_unknown_generation_is_unknown_with_unpublished` first exposed that a later remote observation could lose the strongest justified pending classification. The green implementation retains ordered pending ids and historical successful-sync facts: a later equal remote remains Equivalent and classifies Unpublished; a different or unknown base records (only when justified) Unknown and classifies UnknownWithUnpublished without moving canonical time backward.

The public-orchestration test `existing_remote_is_cloned_into_fresh_staging_not_pulled_into_local_history` was added before the materializing contract runner behavior. It calls public synchronization, intercepts only the admitted exact `bd init --remote` boundary, proves the repository cwd is newly empty, and materializes the prebuilt local candidate there; all local version/fence/parity/activation work remains real pinned Beads. The resulting green sequence is R0, fresh staging, exact init, canonical remote-list, R1, copied-binary version, status/export/KV/status, and opaque activation. It admits neither active-history copy nor remote add/pull/push/fetch/force/refspec/update-ref/shell.

`stable_sync_requires_complete_shadow_parity` added a raw-output mutation matrix before the parity hardening. Authority and source KV, every logical document fact, operational owner/dependency/order, missing/extra/duplicate rows, and unknown receipt/lease/writer metadata all refuse as `remote_shadow_mismatch` with state and current unchanged. The contract inventory includes representative authority, source, logical, operational, missing, extra, and unknown-KV cases. `sync_failure_activation_is_idempotent` then verified through the full orchestrator that a first changed Unknown observation atomically activates once with `state_changed=true`, while an identical retry is write-free and reports false. `sync_activation_survives_every_interruption_boundary` exercises the four private test-only activation fault boundaries: every interruption leaves the exact old complete generation current; only success exposes the complete validated new generation. These interruption checks are post-hoc boundary coverage of the existing fault adapter and do not create a production fault switch.

`sync_cleanup_failure_precedes_remote_refusal` was added before moving disposable preflight ownership into public synchronization. Its meaningful red showed remote work could be reached after a preflight cleanup error; the green private continuation now returns `temporary_cleanup_failed` before remote/stage work and preserves current. Sync operational refusals now render an empty stdout, one stable human or minimal JSON stderr line, exit 1, and the same `state_changed` boolean; syntax remains exit 2. The installed hostile-checkout and real flock checks prove the checksum-bound wrapper uses compiled configuration, emits `remote_uninitialized` for the exact compiled observation, does not invoke Cargo or rustup, returns `sync_busy` before observation while the generation lock is held, and leaves ordinary installed `list --json` lock-free.

The prior `real 178.29` online-sync run above is superseded and is not acceptance evidence: later completion remediation changed the final source and its installed-wrapper proof was intentionally discarded. A physical strict-Git diagnostic then reached a meaningful red (`exit 97`): pinned Beads 1.1.2 itself runs `git -C <repository> rev-parse --git-dir --git-common-dir` from `internal/config/config.go::gitDirsForRepo`. Test-only `installed_config_git_shim_admits_only_bound_local_beads_discovery` first failed to compile because the strict shim helpers did not exist; after the contract-only multiplexer implementation it passed in 0.45 seconds. The sole delegated local form is exact argc/order, canonical repository equal to the child cwd and the runtime-TMPDIR-derived disposable repository, exact sealed runtime environment, and an absolute pre-resolved Git executable. Wrong cwd/repository/runtime/environment/argc/order/extra arguments and alternate URL, config, remote, fetch, pull, push, or update-ref forms all exit 97 without delegation.

The installed human and JSON physical proof is consequently constrained to exactly eight bound locator records total (four `--show-toplevel` and four `--path-format=absolute --git-common-dir`: launcher plus installed wrapper for each invocation), exactly two compiled `ls-remote` observations, at least one bound Beads-local discovery per invocation, and zero unknown records. The current source has no temporary marker or stage diagnostics. Focused strict-shim coverage passed in 0.45 seconds; `cargo test -p plasmosome-work-state` passed 47 library, 1 binary, 15 CLI, 4 command, 40 contract, 17 document, 10 freshness, 9 pin, 1 project, 6 read, 15 shadow, 25 store, and 10 sync tests in 17.28 seconds. `cargo fmt --all -- --check`, `cargo clippy -p plasmosome-work-state --all-targets -- -D warnings`, and `git diff --check` were green. Final retained physical online-sync, aggregate, coverage, timing, and root-gate evidence remain pending from this exact committed head. No remote write, writer, lease, receipt, lifecycle mutation, heartbeat, hosted/server fixture, credential forwarding, or cutover is claimed.

### 2026-09-03 — TDD: installed-contract locator PATH construction

The first retained final-head online-sync attempt at `3876c1c` was a diagnostic refusal, not acceptance: it exited 1 `cutover_blocked` after `/usr/bin/time -p` `real 126.79`, `user 100.16`, `sys 12.43`, before scenario evidence. A disposable local mirror reproduced the checksum-verified bootstrap independently with the same source/ref/artifacts (exit 0, 70 documents), narrowing the issue to the installed-wrapper contract helper. Temporary internal stage-label probes, removed before the regression edit, isolated the failure to that helper's pre-launch setup; they are not product behavior or acceptance evidence.

Static review found the cause: `std::env::join_paths([fake_bin, original_path])` treated the whole colon-delimited original `PATH` as one element and rejected it. Test-only `installed_config_locator_path_preserves_a_multi_entry_path` first failed to compile because the narrowly named construction helper did not exist. The green implementation prepends the fake-bin element and chains `std::env::split_paths(original_path)`, preserving each original path entry before `join_paths`. The focused test passed; the existing strict local-Beads Git-shim test passed; full `cargo test -p plasmosome-work-state` passed 48 library, 1 binary, 15 CLI, 4 command, 40 contract, 17 document, 10 freshness, 9 pin, 1 project, 6 read, 15 shadow, 25 store, and 10 sync tests in 17.58 seconds. `cargo fmt --all -- --check`, `cargo clippy -p plasmosome-work-state --all-targets -- -D warnings`, and `git diff --check` passed. No diagnostic labels, temporary markers, relaxed binding, or remote write remain. A new retained physical online-sync run is required from the subsequent clean committed head.

### 2026-09-03 — normative private-runtime correction

The corrected all-exit-97 contract-only classifier found four exact `git -C … rev-parse --git-dir --git-common-dir` calls with sealed runtime predicates and canonical argument-to-child-cwd binding, but with the child cwd outside the private repository. This is a version-startup isolation defect, not an admission for that Git shape. The normative fence now requires every Beads version check to use the owning private root as cwd, seals `BEADS_DIR` to that root's `repository/.beads`, and sets `BD_BACKUP_ENABLED=false`; it continues to forbid synthetic actor attribution. A later strict-shim batch may admit only exact private-repository `git config user.name`, after its own red evidence; generic config, email, network, and write forms remain forbidden. No production or test implementation changed in this task-only correction.

### 2026-09-03 — TDD: private runtime version ownership

Test-only `version_checks_use_explicit_private_cwd` first failed to compile because the pinned verified and installed Beads verification APIs had no caller-bound cwd. The minimum green change requires every `bd --version` plan to carry the owning private root explicitly: bootstrap verification, installed generations, disposable read copies, fresh sync staging, and copied candidates all reject `cwd=None`, invoking-checkout, shared-repository, or outside-root forms before their inner runner dispatches. `runtime_environment_binds_private_beads_dir_and_disables_backup` first failed because the sealed runtime environment omitted both values; it is now exactly bound to `<private-root>/repository/.beads` and `BD_BACKUP_ENABLED=false`, including pre-init roots whose repository has not yet been created. `sync_binding_refuses_missing_or_altered_private_beads_environment` then first accepted a missing `BEADS_DIR`; the sync command fence now independently refuses missing or altered private Beads/backup bindings. Finally, `malformed_runtime_environment_cannot_admit_a_none_cwd_version` first reached the recording inner runner as `unexpected_command`, proving that matching failed environment derivations could otherwise admit `cwd=None`; both read and bootstrap version validators now require successful owner derivation and an exact `Some(private-root)` cwd before dispatch. Focused targets and `cargo test -p plasmosome-work-state` passed (50 library tests, 1 binary, 15 CLI, 4 command, 40 contract, 17 document, 10 freshness, 10 pin, 1 project, 6 read, 15 shadow, 25 store, and 11 sync); `cargo fmt --all -- --check` and `git diff --check` passed. No remote, actor, backup, or writer behavior is admitted by this batch.

### 2026-09-03 — TDD: sealed private Git side reads

The existing test-only `installed_config_git_shim_admits_only_bound_local_beads_discovery` was strengthened before implementation to require exactly the pinned local `rev-parse --git-dir --git-common-dir --show-toplevel` and `config user.name` forms in the sealed private repository, while refusing the prior `-C` discovery, owner/email lookup, generic config, wrong cwd/repository/runtime/environment, missing or altered `BEADS_DIR`/backup flag, malformed/reordered/extra argv, and every network or write form. Its meaningful red was the expected repo-context invocation returning exit 97 rather than 0. The green contract-only multiplexer derives the runtime root physically, validates that the lexical `BEADS_DIR` suffix resolves to that same root (preserving macOS canonical-path equivalence), requires `BD_BACKUP_ENABLED=false`, records the two exact local categories, and delegates only via the pre-resolved absolute Git executable. The installed physical record assertion now requires exactly 4 top-level locators, 4 common-dir locators, 2 compiled observations, 8 private repo-context reads, 8 `user.name` reads, no old `-C` discovery, no unexpected record, and no extra record. The focused test passed in 0.55 seconds; `cargo test -p plasmosome-work-state` passed 50 library, 1 binary, 15 CLI, 4 command, 40 contract, 17 document, 10 freshness, 10 pin, 1 project, 6 read, 15 shadow, 25 store, and 11 sync tests; fmt, clippy, and diff checks passed. No temporary diagnostics, raw path/argv output, remote write, actor injection, or generic Git admission remains. Final retained physical evidence is still pending from the committed head.

### 2026-09-03 — TDD: busy sync preflight stays local before the activation lock

The first countable installed-wrapper online-sync run from `d027799c` reached the lock-held busy check but refused at the previous locator-only post-lock assertion. Temporary sanitized diagnostics (removed before this correction) established the exact busy interval: 2 top-level locators, 2 common-dir locators, 0 observations, 4 sealed private-repository context reads, 4 sealed `user.name` reads, 0 exact-`-C` discovery, 0 unexpected forms, 12 total; its retained timing was `real 234.49`, `user 196.28`, `sys 15.15`. This is the specified local disposable preflight before lock acquisition: the losing sync may duplicate immutable checksum/version/fenced-snapshot work, but must not observe R0, stage a generation, replace `current`, or change the shared generation tree.

Test-only `busy_sync_git_records_are_exact_preflight_reads` first failed meaningfully with `Err("cutover_blocked")` where the exact 12-record trace was required to pass. Its green assertion accepts only the separately exact initial 26-record trace or this exact busy trace; it binds locator cwd to the fixture worktree, requires the local reads to share one distinct absolute private repository cwd, and refuses observation, unknown, extra, wrong-argv, or wrong-cwd variants. The physical helper now captures/asserts the busy interval before the subsequent ordinary lock-free list, verifies the complete shared state tree remains byte-for-byte unchanged, and proves that the ordinary list adds no shim records or state change. The exact `sync_busy` response with empty stdout and `state_changed=false` also proves the disposable preflight completed and cleaned up before refusing; the existing cleanup-precedence contract remains separately exercised. `cargo test -p plasmosome-work-state` passed 51 library, 1 binary, 15 CLI, 4 command, 40 contract, 17 document, 10 freshness, 10 pin, 1 project, 6 read, 15 shadow, 25 store, and 11 sync tests; clippy, fmt, and diff checks passed. No lock timing, remote behavior, staging, launcher, or production sync behavior changed; a fresh retained physical acceptance run remains required from the new committed head.

### 2026-09-03 — TDD: contract-only validated parity-digest cache

Test-only `cached_tree_snapshot_rehashes_on_identity_change_and_reuses_only_unchanged_regular_files` first failed to compile because neither the contract-private cache nor cached snapshot helper existed. Its green counter proves one digest read for an unchanged regular file across repeated snapshots, then a mandatory rehash after content replacement, atomic file replacement, and mode change; adding/removing a path changes the complete snapshot without re-reading an unchanged file. The same test confirms symlink and special-file refusal remain unchanged.

The cache is scoped solely to the seven-case representative parity inventory. Every Unix lookup still walks the complete tree and does `lstat`, descriptor open, `fstat`, and a second `lstat`; cache misses hash through that descriptor and repeat `fstat`/`lstat` before insertion. Its identity key is absolute path plus device, inode, length, mode, uid, gid, link count, and mtime/ctime seconds/nanoseconds. Thus it never trusts a manifest or omits the pinned `bd` digest; it only reuses bytes when the same validated regular file identity is unchanged. Non-Unix builds retain the uncached snapshot path. Focused cache coverage passed in 0.01 seconds; `cargo test -p plasmosome-work-state` passed 52 library, 1 binary, 15 CLI, 4 command, 40 contract, 17 document, 10 freshness, 10 pin, 1 project, 6 read, 15 shadow, 25 store, and 11 sync tests in 8.63 seconds wall time. Clippy, fmt check, and diff check passed.

The obsolete retained `online-sync` diagnostic from clean head `30ee3bb` was intentionally stopped after it remained CPU-active while hashing the 134,188,304-byte pinned `bd` repeatedly: `/usr/bin/time -p` recorded `real 1559.35`, `user 1431.48`, `sys 58.03`, exit 130, after only 3 of 7 parity directories. It is explicitly not acceptance evidence. This contract-only cache is evidence-performance remediation, not a production behavior change or threshold; final retained physical acceptance, aggregate, coverage, timing, and root gates remain required from the subsequent committed head.

### 2026-09-03 — TDD: one bound state-snapshot cache across the main online inventory

The retained `online-sync` run from clean head `0e76e63` was intentionally stopped after all recorded-remote and all 7 representative parity directories had completed. It then spent its remaining time in the formerly uncached cleanup-inventory state-tree snapshot. `/usr/bin/time -p` recorded `real 1765.09`, `user 1581.40`, `sys 76.88`, exit 130. This is a performance diagnostic only, not acceptance evidence.

Test-only `online_sync_state_snapshots_reuse_digests_across_inventory_boundaries_and_bind_one_state_root` first failed to compile because `StateTreeSnapshots` did not exist. Its green contract-private wrapper owns the existing validated digest cache and binds exactly one canonical non-symlink state root. The test proves unchanged files are not reread across snapshot boundaries, a non-`bd` mutation rehashes only that file, a replaced `bd` rehashes and changes the snapshot, and a second state root is refused. Every cached lookup retains complete tree traversal and the existing lstat/open/fstat/re-lstat and miss revalidation rules.

The main online fixture now carries one wrapper across exactly 23 state snapshots: bootstrap unchanged proof (2), installed-wrapper/lock proof (3), seven parity cases (14), cleanup-before-remote (2), and stable-success before/after (2). The independent pending and activation fixtures deliberately use the old bootstrap helper and therefore fresh wrappers. Source, worktree, Git, mirror, and hook snapshots remain uncached. Focused cache tests passed; `cargo test -p plasmosome-work-state` passed 53 library, 1 binary, 15 CLI, 4 command, 40 contract, 17 document, 10 freshness, 10 pin, 1 project, 6 read, 15 shadow, 25 store, and 11 sync tests in 19.82 seconds wall time. Clippy with warnings denied, fmt check, and diff check passed. This is contract-evidence performance work only: it adds no production sync behavior, remote action, writer, heartbeat, or acceptance threshold.
