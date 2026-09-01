---
id: 042
title: Prove the pinned Beads transport fence
status: in_progress
priority: 1
specs: [014]
intents: [015]
refs:
  [
    AGENTS.md,
    Cargo.toml,
    .gitignore,
    crates/plasmosome-guards/tests/workspace_guards.rs,
    docs/intents/015-local-first-shared-work-state.md,
    docs/specs/014-local-first-work-state.md,
    .agents/skills/tasks/SKILL.md,
    .agents/skills/pr-review/SKILL.md,
    docs/templates/task.md,
  ]
done_when:
  - The repository records the Beads 1.1.2 release source, upstream archive checksums and verified extracted-binary checksums for Apple Silicon macOS and x86_64 Linux, and rejects every other version, checksum or platform before opening a store.
  - `./tools/work-state contract-test hermetic` verifies the supplied pinned artifact and initializes only disposable state without hooks, agent-file edits, staged files, telemetry, automatic pushes or writes to the user's global configuration.
  - With an explicitly supplied disposable public GitHub fixture, the GitHub contract cases use two independent clones to prove that `refs/dolt/data` rejects a stale Beads 1.1.2 writer, preserves the winner, recovers a divergent contender by pull/replay/push, and treats failures before and after publication as retryable without a second accepted generation.
  - A missing, non-public, non-disposable or already-populated GitHub fixture fails clearly before a remote write; the runner never creates or deletes a repository and compare-and-set-deletes only the temporary ref generation it created.
  - An unsafe Beads or GitHub result exits non-zero as `cutover_blocked` with the observed generations and commands, rather than weakening the assertion or beginning migration.
  - Coverage is collected and its meaningful misses are reviewed; the timed full root gate exits 0; and no migration, cutover or operational work-state claim occurs.
pr:
evidence:
---

## Why

Spec 014 makes remote compare-and-set behavior the fence that prevents two sessions from publishing
the same coordination action. Beads 1.1.2 must prove that behavior against GitHub before any current
Markdown state is imported or any authority changes.

This task establishes the pinned tool and a disposable contract runner. It may conclude that
cutover is blocked; that is a successful finding when the evidence shows the transport is unsafe.

## Plan

### Deliverable, in one sentence

Add a repository-pinned Beads 1.1.2 verifier and `./tools/work-state contract-test` foundation that
runs hermetic initialization checks locally and, only when a disposable GitHub repository is
injected, proves or blocks expected-base publication and stale-writer recovery on
`refs/dolt/data`.

### Out of scope

- No import of intents, specs or tasks; no shadow database; no parity comparison.
- No project store in this checkout, no `.beads/`, no `.plasmosome/`, and no change to the current
  Markdown authority or lifecycle fields.
- No writer lease, task ownership lease, lifecycle transition, operation receipt, freshness model,
  heartbeat change, branch creation, dispatch or other external coordination effect.
- No cutover, backup, rollback, production claim or mutation of Plasmosome's own
  `refs/dolt/data`.
- No raw `bd` workflow for agents. The only `bd` writes in this task are fixture records inside
  temporary directories used by the contract runner.
- No Homebrew, install script, `cargo install`, `rustup component add`, global path edit or other
  user/system installation. The runner consumes archive and binary paths supplied by the caller.
- No creation, deletion or settings change of a GitHub repository. A fixture repository is injected
  configuration owned outside this task.
- No claim that the full `mutation-retries`, `interrupted-mutation`, `claim-race` or other spec 014
  acceptance cases are complete. This task names its narrower probe `transport-retries`.
- No edit outside the files named below. In particular, do not edit skills, templates, guards,
  existing crates, intents, specs or existing tasks.

### Files to read, and nothing else

Read only this task, root `AGENTS.md`, root `Cargo.toml`, root `.gitignore`,
`crates/plasmosome-guards/tests/workspace_guards.rs`,
`docs/intents/015-local-first-shared-work-state.md`,
`docs/specs/014-local-first-work-state.md`, `.agents/skills/tasks/SKILL.md`,
`.agents/skills/pr-review/SKILL.md` and `docs/templates/task.md`. Do not explore beyond them. The
upstream facts and commands needed for Beads are recorded in this plan; do not clone or read the
upstream source tree during execution.

Create or edit exactly these files:

- `Cargo.toml` and generated `Cargo.lock`;
- `tools/work-state` and `tools/work-state-beads-1.1.2.toml`;
- `crates/plasmosome-work-state/Cargo.toml`, `AGENTS.md`, `CLAUDE.md`, `README.md`;
- `crates/plasmosome-work-state/src/{lib,command,contract,pin,main}.rs`;
- `crates/plasmosome-work-state/tests/{pin,contract,cli}.rs`; and
- this task for its lifecycle fields and dated `## Notes` only.

