---
id: 042
title: Prove the pinned Beads transport fence
status: in_review
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
    tools/work-state,
    tools/work-state-beads-1.1.2.toml,
    crates/plasmosome-work-state/Cargo.toml,
    crates/plasmosome-work-state/AGENTS.md,
    crates/plasmosome-work-state/CLAUDE.md,
    crates/plasmosome-work-state/README.md,
    crates/plasmosome-work-state/src/lib.rs,
    crates/plasmosome-work-state/src/command.rs,
    crates/plasmosome-work-state/src/contract.rs,
    crates/plasmosome-work-state/src/pin.rs,
    crates/plasmosome-work-state/src/main.rs,
    crates/plasmosome-work-state/tests/pin.rs,
    crates/plasmosome-work-state/tests/contract.rs,
    crates/plasmosome-work-state/tests/cli.rs,
  ]
done_when:
  - The repository records the Beads 1.1.2 release source, upstream archive checksums and verified extracted-binary checksums for Apple Silicon macOS and x86_64 Linux, and rejects every other version, checksum or platform before opening a store.
  - `./tools/work-state contract-test hermetic` verifies the supplied pinned artifact and initializes only disposable state without hooks, agent-file edits, staged files, telemetry, automatic pushes or writes to the user's global configuration.
  - `./tools/work-state contract-test transport` uses the real pinned binary to initialize two independent temporary clone-local stores, then the existing `RecordingCommandRunner` scripts the exact expected-base, winning push, stale non-fast-forward, retry-before-publication and lost-response observations required by the documented GitHub/Git contract.
  - Production command plans are explicit and redacted: ordinary publication is non-forcing, any exceptional compare-and-set ref update uses `--force-with-lease` with the exact expected SHA, and neither a hosted repository, credential, server nor GitHub API mock is required.
  - Retry publishes or rediscovers one generation, stale-base refusal never enters the transport-retry path, and every initialized Beads child is stopped and waited before isolated temporary state is removed.
  - An unleased force, missing expected base or scripted result contradicting the documented non-fast-forward contract exits non-zero as `cutover_blocked`; absence of a live hosted test does not.
  - Coverage is collected and its meaningful misses are reviewed; the timed full root gate exits 0; and no migration, cutover or operational work-state claim occurs.
pr: https://github.com/teonimesic/plasmosome/pull/75
evidence:
---

## Why

Spec 014 makes remote compare-and-set behavior the fence that prevents two sessions from publishing
the same coordination action. The wrapper around Beads 1.1.2 must prove that its explicit commands
and result handling preserve standard Git receive-pack non-fast-forward behavior before any current
Markdown state is imported or any authority changes.

This task establishes the pinned tool and a disposable contract runner. It may conclude that
cutover is blocked; that is a successful finding when the evidence shows the transport is unsafe.

## Plan

### Deliverable, in one sentence

Finish the repository-pinned Beads 1.1.2 verifier and `./tools/work-state contract-test` foundation
so two real temporary stores prove hermetic initialization while the existing recording seam proves
the production adapter's command safety, conflict classification and idempotent retry decisions
against GitHub's documented non-fast-forward contract.

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
- No hosted repository, GitHub credential, GitHub API call or GitHub behavior emulator. GitHub's
  documented non-fast-forward rejection is the platform contract because Beads sync uses Git
  transport rather than GitHub REST.
- No WireMock, Moctokit, fakehub, Forgejo, Gitea, container, Node runtime, new mock dependency or
  dedicated remote repository. Do not start a Git daemon, HTTP bridge or any other local server.
- No claim that the full `mutation-retries`, `interrupted-mutation`, `claim-race` or other spec 014
  acceptance cases are complete. This task names its narrower probe `transport-retries`.
- No edit outside the files named below. In particular, do not edit skills, templates, guards,
  existing crates, intents, specs or existing tasks.

### Files to read, and nothing else

