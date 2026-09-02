---
id: 045
title: Import the Markdown work records into two shadow Beads stores
status: planned
priority: 1
specs: [014]
intents: [015]
refs:
  [
    AGENTS.md,
    Cargo.toml,
    .agents/skills/tasks/SKILL.md,
    .agents/skills/pr-review/SKILL.md,
    docs/intents/015-local-first-shared-work-state.md,
    docs/specs/014-local-first-work-state.md,
    tasks/042-beads-transport-foundation.md,
    tools/work-state,
    tools/work-state-beads-1.1.2.toml,
    crates/plasmosome-work-state/Cargo.toml,
    crates/plasmosome-work-state/AGENTS.md,
    crates/plasmosome-work-state/README.md,
    crates/plasmosome-work-state/src/lib.rs,
    crates/plasmosome-work-state/src/command.rs,
    crates/plasmosome-work-state/src/contract.rs,
    crates/plasmosome-work-state/src/pin.rs,
    crates/plasmosome-work-state/src/main.rs,
    crates/plasmosome-work-state/tests/cli.rs,
    crates/plasmosome-work-state/tests/command.rs,
    crates/plasmosome-work-state/tests/contract.rs,
    crates/plasmosome-work-state/tests/pin.rs,
  ]
done_when:
  - `./tools/work-state contract-test document-mapping --source-ref REF --archive PATH --bd PATH` resolves REF once, dynamically discovers every numeric intent, spec and task in that Git tree, verifies each file against its content-establishing commit, and preserves every logical record through a real Beads export/reimport between two independent temporary stores.
  - The logical round trip preserves the exact three-digit id, kind-qualified key, canonical path, Markdown title, content commit, initial state version and ordered upward links; the same numeric id in the intent, spec and task namespaces remains three distinct records.
  - `./tools/work-state contract-test shadow-parity --source-ref REF --archive PATH --bd PATH` records `markdown-shadow` and the resolved source commit in both temporary stores, then reports no missing, extra or changed lifecycle, task priority, link, PR or evidence projection while Markdown remains authoritative.
  - Both cases use the requested real repository ref without a configured current-record count, and the fixed ref `13c0f68c13743f4db2fb123fef560f3fa12734d1` separately imports exactly 39 tasks.
  - Duplicate ids within one namespace, unresolved typed link targets, reordered links, and a file whose claimed content commit does not contain the imported path and contents are refused with the offending document key; invalid input cannot be accepted as parity.
  - The pinned Beads 1.1.2 binary performs the two local imports and exports under the existing isolated command seam; no live or fake GitHub repository, GitHub API, hosted fixture, local Git server, broad mock framework or committed repository snapshot is introduced.
  - Tests are written and observed failing before each implementation batch, coverage and uncovered branches are reviewed, the timed workspace suite and all five root gates exit 0, and the change makes no claim about reads, freshness, online mutation, leases, reconciliation, CI acquisition, backup/restore or authority cutover.
pr:
evidence:
---

## Why

The transport foundation can create safe disposable Beads stores, but none of Plasmosome's real
work documents can enter them yet. Before any ledger state may become authoritative, the wrapper
needs one exact, repeatable account of which Markdown records exist at a Git revision and what each
record means.

This task adds that one-way shadow import and proves it with the current repository and the fixed
39-task historical revision. Markdown remains the only authority, and the imported stores are
temporary evidence rather than a cutover.

## Plan

### Deliverable, in one sentence

Add the stable document model and `markdown-shadow` adapter so the contract runner can load every
numeric work document from one resolved Git commit, import it into a real temporary Beads 1.1.2
store, export and reimport it into a second independent store, and prove exact document mapping and
shadow-state parity.

### Out of scope

- No `list`, `show`, `ready`, `blocked`, `heartbeat observe` or other local read/freshness command,
  freshness envelope, ready/blocked calculation or network-disable proof.
- No online authoritative mutation, operation receipt, expected-version update, writer lease, task
  ownership lease, claim, dispatch, branch effect, publication or write to `refs/dolt/data`.