`CLAUDE.md` contains only `@AGENTS.md`. The new package is a workspace member named
`plasmosome-work-state`, has a library plus a same-named binary, and says `publish = false` so the
existing workspace guard remains true. Add workspace `sha2 = "0.10"`; reuse workspace `serde`,
`toml`, `serde_json` and `tempfile`. Add no HTTP, GitHub, Git or archive library: the runner invokes
the caller-supplied `bd` plus system `git`, and it never downloads or extracts an artifact.

### The pin manifest

`tools/work-state-beads-1.1.2.toml` is the only production pin input. Record these exact values:

```text
version             1.1.2
release             https://github.com/gastownhall/beads/releases/tag/v1.1.2
source_commit       20e493e569c922d1253bdeff068c5e56c94957fb
license             MIT
checksums_url       https://github.com/gastownhall/beads/releases/download/v1.1.2/checksums.txt
checksums_sha256    8ea26179417c8a206b8d18c515b9a7588c1dad5336f6ce1e61b329c2ed7138a5

aarch64-apple-darwin archive beads_1.1.2_darwin_arm64.tar.gz
archive_sha256      9b0137a83a2afd343e2abd2a506be72ea032721000f76669c2cf81729e78501d
binary_sha256       621b7b6c20c38db27ef4120398eb46dc35ba5b3e6c3611e19e14d33de10ce351

x86_64-unknown-linux-gnu archive beads_1.1.2_linux_amd64.tar.gz
archive_sha256      a72d71ed374955dc9f83a0f90b54bd7b6a0016709dd1676ae2e368651ed401c2
binary_sha256       6d767629e90560506d0ea3de9823aef48386414f5425d8853e2ae3312cad9a82
```

The archive hashes are copied from the upstream `checksums.txt`; the binary hashes are SHA-256 of
the `bd` file extracted from the corresponding verified archive. Supporting a new target is later
work that adds both hashes and a real-artifact run on that target. Do not silently select a nearby
architecture.

`pin::PinManifest::load(path)` parses with unknown fields denied and refuses duplicate targets,
non-HTTPS source URLs, a version other than `1.1.2`, non-64-hex hashes and filenames not belonging
to v1.1.2. `pin::VerifiedBeads::verify(manifest, target, archive, binary, runner)` performs, in
order: supported-target lookup, archive filename and SHA-256, binary SHA-256, then
`bd --version`. Only the exact one-line form `bd version 1.1.2 (<build text>)` passes. No store or
Git command may run before verification succeeds. Errors have stable codes
`unsupported_beads_version`, `beads_checksum_mismatch`, `unsupported_beads_platform` and
`invalid_beads_pin`; paths and credentials are never included in structured output.

### Process and disposal seam

`command.rs` defines a small accepted dependency, not a global singleton:

```text
CommandRunner::run(CommandSpec) -> CommandOutput
SystemCommandRunner
RecordingCommandRunner (tests only)
CommandSpec = program, argv, cwd, explicit environment, redacted argv positions
```

`SystemCommandRunner` uses `std::process::Command`; tests inject the recording implementation to
prove ordering, error classification and that no command follows a refusal. Keep command building
separate from execution so wrong-version, stale-base and transport failures are unit-testable
without GitHub.

Every real contract case creates its own temporary root and puts `HOME`, `XDG_CONFIG_HOME`,
`XDG_CACHE_HOME`, `XDG_DATA_HOME`, `TMPDIR`, `GIT_CONFIG_GLOBAL` and all Beads state beneath it.
Set `GIT_CONFIG_NOSYSTEM=1`, `BD_DISABLE_METRICS=1`, `BD_DISABLE_EVENT_FLUSH=1`,
`BD_NON_INTERACTIVE=1`, `CI=true` and `GIT_TERMINAL_PROMPT=0` on every `bd`/`git` child. Preserve
only the explicitly named credential/SSH environment needed by the injected GitHub remote and
never print it. Do not inherit a user `HOME`, Git config, Beads config or telemetry endpoint.

The cleanup guard always attempts `bd dolt stop` for each initialized clone before removing its
temporary root. Cleanup failure makes the case fail and names the clone label, never a secret path.

### Command surface

`tools/work-state` is an executable Bash launcher using `set -euo pipefail`; it resolves the
repository root and executes the workspace binary through Cargo. It offers only this surface in
this task:

```text
./tools/work-state contract-test version-pin --archive PATH --bd PATH
./tools/work-state contract-test stealth-init --archive PATH --bd PATH
./tools/work-state contract-test stale-base-fence --archive PATH --bd PATH \
  --github-remote URL --confirm-disposable refs/dolt/data
./tools/work-state contract-test push-conflict-recovery --archive PATH --bd PATH \
  --github-remote URL --confirm-disposable refs/dolt/data
./tools/work-state contract-test transport-retries --archive PATH --bd PATH \
  --github-remote URL --confirm-disposable refs/dolt/data
./tools/work-state contract-test hermetic --archive PATH --bd PATH
./tools/work-state contract-test github --archive PATH --bd PATH \
  --github-remote URL --confirm-disposable refs/dolt/data
./tools/work-state contract-test all --archive PATH --bd PATH \
  --github-remote URL --confirm-disposable refs/dolt/data
```

`hermetic` runs `version-pin` then `stealth-init`. `github` runs the three remote probes, resetting
only the temporary ref between them. `all` runs both aggregates. Structured JSON Lines go to
stdout and human diagnostics to stderr. Each line includes case, outcome, Beads version, clone
labels, observed base, final generation and probe operation ids where applicable. Exit 0 means all
requested assertions passed. Bad input or a missing fixture exits 2 with a stable refusal code.
A disproved transport invariant exits 1 with `cutover_blocked`. A requested case is never skipped.

### Hermetic cases

`version-pin` verifies the caller-supplied archive and already-extracted binary without downloading,
extracting or installing either. The task executor may download the named release archive into a
`mktemp -d` directory and extract it there, but neither the runner nor tests do that implicitly.

`stealth-init` makes a temporary Git repository containing sentinel `AGENTS.md`, `CLAUDE.md`, a
sentinel hook and one tracked file. Snapshot tracked contents, index, hooks, local config and the
isolated global config. Run the verified binary as:

```text
bd --sandbox init --stealth --skip-agents --skip-hooks --non-interactive
```

Then set and read back `dolt.auto-push=false`. Assert the sentinels, tracked status, hooks and
global config are unchanged; no file is staged; no metrics queue exists; no daemon, hook or
background push is configured; and `git ls-remote` was never invoked. A `.beads` directory inside
the disposable repository and `.git/info/exclude` changes are allowed. Stop the embedded Dolt
process before cleanup. This proves a safe initialization command only; selecting the eventual
clone-shared production store is later work.

### Injected GitHub fixture

Remote cases have no default remote. Without `--github-remote` and the exact confirmation token,
they fail before any write with `github_fixture_required`; `all` does the same after completing and
reporting its hermetic cases. This is a failure, not a skip.

The supplied URL is configuration for this run only and is never persisted in the checkout. Before
a write, require all of the following:

- an HTTPS or SSH GitHub URL without embedded credentials;
- a repository basename beginning `plasmosome-work-state-fixture`;
- anonymous HTTPS `git ls-remote` succeeds with credential helpers and prompts disabled, proving
  the repository is public;
- authenticated read/write access succeeds through the caller's existing injected credential or
  SSH agent; and
- `refs/dolt/data` is absent.

If any check fails, return `github_fixture_invalid` or `github_fixture_not_empty` before `bd init`.
Never create, delete, rename or change settings on the repository. Never use the Plasmosome source
repository as a fixture.

Each remote case uses two separate ordinary Git clones, `clone-a` and `clone-b`, beneath the
temporary root; no worktree and no shared local bare repository stands in for GitHub. Beads remote
configuration points at the injected GitHub URL and uses the exact `refs/dolt/data` that Beads
1.1.2 publishes. Before every publication, record `git ls-remote URL refs/dolt/data`. After the
case, delete only the generation this run created with a force-with-lease deletion naming that
exact final SHA. If another writer advanced it, return `fixture_cleanup_conflict` and leave the ref
untouched. Confirm absence after successful cleanup. The repository itself remains.

### GitHub case algorithms

`stale-base-fence`:

1. Clone A initializes in stealth/sandbox mode, creates a fixture issue carrying operation id
   `task-042-base`, commits and pushes generation G0. Clone B is a fresh ordinary clone followed by
   `bd --sandbox bootstrap --non-interactive`, and both report G0.
2. A and B create distinct fixture issues and commit candidates descended from G0. A pushes and the
   remote becomes G1.
3. B pushes without pulling. It must exit non-zero, remote must remain G1, and a fresh observation
   through A after pull must contain A's operation and not B's.
4. A prepares another candidate at G1. B pulls, replays its semantic fixture operation, commits and
   pushes G2. A's paused G1 candidate then pushes and must be rejected while the remote stays G2.