Read only this task, root `AGENTS.md`, root `Cargo.toml`, root `.gitignore`,
`crates/plasmosome-guards/tests/workspace_guards.rs`,
`docs/intents/015-local-first-shared-work-state.md`,
`docs/specs/014-local-first-work-state.md`, `.agents/skills/tasks/SKILL.md`,
`.agents/skills/pr-review/SKILL.md`, `docs/templates/task.md`, and every existing
`tools/work-state*` and `crates/plasmosome-work-state` file enumerated in `refs:`. Do not explore
beyond them. The upstream facts and commands needed for Beads are recorded in this plan; do not
clone or read the upstream source tree during execution.

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
`toml`, `serde_json` and `tempfile`. Add no HTTP, GitHub, Git, process-mocking or archive library:
the runner invokes the caller-supplied `bd` plus system `git`, and it never downloads or extracts
an artifact. Add no server implementation or runtime dependency.

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
RecordingCommandRunner (tests and contract self-tests only)
CommandSpec = program, argv, cwd, explicit environment, redacted argv positions
```

`SystemCommandRunner` uses `std::process::Command`; tests inject the recording implementation to
prove command construction, ordering, error classification and that no command follows a terminal
refusal. Do not add another runner or a Git/GitHub mock. Keep command building separate from
execution so the production `git`/Beads argv, expected-base observations, non-fast-forward result,
transport failure and lost-response ordering are exact and deterministic.
Give `RecordingCommandRunner` one minimal ordered-script constructor accepting queued
`Result<CommandOutput, String>` values. It records every `CommandSpec` and the contract refuses an
unexpected command or an unconsumed result, so a missing observation and a hidden retry both fail.
Do not add matching rules, protocol behavior or permissive defaults.

Every real contract case creates its own temporary root and puts `HOME`, `XDG_CONFIG_HOME`,
`XDG_CACHE_HOME`, `XDG_DATA_HOME`, `TMPDIR`, `GIT_CONFIG_GLOBAL` and all Beads state beneath it.
Set `GIT_CONFIG_NOSYSTEM=1`, `BD_DISABLE_METRICS=1`, `BD_DISABLE_EVENT_FLUSH=1`,
`BD_NON_INTERACTIVE=1`, `CI=true` and `GIT_TERMINAL_PROMPT=0` on every `bd`/`git` child. Preserve
only `PATH` from the caller. Do not inherit a user `HOME`, Git config, credential helper, Beads
config or telemetry endpoint.

Embedded mode starts no Dolt server process, so its cleanup closes or drops the store handles and
removes the temporary root without planning or invoking `bd dolt stop`. The harness waits and reaps
every child process it actually started; `bd dolt stop` is reserved for a server-mode process the
harness started, which is out of scope here. A contract test retains the root path outside the
guard's scope and proves it is absent and no child remains unwaited. An unexpected live child or
failed removal makes the case fail as `fixture_cleanup_failed` and names only the clone label or
process role, never a machine-specific path.

### Command surface

`tools/work-state` is an executable Bash launcher using `set -euo pipefail`; it resolves the
repository root and executes the workspace binary through Cargo. It offers only this surface in
this task:

```text
./tools/work-state contract-test version-pin --archive PATH --bd PATH
./tools/work-state contract-test stealth-init --archive PATH --bd PATH
./tools/work-state contract-test stale-base-fence --archive PATH --bd PATH
./tools/work-state contract-test push-conflict-recovery --archive PATH --bd PATH
./tools/work-state contract-test transport-retries --archive PATH --bd PATH
./tools/work-state contract-test hermetic --archive PATH --bd PATH
./tools/work-state contract-test transport --archive PATH --bd PATH
./tools/work-state contract-test all --archive PATH --bd PATH
```

`hermetic` runs `version-pin` then `stealth-init`. `transport` initializes two independent
temporary stores and runs the three scripted command-contract probes. `all` runs both aggregates.
Structured JSON Lines go to stdout and human diagnostics to stderr. Each line includes case,
outcome, Beads version, clone
labels, redacted command plans, observed base, final generation and probe operation ids where
applicable. Exit 0 means all requested assertions passed. Bad input exits 2 with a stable refusal
code. An unsafe plan or scripted result contradicting the documented contract exits 1 with
`cutover_blocked`. A requested case is never skipped, and no case accepts a remote URL, credential
or disposal confirmation from the caller.

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
the disposable repository and `.git/info/exclude` changes are allowed. Embedded mode has no Dolt
server process to stop; cleanup drops the store handles before removing the temporary root. This
proves a safe initialization command only; selecting the eventual
clone-shared production store is later work.

### Production command contract

Create two independent temporary Git repositories and initialize each with the real pinned binary
using the stealth command above. Their `.beads`/Dolt roots and HOME/XDG/TMP/global-config roots must
be distinct. Embedded mode has no server process; drop both store handles and reap only any child
the harness actually started after the test. Do not configure a live remote;
the transport cases inject `RecordingCommandRunner` at the already-existing process boundary and
feed it an exact ordered script of command outputs.

The production adapter constructs these observable command shapes with redacted remote/path
positions:

```text
observe:  git ls-remote --exit-code origin refs/dolt/data
publish:  bd --sandbox dolt push --remote origin
refresh:  bd --sandbox dolt pull --remote origin
leased exceptional ref update only:
          git push origin --force-with-lease=refs/dolt/data:<expected-sha>
          <candidate-sha>:refs/dolt/data