- No lifecycle transition or gate implementation. Imported lifecycle values are Markdown shadow
  projections only; this task neither approves nor advances any record.
- No GitHub polling or reconciliation, no PR/check/review/merge upsert, and no inference from live
  forge state. `pr:` and `evidence:` come only from the selected Markdown revision.
- No agent instruction, skill, template, selector, hook or CI cutover. Do not add artifact download
  to CI and do not edit the work-state rules outside this task and the crate's own description.
- No backup, restore, rollback, write freeze, dual-authority enforcement, ledger authority epoch or
  authority cutover. The project checkout gets no `.beads/` or `.plasmosome/` store.
- No incremental import into a long-lived store. Every contract run starts two fresh stores, so
  every document starts at `state_version = 1`; later mutation/version behavior is separate work.
- No repair of legacy Markdown. Preserve empty or incomplete copied link lists exactly, including
  tasks whose `intents:` copy differs from their specs' closure. This task validates that every id
  which is present resolves to the required kind; lifecycle gate work may evaluate closure later.
- No committed copy of the repository at either source revision. Derive both successful corpora
  from Git. Small in-memory record builders are allowed only for the explicit refusal tests below;
  exact finite command outputs may test command ordering and parsing but may not model a project.
- No live GitHub repository, credential, GitHub API mock, hosted fixture, fake forge, local Git
  server, daemon, container or new mock dependency. Keep the existing real/recording command seam.
- No new parser or process dependency. The repository's frontmatter grammar needed here is small
  enough to parse directly, and the existing `serde_json` support covers the Beads interchange.
- No heartbeat work. STOP when this task is done; do not start the next work-state capability.

### Files to read, and nothing else

Read only this task and the files listed in `refs:`. The accepted behavior is in spec 014; task 042
defines the pinned artifact, isolated environment, two-store fixture and command boundary being
extended. Do not explore unrelated crates, tasks, branches or the open work for tasks 030, 043 and
044.

Create or edit exactly these files:

- `crates/plasmosome-work-state/AGENTS.md` and `README.md`, replacing only the obsolete statement
  that this crate never migrates work state with the bounded shadow-import capability;
- `crates/plasmosome-work-state/src/lib.rs`, `command.rs`, `contract.rs` and `main.rs`;
- new `crates/plasmosome-work-state/src/document.rs` and `shadow.rs`;
- `crates/plasmosome-work-state/tests/cli.rs`, `command.rs` and `contract.rs`;
- new `crates/plasmosome-work-state/tests/document.rs` and `shadow.rs`; and
- this task only for lifecycle fields and dated `## Notes` evidence.

Do not add a crate or dependency and do not edit `Cargo.toml`, `Cargo.lock`, the pin manifest,
`pin.rs`, its passing tests, `tools/work-state`, the intent, the accepted spec, skills, templates,
hooks or CI. If the design below cannot be implemented within that boundary, STOP and report the
contradiction instead of expanding the task.

### Stable source model

`document.rs` owns two serializable value types and one typed error:

```text
DocumentRecord
  document_key       "intent:015" | "spec:014" | "task:001"
  kind               intent | spec | task
  document_id        exactly three ASCII decimal digits
  document_path      canonical Git-relative Markdown path
  title              non-empty Markdown frontmatter title
  content_commit_sha exact lower-case 40-hex establishing commit
  state_version      1 in this task
  intent_ids         ordered Vec of three-digit ids
  spec_ids           ordered Vec of three-digit ids

MarkdownShadow
  lifecycle          exact kind-specific Markdown status
  priority           Some(1..=3) for a task, None otherwise
  pr                 task Markdown value, absent/blank as None
  evidence           task Markdown value, absent/blank as None
```