5. A pulls G2 and the logical export contains the base, winner and replay operation ids exactly
   once. Record every SHA and classify any accepted stale publication, forced overwrite or missing
   history as `cutover_blocked`.

`push-conflict-recovery` repeats the two-clone divergence with one operation per clone, proves the
loser cannot push from G0, then uses guarded pull, semantic replay, commit and push. Export after a
fresh pull must contain both ids exactly once and Dolt history must retain G0, the winning commit
and the recovery commit. Do not accept a history count alone; compare operation ids and issue
contents before and after.

`transport-retries` has two subcases and does not claim spec 014's operation-receipt behavior:

- Before publication, route one push through a `CommandRunner` fault that returns a transport
  error without invoking `bd`. Verify the remote generation is unchanged; remove the fault and
  retry the same prepared candidate, which publishes once.
- After publication, let the real push complete and then inject a lost-response error before the
  harness records success. Re-observe the remote, retry the same `bd dolt push`, and require an
  idempotent success with the same remote SHA and one logical operation id.

Transport unavailability is distinct from a stale-base refusal. Retrying the stale candidate from
`stale-base-fence` must remain refused; the runner must never turn that terminal conflict into the
transport-retry path.

### TDD test table

For each group, edit tests only, run the narrow command and capture the stated red observation.
Then edit implementation only and rerun to green. A compile failure from a missing new crate/API is
an acceptable first red; once the API exists, later reds must reach the named assertion. Never add
or relax an assertion while making its implementation pass.

| Test | Initial failing observation | What passing proves |
| --- | --- | --- |
| `production_manifest_names_the_v1_1_2_release_and_supported_artifacts` (pin) | manifest missing or `invalid_beads_pin` | Exact release, source commit, MIT license, checksums asset and two supported targets are recorded |
| `verified_release_binary_is_accepted` (pin) | verifier stub refuses | Matching target, archive hash, binary hash and exact version pass |
| `lower_higher_and_unparsable_versions_are_refused` (pin) | one fake version passes or wrong code | 1.1.1, 1.1.3 and malformed output each return `unsupported_beads_version` |
| `wrong_archive_and_wrong_binary_checksums_are_refused` (pin) | mutated bytes pass | Both corruption points return `beads_checksum_mismatch` |
| `a_binary_claiming_1_1_2_with_other_bytes_is_refused` (pin) | fake executable passes | Version text cannot replace binary provenance |
| `a_missing_or_duplicate_platform_is_refused` (pin) | lookup selects another target | No architecture fallback and no ambiguous target |
| `checksum_refusal_runs_no_program_or_store_command` (pin) | recording runner sees a child | Checksum failure precedes `bd --version`, Git and store access |
| `unknown_manifest_fields_and_non_https_sources_are_refused` (pin) | loose TOML parse succeeds | Pin input cannot silently drift |
| `command_output_redacts_credentials_and_paths` (command) | diagnostic includes sentinel secret/path | Structured evidence is reproducible without leaking credentials |
| `every_bd_child_has_the_isolated_environment` (command) | one required variable absent | HOME/config/metrics/event/prompt isolation is applied centrally |
| `hermetic_aggregate_never_asks_for_a_github_remote` (cli) | parser requires remote | Pin and stealth cases run without fixture configuration |
| `github_and_all_refuse_a_missing_fixture` (cli) | success or skipped case | Missing remote is `github_fixture_required`, never a skip |
| `a_fixture_with_credentials_wrong_name_or_existing_ref_is_refused_before_write` (contract) | recording runner sees init/push | Remote safety preflight happens before mutation |
| `stealth_init_preserves_sentinels_hooks_index_and_global_config` (contract) | fixture snapshot differs | The exact init command has no repository/global integration side effects |
| `stealth_init_disables_metrics_events_and_auto_push` (contract) | event queue/config exists | No implicit network or background publication is enabled |
| `stale_contender_is_classified_separately_from_transport_failure` (contract) | both map to one error | Callers may retry transport but must refresh after stale base |
| `stale_base_fence_uses_two_independent_clone_paths` (contract) | fake plan shares a path | The GitHub proof cannot collapse into a shared-filesystem simulation |
| `the_first_stale_push_preserves_the_winners_generation` (GitHub case) | initial runner reports `not_run`; later an accepted stale push is red | A contender from G0 cannot replace G1 |
| `a_paused_former_holder_cannot_publish_after_recovery_advances_the_ref` (GitHub case) | initial `not_run`; later stale A changes G2 | A prepared G1 candidate cannot replace G2 |
| `guarded_pull_replay_push_preserves_both_operations_once` (GitHub case) | initial `not_run`; later export/history mismatch | Divergence recovery loses and duplicates no operation |
| `failure_before_publication_retries_the_same_candidate_once` (contract/GitHub) | retry creates zero or two generations | Transport failure does not consume the prepared attempt |
| `lost_response_is_recovered_without_a_second_generation` (contract/GitHub) | retry advances the ref | A published result can be rediscovered idempotently |
| `cleanup_deletes_only_the_generation_this_run_owns` (contract) | cleanup issues unconditional delete | Ref cleanup is compare-and-set and leaves a foreign advance untouched |
| `unsafe_remote_behavior_is_cutover_blocked_with_evidence` (cli) | assertion error lacks stable result | A failed proof is a durable blocker, not a weakened test |