```

Ordinary publication never includes `--force`. Reject a `bd dolt push --force`, a Git push with
bare `--force`, and a force-with-lease missing the exact 40-hex generation just observed for that
operation before executing any command. A normal successful push must be followed by a fresh
`ls-remote`; a success message alone is not publication evidence. Exceptional leased ref updates
are construction/classification coverage for an explicit compare-and-set operation, not permission
to replace the normal Beads push.

GitHub documents that it rejects stale non-fast-forward pushes, and Git receive-pack defines the
ref-update boundary:

- https://docs.github.com/en/get-started/using-git/dealing-with-non-fast-forward-errors
- https://git-scm.com/docs/git-receive-pack
- https://git-scm.com/docs/git-daemon

Cloudflare OS's GitHub Gatekeeper tests inject only `fetch` at the HTTP boundary rather than
building a GitHub emulator:
https://github.com/cloudflare/cloudflare-os/blob/main/packages/gatekeeper-github/__tests__/github-api.test.ts.
Beads does not use GitHub REST for this sync path, so this task needs neither that fetch seam nor a
larger API fake. Treat the documented GitHub behavior as the platform contract. A live GitHub run,
credential or local Git server is neither a prerequisite nor a blocker.

### Transport case algorithms

`stale-base-fence`:

1. Script both clients observing G0. A's non-forcing push succeeds; a fresh observation reports G1
   containing A's operation id.
2. B's candidate is still based on G0. Its non-forcing push returns the documented
   non-fast-forward/stale output. A fresh observation remains G1. Classify B as terminal
   `stale_base`; do not retry it and do not report B's operation as published.
3. Script B refreshing G1, semantically replaying its operation and publishing non-forcing G2. A
   paused candidate based on G1 then receives the same terminal stale result, and observation stays
   G2.
4. The final scripted logical export contains the base, winner and replay operation ids exactly
   once. An accepted stale result, an unleased force, a missing expected base or missing history is
   `cutover_blocked`.

`push-conflict-recovery` repeats the two-client script with one operation per client, proves the
loser is terminal at G0, then scripts refresh, semantic replay, commit, non-forcing push and fresh
observation. The scripted export contains both ids exactly once and history retains G0, the winner
and recovery generation. Do not accept a history count alone; compare operation ids and issue
contents before and after.

`transport-retries` has two subcases and does not claim spec 014's operation-receipt behavior:

- Before publication, script a transport failure and a fresh observation still at G0. Retry the
  same prepared candidate and operation id; the push succeeds, observation becomes G1, and the
  logical export contains the operation once.
- After publication, script a lost response followed by observation G1 containing the operation.
  Return the already-published result without issuing a second push. If observation instead stays
  G0, retry the same prepared candidate once; never mint a second candidate or operation id.

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
| `command_output_redacts_secrets_and_paths` (command) | diagnostic includes sentinel secret/path | Structured evidence is reproducible without leaking local details |
| `every_bd_child_has_the_isolated_environment` (command) | one required variable absent | HOME/config/metrics/event/prompt isolation is applied centrally |
| `all_and_transport_accept_no_remote_or_credential_arguments` (cli) | old parser fields or `github_fixture_required` remain | The full contract is self-contained and offline |
| `publication_plan_is_non_forcing_and_observes_before_and_after` (contract) | argv includes force or omits one `ls-remote` | Normal publication cannot overwrite and success requires a fresh generation observation |
| `leased_ref_update_requires_the_exact_expected_generation` (contract) | bare force or missing/wrong lease is accepted | Exceptional CAS commands use only an exact `--force-with-lease` expected SHA |
| `two_real_pinned_stores_initialize_in_independent_roots` (contract case) | paths share a store or one init is recorded rather than real | The two clients are real isolated Beads stores even though remote outcomes are scripted |
| `stealth_init_preserves_sentinels_hooks_index_and_global_config` (contract) | fixture snapshot differs | The exact init command has no repository/global integration side effects |
| `stealth_init_disables_metrics_events_and_auto_push` (contract) | event queue/config exists | No implicit network or background publication is enabled |
| `stale_contender_is_classified_separately_from_transport_failure` (contract) | both map to one error | Callers may retry transport but must refresh after stale base |
| `stale_base_fence_uses_two_independent_clone_and_store_paths` (contract) | paths or Beads data roots are shared | The proof has two clients, not one shared local store |
| `the_first_stale_push_preserves_the_winners_generation` (transport case) | script accepts stale push or reports another SHA | Documented non-fast-forward output is terminal and observation remains G1 |
| `a_paused_former_holder_cannot_publish_after_recovery_advances_the_ref` (transport case) | initial `not_run`; later stale A changes G2 | A prepared G1 candidate cannot replace G2 |
| `guarded_pull_replay_push_preserves_both_operations_once` (transport case) | initial `not_run`; later export/history mismatch | Divergence recovery loses and duplicates no operation |
| `failure_before_publication_retries_the_same_candidate_once` (contract/transport) | script mints a candidate or creates zero/two generations | A failed publication attempt does not consume the semantic operation |
| `lost_response_is_recovered_without_a_second_push` (contract/transport) | recording runner sees another push after G1 is observed | A published operation is rediscovered instead of republished |
| `stale_base_is_never_routed_through_transport_retry` (contract) | recording runner invokes a retry after non-fast-forward | Only safely uninducible transport/lost-response failures use the recording seam |
| `cleanup_drops_embedded_stores_and_removes_both_roots` (contract) | `bd dolt stop` is planned, a harness child remains unwaited or root exists after scope | Embedded cleanup has no server command and owns every real local store and harness child |
| `unsafe_configured_command_or_contradictory_result_is_cutover_blocked` (cli) | unsafe plan passes or absence of live test blocks | Only our unsafe command/result is a blocker; no hosted test is required |

The real pinned binary initializes and cleans up both stores. The existing
`RecordingCommandRunner` supplies the exact external Git/Beads outcomes for publication logic;
there is no second fake or live-remote path. Report this as command-contract evidence, never as a
live GitHub run.

Use these narrow red/green commands; do not run an unrelated package while a group is still red:

```shell
cargo test -p plasmosome-work-state --test pin
cargo test -p plasmosome-work-state --test contract
cargo test -p plasmosome-work-state --test cli
```

### Execution order and evidence

1. Resume from the retained implementation and its recorded pin/hermetic TDD evidence; do not redo
   or weaken passing coverage merely because the transport proof boundary changed.
2. Edit tests only for the removed remote arguments, explicit non-forcing/leased command builders,
   two real store roots and cleanup/reaping. Run the narrow contract/CLI suites and capture reds
   that reach those assertions. Then edit implementation only and rerun them green.
3. Add winner, stale, recovery and retry/idempotency scripts against the existing
   `RecordingCommandRunner`, observe red, then implement only the minimal state machine and output
   classification needed to pass. Do not implement Git or GitHub behavior.
4. Run the real checksum-verified artifact through `contract-test hermetic`, then each of
   `stale-base-fence`, `push-conflict-recovery`, `transport-retries`, `transport` and `all`. They
   require no hosted fixture, local server, credential or internet route.
5. If a scripted case returns `cutover_blocked`, preserve its generations, redacted command
   outcomes and operation ids in `## Notes`; continue through
   coverage and the root gate so the task can finish with an honest negative cutover conclusion.
   Do not change the assertion, add a live remote requirement or start migration. Absence of a live
   GitHub run is not `cutover_blocked`.

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
command safety, result classification, retry and cleanup paths in `contract.rs`; add tests for
meaningful version, checksum, unsafe command, stale/transport distinction and cleanup misses. If `cargo-llvm-cov` is
unavailable, do not install it globally: STOP and report that prerequisite. The real external
subprocess bodies do not need an arbitrary percentage target, but their decision and refusal
branches do.