Wrap the pair as `ShadowDocument`. Equality is typed equality over every field; do not compare a
subset or an unordered map. `DocumentError` carries a stable code plus `offending_key` when the
path contains enough information to form one. Use these codes in the contract result:
`invalid_source_ref`, `invalid_document`, `duplicate_document_id`,
`missing_document_target`, `content_commit_mismatch`, `document_mapping_mismatch` and
`shadow_parity_mismatch`. Source/parity violations exit 1; malformed CLI input exits 2.

Resolve the supplied source ref exactly once with real Git, require one 40-hex commit, and use that
commit for every later read. Run Git with the same cleared, isolated environment as the existing
fixture, with the actual worktree root as `cwd`. The only source-repository commands are:

```text
git rev-parse --verify --end-of-options <source-ref>^{commit}
git ls-tree -r --name-only <resolved-sha> -- docs/intents docs/specs tasks
git show <resolved-sha>:<document-path>
git log -1 --format=%H <resolved-sha> -- <document-path>
git show <content-commit-sha>:<document-path>
```

First make the existing process seam fail closed on non-UTF-8 output. Replace lossy stdout/stderr
conversion in `SystemCommandRunner` with checked UTF-8 conversion and a stable command error;
otherwise two invalid blobs can both acquire replacement characters and appear equal. Keep
`CommandOutput` and the trait shape unchanged. This small seam correction is covered before the
document implementation and applies centrally to the real Git reads.

Add `GIT_NO_LAZY_FETCH=1` to every source-repository Git `CommandSpec`. A missing object in a
partial/promisor clone must refuse locally instead of causing `rev-parse`, `log` or `show` to fetch
from `origin`. Test the constructed environment explicitly; do not rely on the caller's Git config
or network state.

Reject a missing, failed, multi-line or non-40-hex resolution. From `ls-tree`, accept only these
canonical paths and sort by kind in `intent`, `spec`, `task` order, then numeric id:

```text
docs/intents/NNN-<non-empty-slug>.md
docs/specs/NNN-<non-empty-slug>.md
tasks/NNN-<non-empty-slug>.md
```

Ignore the three directory `README.md` files and other nonnumeric files. A basename beginning with
three digits but not matching its namespace's canonical form is `invalid_document`, not an ignored
record. The id in frontmatter must equal the path prefix. Reject a second path with the same kind
and id, but do not collide equal ids across kinds. Derive the immutable key from kind plus id;
never derive it from the slug or a Beads row id.

For every path, take `git log -1` as the newest commit on the selected history which established
that current path/content revision. Require that its second `git show` returns exactly the same
UTF-8 contents as the file at the resolved source commit. An unrelated later source commit therefore
does not replace `content_commit_sha`; a moved path or edited file does. Refuse a missing path,
invalid SHA or different contents before a Beads import command runs.

Parse only the delimited frontmatter and only the fields this model consumes. Required scalar
fields must occur exactly once. Parse `intents:` and `specs:` as one-line flow lists while retaining
input order and exact three-digit strings. Accept a task `pr:` or `evidence:` as absent, blank, a
plain scalar, or the folded/literal block scalar forms already used by the repository; reconstruct
the scalar text using YAML newline folding/chomping rules needed by those forms. Do not mistake an
`evidence:` line in the Markdown body for frontmatter. Validate the current lifecycle vocabularies
and task priorities, but ignore unrelated durable fields rather than trying to parse all YAML.

After every record is parsed, validate targets in one complete pass so forward references work:
each spec `intent_ids` entry and each task `intent_ids` entry resolves to an intent, and each task
`spec_ids` entry resolves to a spec. An empty list is valid shadow input. Do not sort, deduplicate,
complete or otherwise repair a list. Duplicate entries in one list are retained; parity compares
them exactly. This is source reconstruction, not the later lifecycle gate.

### Beads shadow representation and one-way adapter

`shadow.rs` converts validated `ShadowDocument` values to Beads JSONL and back. Use one explicit,
stable native id per namespace (`plasmosome-intentNNN`, `plasmosome-specNNN`,
`plasmosome-taskNNN`), set `external_ref` to the logical `document_key`, keep Beads `title` equal to
the Markdown title, and use Beads priority 2 for non-tasks and the exact Markdown priority for
tasks. Do not expose the native id as part of `DocumentRecord`.