The last six rows have deterministic command-planning/classification tests under `cargo test`; their
real GitHub observations are also exercised through the named contract commands. Do not mark the
task complete from mocks alone.

### Execution order and evidence

1. Commit this planned task only. When execution begins in a new worktree, flip it to
   `in_progress` before other changes.
2. Add the first test group and observe red. Add only enough pin implementation to make it green.
3. Repeat red then green separately for command isolation, hermetic initialization, fixture
   preflight, GitHub algorithms and CLI aggregation. Do not edit tests and implementation in the
   same working step or commit.
4. Run the real pinned artifact through `contract-test hermetic`. It must need no GitHub fixture.
5. If the injected disposable GitHub fixture is absent, run `contract-test github` once to capture
   the required `github_fixture_required`, then STOP and report the prerequisite; do not substitute
   a local bare repository or the Plasmosome source repository. If supplied, run all three real
   GitHub cases and `all`.
6. If any empirical case returns `cutover_blocked`, preserve its generations, command outcomes and
   operation ids in `## Notes`; do not change the assertion and do not start migration.

### Coverage, refactor check and definition of done

After all available real cases and the unit suite are green, use the already-installed
`cargo-llvm-cov` without installing or updating anything:

```shell
cargo llvm-cov --version
cargo llvm-cov -p plasmosome-work-state --all-targets --lcov \
  --output-path target/task-042-lcov.info
cargo llvm-cov report -p plasmosome-work-state --summary-only
```

Record the summary in `## Notes`. Inspect uncovered branches in `pin.rs`, `command.rs` and the
fixture safety/cleanup paths in `contract.rs`; add tests for meaningful version, checksum,
classification and cleanup misses. If `cargo-llvm-cov` is unavailable, do not install it globally:
STOP and report that prerequisite. The real external subprocess bodies do not need an arbitrary
percentage target, but their decision and refusal branches do.

Then perform one refactor pass. Check repeated command construction, duplicated fixture cleanup,
oversized test setup and error strings that have become part of the CLI contract. Refactor tests
and implementation in separate steps, rerunning the narrow suite after each. Keep the accepted
`CommandRunner` seam because it has real and recording implementations; add no second abstraction
without two concrete callers.

Run the entire test suite with timing, note its wall time and whether the explicit external cases
remain outside the ordinary suite so a missing credential cannot slow or flake it:

```shell
/usr/bin/time -p cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
./.githooks/provenance-guard
./.githooks/attribution-guard
```

All five root gate commands must exit 0. Report bare exit codes and the test-suite duration. The PR
description leads with the reason from `## Why`, ends with `task: 042`, and does not claim migration
or cutover. STOP when done — do not start the next piece of work.

## Notes

2026-09-01: TDD evidence: the first pin test command failed as intended because
`plasmosome-work-state` was not yet a workspace package. After the package and verifier were
implemented, `cargo test -p plasmosome-work-state --test pin` passed 5 tests. The command,
contract and CLI groups first failed on absent contract APIs, then passed 8 tests; the stealth-init
planning test first failed on the absent command builder, then passed after implementation. The
real v1.1.2 Apple Silicon archive was downloaded and extracted only under `mktemp -d`;
`./tools/work-state contract-test hermetic` passed after the runner created its isolated HOME/XDG
directories and supplied PATH to the isolated child environment.

2026-09-01: No injected disposable public GitHub fixture was supplied. The required real command
`./tools/work-state contract-test github --archive <verified-temporary-archive> --bd
<verified-temporary-bd>` returned JSON code `github_fixture_required` and exit 2 before any remote
write. GitHub transport cases, coverage, the full root gate, PR and cutover conclusion therefore
remain unrun. Required input: an explicit public, empty, disposable GitHub repository URL whose
basename starts `plasmosome-work-state-fixture`, with existing caller-provided read/write access,
and the exact confirmation token `refs/dolt/data`.
