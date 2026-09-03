---
id: 047
title: Synchronize the installed Beads shadow online
status: planned
priority: 1
specs: [014]
intents: [015]
refs: [AGENTS.md, README.md, Cargo.toml, Cargo.lock, .agents/skills/planning-work/SKILL.md, .agents/skills/tasks/SKILL.md, .agents/skills/pr-review/SKILL.md, docs/intents/015-local-first-shared-work-state.md, docs/specs/014-local-first-work-state.md, tasks/042-beads-transport-foundation.md, tasks/045-beads-shadow-import.md, tasks/046-beads-local-reads.md, tools/work-state, tools/work-state-beads-1.1.2.toml, crates/plasmosome-work-state/Cargo.toml, crates/plasmosome-work-state/AGENTS.md, crates/plasmosome-work-state/README.md, crates/plasmosome-work-state/src/lib.rs, crates/plasmosome-work-state/src/command.rs, crates/plasmosome-work-state/src/contract.rs, crates/plasmosome-work-state/src/freshness.rs, crates/plasmosome-work-state/src/main.rs, crates/plasmosome-work-state/src/pin.rs, crates/plasmosome-work-state/src/read.rs, crates/plasmosome-work-state/src/shadow.rs, crates/plasmosome-work-state/src/store.rs, crates/plasmosome-work-state/tests/cli.rs, crates/plasmosome-work-state/tests/command.rs, crates/plasmosome-work-state/tests/contract.rs, crates/plasmosome-work-state/tests/freshness.rs, crates/plasmosome-work-state/tests/read.rs, crates/plasmosome-work-state/tests/shadow.rs, crates/plasmosome-work-state/tests/store.rs]
done_when:
  - After the current wrapper is installed with the existing checksum-verified bootstrap, ./tools/work-state sync [--json] runs that installed wrapper without Cargo, artifact paths, caller ref, remote, or credential.
  - A compiled strict project binding fixes project plasmosome, remote origin, HTTPS URL https://github.com/teonimesic/plasmosome.git, and refs/dolt/data; any unbound configuration refuses before network.
  - Sync verifies installed wrapper and pinned Beads, operates only on a staged copy, observes before and after one pinned pull, and atomically activates only exact same-authority, source, logical, and operational Markdown-shadow parity.
  - Missing ref, transport loss, moving ref, pending operations, and remote mismatch never write remote or expose pulled data, and retain strongest justified freshness without erasing prior sync or pending ids.
  - Bootstrap and sync share one nonblocking activation lock; readers stay lock-free; interruption exposes an old or fully validated new generation only.
  - Sync's allowlist has no push, force, ref update, fetch, arbitrary URL, shell, or ambient credential path; Task 042 helpers remain contract scaffolding.
  - Strict test-first reds, real pinned local-store plus recorded-transport online-sync contract, coverage and branch review, timed suite, and five root gates pass; no writer, lifecycle, claim, heartbeat, reconciliation, backup, or cutover acceptance is claimed.
pr:
evidence:
---

## Why

Task 046 made the shared offline projection usable, but later writers cannot safely start until production binds the right project remote, observes it, stages a pull, validates it, and records freshness without touching active state.

This task adds one explicit installed-wrapper sync command that observes the pinned data ref, pulls only in staging, atomically activates an exact compatible Markdown shadow, and otherwise retains local data with honest freshness. Markdown remains authoritative for every imported fact.

## Plan

### Deliverable, in one sentence

Add an explicit zero-remote-write installed-wrapper synchronization command that observes the compiled project remote, pulls only into a staged Beads copy, validates exact Markdown-shadow parity, atomically activates a complete compatible generation, and reports the complete freshness envelope.

### Authority boundary and out of scope

The current store is markdown-shadow. Spec 014 permits that mode only as one-way Markdown import, and existing shadow validation requires state_version equals 1. Task 047 therefore provides production online remote observation and synchronization of the installed shadow with zero remote writes. Do not add a library-only speculative writer, successful lease, receipt, version, lifecycle command, or ledger mode.

- No remote push, force, update-ref, refspec, or creation of refs/dolt/data.
- No writer lease, fence, admission, renewal, release, expiry, takeover, typed receipt, fingerprint, expected document version, state-version increment, or ledger mode.
- No lifecycle, priority, PR, evidence, dependency, owner, claim, approval, acceptance, task, branch, dispatch, or external-effect mutation.
- No GitHub API, reconciliation, heartbeat, freeze, epoch, dual authority, cutover, backup, restore, rollback, garbage collection, hosted/live remote fixture, GitHub mock, local Git server, broad mock, daemon, container, paid service, implicit read sync, source-ref resolution, Markdown reimport, dependency, or toolchain installation. Do not run or edit heartbeat.