Keep the exact logical and shadow fields in one nested `metadata.plasmosome_document` object with
`schema_version = 1` and `authority_mode = "markdown-shadow"`. Use Beads' ordinary `open` status
for this import rather than pretending its built-in status vocabulary is the Plasmosome lifecycle;
the exact lifecycle remains the named shadow metadata value. Do not copy document body prose into
the issue description. Markdown and Git remain the content authority.

Before importing, serialize every row to a JSONL file under that clone's temporary root. Invoke
the verified binary through `SystemCommandRunner` with the fixture repository as `cwd`, its exact
isolated environment, and the file argument redacted:

```text
bd --sandbox import <redacted-jsonl-path> --json
bd --sandbox export
bd --sandbox kv set plasmosome.authority-mode markdown-shadow
bd --sandbox kv set plasmosome.source-commit <resolved-sha>
bd --sandbox kv get plasmosome.authority-mode
bd --sandbox kv get plasmosome.source-commit
```

Construct those same plans through the existing `CommandRunner`; do not spawn around it and do not
add stdin behavior to the seam. Check every exit status and parse the import JSON response. The
created ids/count must equal the validated input. Parse `bd export` JSONL with unknown outer Beads
fields tolerated but with the nested Plasmosome schema strict. Reject duplicate logical keys,
duplicate native ids, missing metadata, a title/external-ref disagreement, a non-shadow mode or an
invalid logical field.

Set and read back the two project keys in each fresh store. This records which authority and exact
Git snapshot populated it; it does not create an authority transition. No Git command may write
the source checkout, and no Beads command may run with that checkout as its store.

Populate clone A from the Git-derived records, export A, decode it to the typed logical form and
serialize that canonical logical export. Decode the logical export and import it into fresh clone
B, then export B. Compare source -> A -> logical export -> B in two passes:

- `document-mapping` compares record count/key sets and every `DocumentRecord` field, including
  list position and duplicates; and
- `shadow-parity` additionally compares every `MarkdownShadow` field.

Either comparison names the first offending key in canonical order and reports whether it was
missing, extra or different. It must not turn lists into sets. A changed title in Beads fails both
mapping and shadow parity because imported titles are read-only Markdown projections.

### Contract runner surface and evidence

Extend `ContractRequest` with an optional source ref and add only these commands:

```text
./tools/work-state contract-test document-mapping --source-ref REF --archive PATH --bd PATH
./tools/work-state contract-test shadow-parity --source-ref REF --archive PATH --bd PATH
```

Both new individual cases require exactly one non-empty `--source-ref`; duplicate flags, a missing
value or a source ref on another individual case is `invalid_command`. Preserve the existing
`all --archive PATH --bd PATH` form from task 042 by defaulting that aggregate alone to
`origin/main`; `all` may also accept one explicit `--source-ref` override. It runs the existing
hermetic/transport probes plus both new comparisons using one validated snapshot and one pair of
fresh stores. Existing individual pin, hermetic and transport forms remain unchanged.

Extend structured results without removing current fields. New-case output includes the input ref,
resolved source commit, per-kind counts, total count, a SHA-256 of the canonical logical export,
both clone labels, `markdown-shadow`, redacted Git/Beads command plans and `offending_key` on a
refusal. Never print the worktree path, temporary JSONL path, archive path or binary path. A
requested case is never skipped.

The current count is deliberately not asserted. The test and manual run against
`origin/main` compare discovery with that tree at runtime. A separate real-source test and manual
run use `13c0f68c13743f4db2fb123fef560f3fa12734d1` and assert exactly 39 task records; do not encode
39 in importer configuration or use it as a maximum. Do not create a branch, tag, archive or copied
tree for either source.

### TDD test table