Then perform one refactor pass. Check repeated command construction, duplicated store cleanup,
oversized test setup and error strings that have become part of the CLI contract. Refactor tests
and implementation in separate steps, rerunning the narrow suite after each. Keep the accepted
`CommandRunner` seam because it has real and recording implementations; add no second abstraction
without two concrete callers.

Run the entire test suite with timing, note its wall time and whether the explicit external cases
remain outside the ordinary suite so subprocess-heavy Beads cases do not slow it:

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
directories and supplied PATH to the isolated child environment. A first verifier run exposed an
unexpected relative `~/.config/bd/config.yaml` footprint because `bd --version` had no isolated
environment; the runner was corrected so verification receives the same isolated environment,
the footprint was moved recoverably into `mktemp -d`, and the repeated hermetic run passed with no
checkout footprint.

2026-09-01: Owner direction supersedes both the hosted-GitHub-fixture prerequisite and the proposed
local smart-HTTP server. The CLI must remove `github_fixture_required`, and no missing live remote
or unsupported local transport is a blocker. Keep the real pinned binary for hermetic initialization
of two independent temporary stores. Prove remote logic at the existing command seam by scripting
the exact Git/Beads results: winner G0 -> G1, terminal stale non-fast-forward with G1 preserved,
guarded recovery to G2, retry before publication and lost-response rediscovery. `cutover_blocked`
now means our configured command is unsafe (unleased force or missing expected base) or a supplied
observation contradicts the documented GitHub/Git contract. No repository, credential, server,
fake forge or new mock dependency is part of task 042.