Task 048 owns typed project lease and receipt schemas, acquire/renew/release/takeover, non-forcing expected-base publication, request fingerprint/idempotency, retries, stale conflict, lost-response recovery, and ledger-gated work. Task 049 owns expected document version, increments, typed lifecycle receipts, and transition graph/gates. Claims, effects, and cutover remain later work. STOP when this plan passes; do not begin Task 048 or Task 049.

### Files to read and exact edit boundary

Read only this task and the files in refs. Do not explore unrelated crates, tasks, branches, or the concurrent work for Tasks 030, 043, and 044.

Create or edit only new tools/work-state-project.toml, src/project.rs, tests/project.rs, src/sync.rs, and tests/sync.rs; focused src/lib.rs, src/main.rs, src/store.rs, src/freshness.rs, and src/contract.rs; focused tests/cli.rs, tests/store.rs, tests/freshness.rs, and tests/contract.rs; tools/work-state; the work-state crate AGENTS.md and README.md; and this task for lifecycle/dated Notes. Do not edit document.rs, shadow.rs, read.rs, pin.rs, Cargo manifests/lockfile, root instructions, Spec 014, existing intent/spec/task content, or any other file. A required change outside this list is a STOP.

### Compiled project binding and public command

Track tools/work-state-project.toml exactly:

    schema_version = 1
    project_id = "plasmosome"
    remote_name = "origin"
    remote_url = "https://github.com/teonimesic/plasmosome.git"
    data_ref = "refs/dolt/data"

ProjectConfig denies unknown and duplicate fields and validates exact schema, project id, remote name, credential-free canonical HTTPS URL, and ref. Reject userinfo, query, fragment, controls, whitespace, alternate scheme, host, repository, or ref. Load it only with include_str! in the checksum-bound installed wrapper, never a mutable checkout. The wrapper hash binds it; do not bump the manifest schema. Same-source new-wrapper bootstrap reinstalls safely while preserving source, freshness, pending ids, owners, dependencies, and digests.

Add only ./tools/work-state sync [--json] and ./tools/work-state contract-test online-sync --source-ref REF --archive PATH --bd PATH; include online-sync exactly once in all. Sync accepts only one optional --json and no project/ref/URL/remote/artifact/store/token/actor/session/operation/credential/timestamp flags. Syntax refusal is exit 2 invalid_command; ordinary launcher execution uses the installed wrapper, never Cargo, rustup, build, or registry.

Exit-0 output carries command=sync, project_id=plasmosome, outcome=synchronized, authority_mode=markdown-shadow, source_commit, and the exact Task 046 freshness envelope. Human output says synchronized as of, never current. Exit-1 codes include not_initialized, invalid_project_config, sync_busy, store_changed, installed_beads_missing, beads_checksum_mismatch, unsupported_beads_version, remote_uninitialized, remote_transport, invalid_remote_observation, remote_configuration_mismatch, remote_changed, pending_mutations, remote_shadow_mismatch, and temporary_cleanup_failed. A refusal has empty stdout and one stable stderr. Cleanup/activation failure wins; a safe observation-only activation says state_changed=true, otherwise false.

### Mandatory pinned-Beads discovery gate

Before implementation, use supplied pinned Beads 1.1.2 and an isolated initialized store to capture machine-readable bd --sandbox --json dolt remote list, prove exact bd --sandbox dolt remote add origin URL, re-list, and confirm the documented pull form works in embedded mode. STOP and return to planning if v1.1.2 lacks deterministic name-and-URL output, add/pull needs server mode, or the exact configured GitHub URL cannot be represented. Do not add a permissive text parser, alternate backend, server, or broad seam.

### Sync sequence and fence

Preflight before network locates with the sealed local locator, binds canonical current_exe generation, verifies wrapper/manifest/installed binary/authority/complete fenced snapshot/compiled config, and uses active source plus exact logical/operational projection as comparison truth. Sync never reads Markdown or resolves a source ref.

Only these new staging commands are allowed:

    <copied-bd> --sandbox --json dolt remote list
    <copied-bd> --sandbox dolt remote add origin https://github.com/teonimesic/plasmosome.git
    git ls-remote --exit-code https://github.com/teonimesic/plasmosome.git refs/dolt/data
    <copied-bd> --sandbox dolt pull --remote origin

Existing copied-runtime version and readonly fenced snapshot commands remain allowed for the staged repository. Redact URL argv. A sync validator independently binds compiled config, executing generation, staging root, copied binary, cwd, runtime dirs, full environment, and phase before the system runner. Its cleared environment has only PATH, staged HOME/XDG/TMP/global-Git, current telemetry/prompt suppression, and Git safety flags; no token/helper/askpass/SSH-agent/proxy/user HOME/global Git/Beads/Dolt credentials. Refuse before dispatch push, fetch, force, refspec, update-ref, shell, wrong binary/root/cwd/URL/ref/remote/environment, shared active cwd, mutable config load, replay, and out-of-order commands.