Work in the batches below. For each batch, edit tests only, run the named narrow command and record
the expected red in `## Notes`. Then edit implementation only and rerun to green. A missing module
or API is an acceptable first compile red; after it exists, new reds must reach the assertion they
name. Never change a test and its implementation in the same step, and never relax an assertion to
make an implementation pass.

| Test | Initial failing observation | What passing proves |
| --- | --- | --- |
| `system_runner_refuses_non_utf8_output_instead_of_replacing_it` | invalid bytes become replacement characters | Exact Git content comparison cannot normalize two invalid blobs into equality |
| `requested_ref_is_resolved_once_before_numeric_documents_are_read` | source loader/API absent | A mutable ref becomes one SHA and every later command uses only that SHA |
| `requested_tree_paths_are_discovered_without_a_configured_count` | recorded `ls-tree` gains no corresponding records | Discovery follows its complete command result rather than a fixed array |
| `real_document_mapping_discovers_the_selected_repository_ref` (explicit contract) | case is absent or omits paths | The requested real tree's numeric paths and imported keys agree dynamically |
| `fixed_historical_source_contains_exactly_39_tasks` (explicit contract) | case is absent or wrong count | The fixed real commit remains the required 39-task migration witness |
| `equal_ids_in_three_namespaces_make_three_distinct_keys` (explicit contract) | id alone collides | Intent, spec and task ids never share a namespace |
| `content_commit_establishes_the_selected_path_and_contents` (explicit contract) | every record receives source head or no blob check | Each stored SHA is the path/content commit and its blob equals the selected file |
| `frontmatter_reconstructs_status_priority_pr_and_evidence_forms` (explicit contract) | parser omits a field form | Scalar, blank/absent and folded evidence plus numeric/URL PR forms survive |
| `legacy_link_copies_are_preserved_without_repair` (explicit contract) | validator fills/rejects a known legacy list | Shadow import represents the authoritative Markdown exactly |
| `numeric_noncanonical_path_or_path_id_mismatch_refuses` | a numeric file is ignored or accepted under another id | Every numeric document has one canonical path/id identity |
| `missing_or_duplicate_required_frontmatter_refuses` | parser accepts an absent or repeated field | A malformed document cannot opt out of its mapping |
| `invalid_lifecycle_or_task_priority_refuses` | unknown values enter metadata | The shadow projection has only the existing vocabularies |
| `duplicate_id_within_one_kind_names_the_key_before_import` | duplicate builder is accepted | Duplicate source ids fail closed without a Beads write |
| `missing_typed_target_names_the_source_key_before_import` | missing intent/spec builder is accepted | Every present upward id resolves to exactly one required kind |
| `content_commit_mismatch_names_the_key_before_import` | scripted Git blobs differ but load succeeds | A claimed SHA cannot describe other contents |
| `source_git_plans_disable_lazy_fetch` | source environment lacks the guard | Missing objects cannot make a hidden network request |
| `beads_jsonl_round_trip_preserves_every_typed_field` (explicit contract) | adapter/case absent | Native representation loses no logical or shadow value |
| `first_shadow_import_sets_state_version_one` (explicit contract) | version absent/defaulted | Fresh migration starts the public per-record counter correctly |
| `shadow_commands_are_isolated_local_and_redact_the_import_path` | command plan absent/wrong | Import/export/KV use the existing seam without network or path disclosure |
| `mode_and_source_commit_are_verified_in_each_store` (explicit contract) | KV commands absent or unchecked | Both stores identify `markdown-shadow` and the exact selected commit |
| `duplicate_native_or_logical_ids_refuse_on_export` | decoder accepts an ambiguous row set | One Beads row and one logical key identify each document |
| `missing_unknown_or_inconsistent_metadata_refuses` | decoder defaults or ignores a bad projection | Schema, mode, title and external ref must agree before parity |
| `import_result_must_name_the_exact_ids_and_count` | adapter trusts exit 0 alone | A partial or substituted Beads import is not reported as complete |
| `changed_link_order_refuses_with_the_offending_key` | equality sorts arrays | Position is part of the mapping contract |
| `missing_extra_and_changed_mapping_records_each_refuse` | comparison checks only overlaps | Mapping parity is a complete key-set and value comparison |
| `changed_lifecycle_priority_pr_or_evidence_each_refuses` | comparison ignores a shadow field | Shadow parity covers every volatile Markdown projection in this task |
| `document_mapping_case_exports_and_reimports_two_real_stores` | CLI says `invalid_command` | The pinned binary performs the complete A-to-B logical round trip |
| `shadow_parity_case_populates_two_real_stores_without_github` | CLI says `invalid_command` | Real local Beads exports equal the real Git-derived shadow projection |
| `individual_new_cases_require_source_ref_and_all_defaults_to_origin_main` | parser accepts an ambiguous form or breaks the old aggregate | The CLI resolves an explicit ref and preserves spec 014's existing `all` form |
| `contract_refusal_exits_nonzero_and_serializes_offending_key` | exit remains generic/input-only | Every named malformed-record path is observable and fails closed |
| `all_includes_mapping_and_shadow_evidence` | aggregate contains transport only | The implemented contract aggregate cannot skip migration parity |