2026-09-01: The real pinned embedded mode reported `bd dolt stop` is unsupported because it has no
Dolt server. Owner direction corrects the cleanup contract: embedded cleanup must not plan or invoke
that command, instead dropping store handles, reaping only harness-started children and removing
temporary roots. A new cleanup test first failed on the absent no-stop plan, then passed after the
embedded cleanup implementation removed the unsupported command. Real hermetic and two-store
transport commands passed after that correction.

2026-09-01: Revised offline command-contract evidence: tests first failed on the absent offline
parser, scripted runner, publication/lease/retry APIs; then pin (5), contract (8) and CLI (1)
tests passed. With the verified temporary Apple Silicon artifact, `hermetic`, `stale-base-fence`,
`push-conflict-recovery`, `transport-retries`, `transport` and `all` each passed without a hosted
repository, credential, API call or local server. `cargo llvm-cov` 0.6.21 reported 67.39% lines,
54.29% functions and 49.69% regions overall: pin.rs was 91.82% lines; the meaningful remaining
misses are SystemCommandRunner error paths and CLI process exit paths, which ordinary unit tests
do not execute, while the real contract commands cover their successful disposable subprocess
paths. Command safety, stale/transport classification, exact lease base, retry and embedded cleanup
branches are covered by the recording-script tests.