Use physical bootstrap.lock, generalized as generation-activation lock, so bootstrap and sync contend on the same inode. Bootstrap returns bootstrap_busy; sync returns sync_busy; reads remain lock-free. Under the lock, reread current, stage a same-filesystem copied generation with existing regular/no-symlink/mode/fsync rules and fresh runtime, list/add/re-list only compiled origin, observe R0, skip pull for pending ids, otherwise pull once, observe R1 and require R1 equals R0, then run status/export/KV/status parity. Complete staged manifest, recursively fsync, rename, and atomically replace current. TempDir cleanup wins; abandoned staging remains retained.

The candidate must be markdown-shadow, source-equal, and exact in document key/kind/id/path/title/content SHA/state version/ordered links/lifecycle/priority/PR/evidence, task owner and dependency, logical and operational digest. No extra/missing row, unknown Plasmosome KV, receipt, lease, or writer metadata is allowed. Internal Dolt generation may differ only with exact projection parity. Any difference is remote_shadow_mismatch, without reimport, activation, or remote overwrite. One status/export/KV/status fence stays unchanged after observation.

Remote observation accepts exactly one lowercase 40-hex SHA paired with refs/dolt/data. Successful malformed/uppercase/duplicate/extra/wrong-ref/empty output is terminal invalid_remote_observation; empty no-match from ls-remote --exit-code is remote_uninitialized; other nonzero is remote_transport; never parse unconstrained stderr.

### Freshness and atomicity

No successful observation is Unknown while preserving prior complete observation, last_successful_sync_at, local generation, and pending ids without inventing time. R0/pull failure records complete Unknown R0/time. R0 different from R1 records R1/time Unknown, returns remote_changed, and exposes no candidate. Stable pull/parity failure records stable remote as Unknown plus remote_shadow_mismatch. Pending at the same synchronized base may be Unpublished; pending at changed/unknown base is UnknownWithUnpublished, with ids preserved in order. Stable valid pull samples post-R1 observation and successful-sync times as SynchronizedAsOf. Equivalent reobservation may postdate a successful sync, canonical UTC never moves backward, the six classifiers remain, and an identical Unknown retry is write-free.

Use narrow crate-private store helpers, not a general manifest writer or duplicated filesystem logic. The lock spans current revalidation, staging, remote work, failure-state preparation, and activation; a selected reader remains on its old generation.

### Strict test-first matrix

| Test | Proves |
| --- | --- |
| project_config_accepts_only_the_compiled_plasmosome_remote | Exact config only; malformed/alternate forms refuse. |
| compiled_project_config_is_bound_by_the_installed_wrapper | Wrapper reinstall preserves facts; no checkout config. |
| sync_cli_accepts_only_optional_json | Every forbidden option refuses. |
| sync_launcher_executes_only_the_installed_wrapper | No Cargo/rustup/build route; old routes remain. |
| sync_refuses_when_the_selected_generation_is_no_longer_current | Pointer flip creates no network/staging mutation. |
| bootstrap_and_sync_contend_on_one_generation_lock | Same lock gives distinct deterministic busy codes. |
| sync_runner_binds_every_command_before_dispatch | Wrong command/root/env/order/replay reaches no runner. |
| sync_runner_rejects_every_remote_write_shape | push/fetch/force/refspec/update-ref/shell pre-dispatch refuse. |
| remote_observation_is_one_exact_lowercase_data_ref | Strict parser and terminal malformed state. |
| remote_no_match_and_transport_are_distinct | Output/status distinguish no-match and transport. |
| failed_sync_records_unknown_without_erasing_history | Failure preserves history/pending and invents no time. |
| equivalent_reobservation_may_postdate_successful_sync | Valid pending reobserve does not regress. |
| pending_mutations_are_observed_but_never_pulled | Pending ids yield honest state and no pull. |
| moving_remote_never_activates_the_pulled_candidate | R0 different from R1 exposes no candidate. |
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

The contract uses real source object database and caller-pinned artifacts, a real temporary shared shadow/current wrapper, real remote-list/add/list, fenced snapshots/parity/staging/activation/linked-worktree/cleanup/lock, and RecordingCommandRunner only for remote observation/pull outcomes. Cover stable equality, identical-projection ahead candidate, missing ref, transport before/after R0, R0/R1 moving, pending, all parity mismatch classes, unknown KV/extra row, and all activation interruptions. Prove zero push/force/update/fetch/credential/effect/lifecycle/Markdown import, and label real-local versus recorded-remote evidence separately.

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

STOP if base/Spec014/Task046 moves incompatibly, an edit leaves this allowlist, a dependency or tool install is needed, active state would mutate, exact parity cannot be established, remote transport needs credential/server/mock, failure freshness cannot be represented atomically, an existing read gains network behavior, a test only turns green by weakening, or implementation approaches Task048/049.

## Notes