Use these narrow commands in order:

```shell
cargo test -p plasmosome-work-state --test command
cargo test -p plasmosome-work-state --test document
cargo test -p plasmosome-work-state --test shadow
cargo test -p plasmosome-work-state --test contract
cargo test -p plasmosome-work-state --test cli
cargo test -p plasmosome-work-state
```

For the explicit refusal builders, keep only enough records to exercise the named edge. A
`RecordingCommandRunner` script may supply the two unequal Git blob outputs for the content/SHA
case and exact Beads export output for order/difference cases. Do not implement repository, Git,
GitHub or Beads behavior inside the recording runner, and do not replace the real-source or real
binary acceptance runs with those unit tests. Ordinary `cargo test` must not require
`origin/main`, the fixed historical object, a remote-tracking ref, a full clone or the pinned
artifact; those are exercised by the explicit contract commands below so the root gate remains
hermetic in shallow and archive checkouts.

### Execution order

1. Obtain the matching archive and extracted `bd` only in a uniquely named `mktemp -d` directory,
   or reuse caller-supplied paths already verified by task 042. Do not install anything or write a
   path into the repository. Set `ARCHIVE_045` and `BD_045` to those absolute temporary paths.
2. Add the new request/result and aggregate expectations to `tests/cli.rs` and
   `tests/contract.rs` only. Run both targets red, then invoke the two new real contract commands
   and record their `invalid_command` red before any implementation edit. Leave this acceptance
   batch red while building its lower layers; do not weaken it to match an intermediate state.
3. Add the checked-output test to `tests/command.rs` only and run its narrow target red against the
   existing lossy conversion. Edit `command.rs` only, then rerun it green before touching document
   code.
4. Add `tests/document.rs` only. Run its narrow command red. Use exact finite
   `RecordingCommandRunner` scripts only for command ordering/output parsing and small in-memory
   builders only for the named refusal inputs; do not embed a successful repository corpus.
   Implement `document.rs` and its `lib.rs` export only, then run it green.
5. Add `tests/shadow.rs` only. Run it red. Implement `shadow.rs` only, using temporary JSONL files
   and the existing command types, then run it green. Refusal tests must prove no Beads import was
   dispatched before source validation completed.
6. Wire `contract.rs` to resolve/validate a snapshot before initializing the two stores, pass those
   stores to the shadow adapter and preserve every existing transport case. Then edit `main.rs`
   and request/result parsing so the initial acceptance batch has the planned exit classification
   and structured offending key. Run contract, CLI and the complete package green.
7. Update the crate `AGENTS.md` and `README.md` only after behavior is green. They should say the
   crate can build disposable Markdown-shadow migrations while Markdown remains authoritative,
   and should still deny production state mutation or installation.
8. Run the exact real cases below. Inspect JSON rather than accepting exit 0 alone: both clone
   labels, source SHA, counts, mode, digest and redacted plans must be present, and the historical
   shadow run must report 39 tasks.

```shell
./tools/work-state contract-test document-mapping --source-ref origin/main \
  --archive "$ARCHIVE_045" --bd "$BD_045"
./tools/work-state contract-test shadow-parity --source-ref origin/main \
  --archive "$ARCHIVE_045" --bd "$BD_045"
./tools/work-state contract-test document-mapping \
  --source-ref 13c0f68c13743f4db2fb123fef560f3fa12734d1 \
  --archive "$ARCHIVE_045" --bd "$BD_045"
./tools/work-state contract-test shadow-parity \
  --source-ref 13c0f68c13743f4db2fb123fef560f3fa12734d1 \
  --archive "$ARCHIVE_045" --bd "$BD_045"
./tools/work-state contract-test all --source-ref origin/main \
  --archive "$ARCHIVE_045" --bd "$BD_045"
```

If a real case fails, preserve its source SHA, record counts, offending key and redacted last
command in `## Notes`. Fix our parser, mapping or adapter when it violates the accepted contract;
do not add a hosted service, a copied repository fixture or a permissive comparison. A missing
historical object is a prerequisite/CI-acquisition problem outside this task: STOP and report it
rather than fetching inside the runner or deleting the fixed assertion.

### Coverage, refactor pass and definition of done

After the real cases and package tests are green, use the already-installed coverage tool. Do not
install or update a global component:

```shell
cargo llvm-cov --version
cargo llvm-cov clean --workspace
cargo llvm-cov -p plasmosome-work-state --all-targets --branch --no-report
cargo llvm-cov run -p plasmosome-work-state --bin plasmosome-work-state \
  --branch --no-report --no-clean -- contract-test all --source-ref origin/main \
  --archive "$ARCHIVE_045" --bd "$BD_045"
cargo llvm-cov run -p plasmosome-work-state --bin plasmosome-work-state \
  --branch --no-report --no-clean -- contract-test shadow-parity \
  --source-ref 13c0f68c13743f4db2fb123fef560f3fa12734d1 \
  --archive "$ARCHIVE_045" --bd "$BD_045"
cargo llvm-cov report -p plasmosome-work-state --all-targets --branch --lcov \
  --output-path target/task-045-lcov.info
cargo llvm-cov report -p plasmosome-work-state --all-targets --branch --summary-only
```

Record the line, function, region and branch summary in `## Notes`. Inspect uncovered branches in
`document.rs`, `shadow.rs`, the new contract dispatch/result code and `main.rs`. Add tests for
meaningful path/id/frontmatter, ref/SHA/blob, target-resolution, complete-set comparison, list-order,
metadata/KV/import-result and exit-code misses. There is no arbitrary percentage target; real
subprocess failure bodies need not be forced, but every decision and refusal introduced by this
task does. If `cargo-llvm-cov` is unavailable, STOP and report the missing prerequisite.

Perform one refactor pass after coverage. Look for repeated Git command construction, repeated
frontmatter field handling, JSONL encode/decode duplication, overly broad error conversion and
oversized fixture builders. Refactor tests and run them green before editing implementation; then
refactor implementation separately and rerun the package. Keep `CommandRunner` as the sole process
seam and do not add an abstraction without two concrete uses.

Finally run the full suite with timing and all five root gates:

```shell
/usr/bin/time -p cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
./.githooks/provenance-guard
./.githooks/attribution-guard
```

Record every exit code and the workspace test wall time in `## Notes`. Compare the duration with
task 042's 7.27-second recorded baseline and note any material increase; real pinned-binary cases
remain explicit contract commands so ordinary tests do not acquire artifacts or make network
calls. All five commands must exit 0. The PR description leads with the missing migration
capability from `## Why`, says explicitly that Markdown is still authoritative and the stores are
temporary, ends with `task: 045`, and names no model as author or co-author. STOP when done — do not
start local reads, mutation, reconciliation or cutover.

## Notes
