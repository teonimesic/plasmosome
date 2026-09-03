use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::command::{
    CommandOutput, CommandRunner, CommandSpec, RecordingCommandRunner, SystemCommandRunner,
};
use crate::document::{
    DocumentError, DocumentKind, ShadowDocument, SourceDocuments, is_lower_hex_sha, load_documents,
};
use crate::freshness::{Freshness, RemoteRelation};
use crate::pin::{PinManifest, VerifiedBeads};
use crate::project::{ProjectConfig, compiled_project_config};
use crate::read::{ReadCommand, project_read, render_human};
use crate::shadow::{
    ActiveOwner, OperationalDocument, OperationalMetadata, ShadowError, ShadowStore,
    canonical_logical_export, canonical_operational_projection, compare_document_mapping,
    compare_shadow_parity, decode_logical_export, decode_operational_beads_jsonl,
    import_operational_shadow_documents, import_shadow_documents, logical_export_digest, native_id,
    operational_projection_digest, to_operational_beads_jsonl,
};
use crate::store::{
    ActivationFault, BootstrapLock, BootstrapOutcome, BootstrapRequest, CurrentGeneration,
    GenerationActivationLock, StateManifest, activate_staged_generation, bootstrap,
    current_generation, locate_store, locator_environment, read_disposable_snapshot,
};
use crate::sync::{synchronize, synchronize_after_disposable_cleanup_failure_for_contract};

const ISOLATED: &[(&str, &str)] = &[
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("BD_DISABLE_METRICS", "1"),
    ("BD_DISABLE_EVENT_FLUSH", "1"),
    ("BD_NON_INTERACTIVE", "1"),
    ("CI", "true"),
    ("GIT_TERMINAL_PROMPT", "0"),
];

#[derive(Clone, Debug)]
pub struct ContractRequest {
    pub case: String,
    pub source_ref: Option<String>,
    pub archive: PathBuf,
    pub binary: PathBuf,
}

/// Counts of reconstructed Markdown documents, grouped by their namespace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DocumentCounts {
    /// Number of canonical numeric intent records from the resolved source commit.
    ///
    /// Consumers may report this as mapping evidence but must not treat it as migration or cutover
    /// authority.
    pub intent: usize,
    /// Number of canonical numeric spec records from the resolved source commit.
    ///
    /// Consumers may report this as mapping evidence but must not treat it as migration or cutover
    /// authority.
    pub spec: usize,
    /// Number of canonical numeric task records from the resolved source commit.
    ///
    /// Consumers may report this as mapping evidence but must not treat it as migration or cutover
    /// authority.
    pub task: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContractResult {
    pub case: String,
    pub outcome: String,
    pub code: String,
    pub beads_version: String,
    pub clone_labels: Vec<String>,
    pub observed_base: Option<String>,
    pub final_generation: Option<String>,
    pub operation_ids: Vec<String>,
    pub command_plans: Vec<String>,
    pub scenarios: Vec<ScenarioEvidence>,
    pub source_ref: Option<String>,
    pub source_commit: Option<String>,
    pub document_counts: Option<DocumentCounts>,
    pub total_document_count: Option<usize>,
    pub logical_export_sha256: Option<String>,
    pub authority_mode: Option<String>,
    pub offending_key: Option<String>,
    pub mismatch: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScenarioEvidence {
    pub case: String,
    pub observed_base: String,
    pub final_generation: String,
    pub operation_ids: Vec<String>,
    pub command_plans: Vec<String>,
}

impl ContractResult {
    fn passed(case: &str, labels: Vec<String>) -> Self {
        Self {
            case: case.into(),
            outcome: "passed".into(),
            code: "ok".into(),
            beads_version: "1.1.2".into(),
            clone_labels: labels,
            observed_base: None,
            final_generation: None,
            operation_ids: Vec::new(),
            command_plans: Vec::new(),
            scenarios: Vec::new(),
            source_ref: None,
            source_commit: None,
            document_counts: None,
            total_document_count: None,
            logical_export_sha256: None,
            authority_mode: None,
            offending_key: None,
            mismatch: None,
        }
    }
    fn refusal(case: &str, code: &str) -> Self {
        Self {
            case: case.into(),
            outcome: "refused".into(),
            code: code.into(),
            beads_version: "1.1.2".into(),
            clone_labels: Vec::new(),
            observed_base: None,
            final_generation: None,
            operation_ids: Vec::new(),
            command_plans: Vec::new(),
            scenarios: Vec::new(),
            source_ref: None,
            source_commit: None,
            document_counts: None,
            total_document_count: None,
            logical_export_sha256: None,
            authority_mode: None,
            offending_key: None,
            mismatch: None,
        }
    }
}

/// Returns the process exit code for a structured contract refusal.
pub fn contract_refusal_exit_code(code: &str) -> i32 {
    if matches!(
        code,
        "cutover_blocked"
            | "invalid_source_ref"
            | "source_ref_unavailable"
            | "invalid_document"
            | "duplicate_document_id"
            | "missing_document_target"
            | "content_commit_mismatch"
            | "document_mapping_mismatch"
            | "shadow_parity_mismatch"
    ) {
        1
    } else {
        2
    }
}

/// Returns whether a case must execute the real mapping and shadow-parity round trip.
pub fn requires_shadow_round_trip(case: &str) -> bool {
    matches!(
        case,
        "document-mapping" | "shadow-parity" | "online-sync" | "all"
    )
}

/// Returns whether a contract case exercises the installed local read projection.
pub fn requires_local_read_contract(case: &str) -> bool {
    matches!(
        case,
        "local-reads" | "freshness" | "combined-freshness" | "all"
    )
}

/// Returns the concrete installed-local-read scenarios selected by one contract invocation.
pub fn local_read_cases(case: &str) -> &'static [&'static str] {
    match case {
        "local-reads" => &["local-reads"],
        "freshness" => &["freshness"],
        "combined-freshness" => &["combined-freshness"],
        "all" => &["local-reads", "freshness", "combined-freshness"],
        _ => &[],
    }
}

/// Returns the explicit online synchronization contract selected by an individual or aggregate
/// invocation. The aggregate uses this one enumeration so the case cannot be duplicated.
pub fn online_sync_contract_cases(case: &str) -> &'static [&'static str] {
    match case {
        "online-sync" | "all" => &["online-sync"],
        _ => &[],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushFailure {
    StaleBase,
    Transport,
    Other,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Publication {
    Published {
        operation: String,
        generation: String,
    },
    StaleBase {
        operation: String,
        generation: String,
    },
    Recovered {
        operation: String,
        generation: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptEvidence {
    pub observed_base: String,
    pub final_generation: String,
    pub operation_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct StoreFixture {
    pub label: String,
    pub clone_root: PathBuf,
    pub repository: PathBuf,
    pub store_root: PathBuf,
    pub environment: BTreeMap<String, String>,
    sentinels: BTreeMap<PathBuf, Vec<u8>>,
    hook_paths: BTreeSet<PathBuf>,
    hooks_snapshotted: bool,
}

impl StoreFixture {
    pub fn snapshot_git_state(&mut self) -> Result<(), &'static str> {
        for path in [
            self.repository.join(".git/index"),
            self.repository.join(".git/config"),
        ] {
            let contents = std::fs::read(&path).map_err(|_| "cutover_blocked")?;
            self.sentinels.insert(path, contents);
        }
        let hooks = self.repository.join(".git/hooks");
        self.hook_paths = std::fs::read_dir(&hooks)
            .map_err(|_| "cutover_blocked")?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|_| "cutover_blocked")
            })
            .collect::<Result<_, _>>()?;
        self.hooks_snapshotted = true;
        for path in &self.hook_paths {
            let contents = std::fs::read(path).map_err(|_| "cutover_blocked")?;
            self.sentinels.insert(path.clone(), contents);
        }
        Ok(())
    }

    pub fn assert_unchanged(&self) -> Result<(), &'static str> {
        if !self.hooks_snapshotted {
            return self.assert_sentinels_unchanged();
        }
        let hooks = self.repository.join(".git/hooks");
        let current_hook_paths = std::fs::read_dir(&hooks)
            .map_err(|_| "cutover_blocked")?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|_| "cutover_blocked")
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if current_hook_paths != self.hook_paths {
            return Err("cutover_blocked");
        }
        self.assert_sentinels_unchanged()
    }

    pub fn assert_after_stealth_init(&self) -> Result<(), &'static str> {
        let local_config = self.repository.join(".git/config");
        let hooks = self.repository.join(".git/hooks");
        let current_hook_paths = std::fs::read_dir(&hooks)
            .map_err(|_| "cutover_blocked")?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|_| "cutover_blocked")
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if current_hook_paths != self.hook_paths {
            return Err("cutover_blocked");
        }
        for (path, expected) in &self.sentinels {
            let actual = std::fs::read(path).map_err(|_| "cutover_blocked")?;
            if path == &local_config {
                let before = config_entries(expected)?;
                let mut after = config_entries(&actual)?;
                after.retain(|(key, value)| !(key == "beads.role" && value == "maintainer"));
                if before != after {
                    return Err("cutover_blocked");
                }
            } else if actual != *expected {
                return Err("cutover_blocked");
            }
        }
        Ok(())
    }

    fn assert_sentinels_unchanged(&self) -> Result<(), &'static str> {
        for (path, expected) in &self.sentinels {
            if std::fs::read(path).map_err(|_| "cutover_blocked")? != *expected {
                return Err("cutover_blocked");
            }
        }
        Ok(())
    }
}

fn config_entries(contents: &[u8]) -> Result<Vec<(String, String)>, &'static str> {
    let contents = std::str::from_utf8(contents).map_err(|_| "cutover_blocked")?;
    let mut section = String::new();
    let mut entries = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        {
            section = name.trim().to_ascii_lowercase();
            continue;
        }
        let (key, value) = line.split_once('=').unwrap_or((line, "true"));
        entries.push((
            format!("{section}.{}", key.trim().to_ascii_lowercase()),
            value.trim().to_owned(),
        ));
    }
    entries.sort();
    Ok(entries)
}

pub fn prepare_store_fixture(root: &Path, label: &str) -> Result<StoreFixture, &'static str> {
    let clone_root = root.join(label);
    let repository = clone_root.join("repository");
    std::fs::create_dir_all(&repository).map_err(|_| "cutover_blocked")?;
    for name in [
        "home",
        "xdg_config_home",
        "xdg_cache_home",
        "xdg_data_home",
        "tmpdir",
    ] {
        std::fs::create_dir_all(clone_root.join(name)).map_err(|_| "cutover_blocked")?;
    }
    let environment = isolated_environment(&clone_root);
    let files = [
        (repository.join("AGENTS.md"), b"fixture agents\n".as_slice()),
        (repository.join("CLAUDE.md"), b"fixture claude\n".as_slice()),
        (
            repository.join("tracked.txt"),
            b"tracked fixture\n".as_slice(),
        ),
        (
            clone_root.join("git_config_global"),
            b"[user]\n\temail = fixture-global@example.invalid\n".as_slice(),
        ),
    ];
    let mut sentinels = BTreeMap::new();
    for (path, contents) in files {
        std::fs::write(&path, contents).map_err(|_| "cutover_blocked")?;
        sentinels.insert(path, contents.to_vec());
    }
    Ok(StoreFixture {
        label: label.into(),
        clone_root,
        repository: repository.clone(),
        store_root: repository.join(".beads"),
        environment,
        sentinels,
        hook_paths: BTreeSet::new(),
        hooks_snapshotted: false,
    })
}

pub fn dispose_fixture_root(root: TempDir) -> Result<(), &'static str> {
    root.close().map_err(|_| "fixture_cleanup_failed")
}

pub fn validate_independent_stores(
    first: &StoreFixture,
    second: &StoreFixture,
) -> Result<(), &'static str> {
    if first.clone_root == second.clone_root
        || first.repository == second.repository
        || first.store_root == second.store_root
    {
        return Err("cutover_blocked");
    }
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "TMPDIR",
        "GIT_CONFIG_GLOBAL",
    ] {
        if first.environment.get(key) == second.environment.get(key) {
            return Err("cutover_blocked");
        }
    }
    Ok(())
}

pub fn validate_logical_export(
    output: &str,
    expected_operations: &[&str],
) -> Result<(), &'static str> {
    let mut exported_operations = Vec::new();
    for line in output.lines() {
        let value: serde_json::Value = serde_json::from_str(line).map_err(|_| "cutover_blocked")?;
        let id = value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or("cutover_blocked")?;
        let title = value
            .get("title")
            .and_then(serde_json::Value::as_str)
            .ok_or("cutover_blocked")?;
        let description = value
            .get("description")
            .and_then(serde_json::Value::as_str)
            .ok_or("cutover_blocked")?;
        let operation = title.strip_prefix("operation:").ok_or("cutover_blocked")?;
        if id != format!("issue-{operation}") || description != format!("issue:{operation}") {
            return Err("cutover_blocked");
        }
        exported_operations.push(operation.to_owned());
    }
    let exported_operations = exported_operations
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    validate_operations_once(&exported_operations, expected_operations)
}

fn validate_operations_once(
    exported_operations: &[&str],
    expected_operations: &[&str],
) -> Result<(), &'static str> {
    let exported =
        exported_operations
            .iter()
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, operation| {
                *counts.entry(operation).or_default() += 1;
                counts
            });
    let expected =
        expected_operations
            .iter()
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, operation| {
                *counts.entry(operation).or_default() += 1;
                counts
            });
    if exported == expected && exported.values().all(|count| *count == 1) {
        Ok(())
    } else {
        Err("cutover_blocked")
    }
}

pub fn validate_scripted_history<R: CommandRunner>(
    runner: &mut R,
    expected_generations: &[&str],
    expected_operations: &[&str],
) -> Result<(), String> {
    let output = runner.run(production_command(
        "git",
        vec![
            "log".into(),
            "--reverse".into(),
            "--format=%H%x09%s".into(),
            "refs/dolt/data".into(),
        ],
        Vec::new(),
    ))?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    let entries = output
        .stdout
        .lines()
        .map(|line| line.split_once('\t').ok_or("cutover_blocked"))
        .collect::<Result<Vec<_>, _>>()?;
    if entries.iter().map(|(sha, _)| *sha).collect::<Vec<_>>() != expected_generations {
        return Err("cutover_blocked".into());
    }
    let operations = entries
        .iter()
        .filter_map(|(_, content)| content.strip_prefix("operation:"))
        .collect::<Vec<_>>();
    validate_operations_once(&operations, expected_operations).map_err(str::to_owned)
}

pub fn isolated_environment(root: &Path) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for (key, value) in ISOLATED {
        environment.insert((*key).into(), (*value).into());
    }
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "TMPDIR",
        "GIT_CONFIG_GLOBAL",
    ] {
        environment.insert(
            key.into(),
            root.join(key.to_ascii_lowercase()).display().to_string(),
        );
    }
    if let Some(path) = std::env::var_os("PATH") {
        environment.insert("PATH".into(), path.to_string_lossy().into_owned());
    }
    environment
}

pub fn parse_contract_request<I, S>(values: I) -> Result<ContractRequest, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let values = values
        .into_iter()
        .map(|value| value.as_ref().to_owned())
        .collect::<Vec<_>>();
    if values.first().map(String::as_str) != Some("contract-test") {
        return Err("invalid_command".into());
    }
    let case = values
        .get(1)
        .cloned()
        .ok_or_else(|| "invalid_command".to_owned())?;
    if !matches!(
        case.as_str(),
        "version-pin"
            | "stealth-init"
            | "stale-base-fence"
            | "push-conflict-recovery"
            | "transport-retries"
            | "hermetic"
            | "transport"
            | "all"
            | "document-mapping"
            | "shadow-parity"
            | "local-reads"
            | "freshness"
            | "combined-freshness"
            | "online-sync"
    ) {
        return Err("invalid_command".into());
    }
    let mut archive = None;
    let mut binary = None;
    let mut source_ref = None;
    let mut index = 2;
    while index < values.len() {
        let flag = values
            .get(index)
            .ok_or_else(|| "invalid_command".to_owned())?;
        let value = values
            .get(index + 1)
            .ok_or_else(|| "invalid_command".to_owned())?;
        if value.trim().is_empty() || value.starts_with("--") {
            return Err("invalid_command".into());
        }
        match flag.as_str() {
            "--archive" if archive.is_none() => archive = Some(PathBuf::from(value)),
            "--bd" if binary.is_none() => binary = Some(PathBuf::from(value)),
            "--source-ref"
                if source_ref.is_none()
                    && matches!(
                        case.as_str(),
                        "all"
                            | "document-mapping"
                            | "shadow-parity"
                            | "local-reads"
                            | "freshness"
                            | "combined-freshness"
                            | "online-sync"
                    ) =>
            {
                source_ref = Some(value.to_owned())
            }
            _ => return Err("invalid_command".into()),
        };
        index += 2;
    }
    let source_ref = match case.as_str() {
        "document-mapping" | "shadow-parity" | "local-reads" | "freshness"
        | "combined-freshness" | "online-sync" => {
            source_ref.ok_or_else(|| "invalid_command".to_owned())?
        }
        "all" => source_ref.unwrap_or_else(|| "origin/main".into()),
        _ if source_ref.is_none() => String::new(),
        _ => return Err("invalid_command".into()),
    };
    Ok(ContractRequest {
        case,
        source_ref: (!source_ref.is_empty()).then_some(source_ref),
        archive: archive.ok_or_else(|| "invalid_command".to_owned())?,
        binary: binary.ok_or_else(|| "invalid_command".to_owned())?,
    })
}

fn production_command(
    program: &str,
    argv: Vec<String>,
    redacted_argv_positions: Vec<usize>,
) -> CommandSpec {
    let root = PathBuf::from("/contract-isolated");
    CommandSpec {
        program: PathBuf::from(program),
        argv,
        cwd: Some(root.join("repository")),
        environment: isolated_environment(&root),
        redacted_argv_positions,
    }
}

pub fn observe_command() -> CommandSpec {
    production_command(
        "git",
        vec![
            "ls-remote".into(),
            "--exit-code".into(),
            "origin".into(),
            "refs/dolt/data".into(),
        ],
        vec![2],
    )
}
pub fn publish_command() -> CommandSpec {
    production_command(
        "bd",
        vec![
            "--sandbox".into(),
            "dolt".into(),
            "push".into(),
            "--remote".into(),
            "origin".into(),
        ],
        vec![4],
    )
}
pub fn refresh_command() -> CommandSpec {
    production_command(
        "bd",
        vec![
            "--sandbox".into(),
            "dolt".into(),
            "pull".into(),
            "--remote".into(),
            "origin".into(),
        ],
        vec![4],
    )
}
fn replay_command(operation: &str) -> CommandSpec {
    production_command(
        "bd",
        vec![
            "--sandbox".into(),
            "create".into(),
            "--title".into(),
            format!("operation:{operation}"),
            "--description".into(),
            format!("issue:{operation}"),
        ],
        Vec::new(),
    )
}
fn commit_command(operation: &str) -> CommandSpec {
    production_command(
        "bd",
        vec![
            "--sandbox".into(),
            "dolt".into(),
            "commit".into(),
            "-m".into(),
            format!("operation:{operation}"),
        ],
        Vec::new(),
    )
}
fn logical_export_command() -> CommandSpec {
    production_command("bd", vec!["--sandbox".into(), "export".into()], Vec::new())
}
pub fn leased_ref_update(expected: &str, candidate: &str) -> Result<CommandSpec, String> {
    if !sha(expected) || !sha(candidate) {
        return Err("cutover_blocked".into());
    }
    Ok(production_command(
        "git",
        vec![
            "push".into(),
            "origin".into(),
            format!("--force-with-lease=refs/dolt/data:{expected}"),
            format!("{candidate}:refs/dolt/data"),
        ],
        vec![1, 3],
    ))
}

pub fn execute_publication_command<R: CommandRunner>(
    runner: &mut R,
    command: CommandSpec,
    observed_base: &str,
) -> Result<CommandOutput, String> {
    if !sha(observed_base) {
        return Err("cutover_blocked".into());
    }
    let expected_lease = format!("--force-with-lease=refs/dolt/data:{observed_base}");
    for argument in &command.argv {
        if argument == "--force"
            || argument == "-f"
            || argument.starts_with("--force=")
            || (argument.starts_with('+') && argument.ends_with(":refs/dolt/data"))
        {
            return Err("cutover_blocked".into());
        }
        if argument.starts_with("--force-with-lease") && argument != &expected_lease {
            return Err("cutover_blocked".into());
        }
    }
    if command
        .argv
        .iter()
        .any(|argument| argument == &expected_lease)
        && (command.program != Path::new("git")
            || !command
                .argv
                .iter()
                .any(|argument| argument.strip_suffix(":refs/dolt/data").is_some_and(sha)))
    {
        return Err("cutover_blocked".into());
    }
    if command.program == Path::new("git")
        && command.argv.first().map(String::as_str) == Some("push")
        && command
            .argv
            .iter()
            .any(|argument| argument.ends_with(":refs/dolt/data"))
        && !command
            .argv
            .iter()
            .any(|argument| argument == &expected_lease)
    {
        return Err("cutover_blocked".into());
    }
    runner.run(command)
}
pub fn classify_push(stderr: &str) -> PushFailure {
    if stderr.contains("non-fast-forward") || stderr.contains("stale") {
        PushFailure::StaleBase
    } else if stderr.contains("connection")
        || stderr.contains("timed out")
        || stderr.contains("network")
    {
        PushFailure::Transport
    } else {
        PushFailure::Other
    }
}

pub fn publish_candidate<R: CommandRunner>(
    runner: &mut R,
    operation: &str,
) -> Result<Publication, String> {
    let observed_base = observe(runner)?;
    match execute_publication_command(runner, publish_command(), &observed_base) {
        Ok(output) if output.status == 0 => Ok(Publication::Published {
            operation: operation.into(),
            generation: observe(runner)?,
        }),
        Ok(output) if classify_push(&output.stderr) == PushFailure::StaleBase => {
            Ok(Publication::StaleBase {
                operation: operation.into(),
                generation: observe(runner)?,
            })
        }
        Ok(output) if classify_push(&output.stderr) == PushFailure::Transport => {
            Err("transport".into())
        }
        Err(error) if classify_push(&error) == PushFailure::Transport => Err("transport".into()),
        _ => Err("cutover_blocked".into()),
    }
}
pub fn retry_after_transport<R: CommandRunner>(
    runner: &mut R,
    operation: &str,
) -> Result<Publication, String> {
    retry_after_transport_with_base(runner, operation).map(|(_, publication)| publication)
}
fn retry_after_transport_with_base<R: CommandRunner>(
    runner: &mut R,
    operation: &str,
) -> Result<(String, Publication), String> {
    let observed_base = observe(runner)?;
    match execute_publication_command(runner, publish_command(), &observed_base) {
        Err(error) if classify_push(&error) == PushFailure::Transport => {}
        Ok(output) if classify_push(&output.stderr) == PushFailure::Transport => {}
        _ => return Err("cutover_blocked".into()),
    }
    let retry_base = observe(runner)?;
    if retry_base != observed_base {
        return Err("cutover_blocked".into());
    }
    let output = execute_publication_command(runner, publish_command(), &retry_base)
        .map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    let generation = observe(runner)?;
    validate_scripted_history(runner, &[&observed_base, &generation], &[operation])?;
    Ok((
        observed_base,
        Publication::Published {
            operation: operation.into(),
            generation,
        },
    ))
}
pub fn recover_after_lost_response<R: CommandRunner>(
    runner: &mut R,
    operation: &str,
) -> Result<Publication, String> {
    recover_after_lost_response_with_base(runner, operation).map(|(_, publication)| publication)
}
fn recover_after_lost_response_with_base<R: CommandRunner>(
    runner: &mut R,
    operation: &str,
) -> Result<(String, Publication), String> {
    let observed_base = observe(runner)?;
    match execute_publication_command(runner, publish_command(), &observed_base) {
        Err(error) if classify_push(&error) == PushFailure::Transport => {}
        Ok(output) if classify_push(&output.stderr) == PushFailure::Transport => {}
        _ => return Err("cutover_blocked".into()),
    }
    let observed_after_failure = observe(runner)?;
    if observed_after_failure != observed_base {
        validate_scripted_history(
            runner,
            &[&observed_base, &observed_after_failure],
            &[operation],
        )?;
        return Ok((
            observed_base,
            Publication::Recovered {
                operation: operation.into(),
                generation: observed_after_failure,
            },
        ));
    }
    let output = execute_publication_command(runner, publish_command(), &observed_base)
        .map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    let generation = observe(runner)?;
    validate_scripted_history(runner, &[&observed_base, &generation], &[operation])?;
    Ok((
        observed_base,
        Publication::Published {
            operation: operation.into(),
            generation,
        },
    ))
}
fn observe<R: CommandRunner>(runner: &mut R) -> Result<String, String> {
    let output = runner
        .run(observe_command())
        .map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    let mut lines = output.stdout.lines();
    let Some(line) = lines.next() else {
        return Err("cutover_blocked".into());
    };
    if lines.next().is_some() {
        return Err("cutover_blocked".into());
    }
    let mut fields = line.split_whitespace();
    let generation = fields.next().unwrap_or("");
    if !sha(generation) || fields.next() != Some("refs/dolt/data") || fields.next().is_some() {
        return Err("cutover_blocked".into());
    }
    Ok(generation.into())
}
fn sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn replay_operation<R: CommandRunner>(runner: &mut R, operation: &str) -> Result<(), String> {
    for command in [replay_command(operation), commit_command(operation)] {
        let output = runner
            .run(command)
            .map_err(|_| "cutover_blocked".to_owned())?;
        if output.status != 0 {
            return Err("cutover_blocked".into());
        }
    }
    Ok(())
}

fn validate_observed_export<R: CommandRunner>(
    runner: &mut R,
    expected_operations: &[&str],
) -> Result<(), String> {
    let output = runner
        .run(logical_export_command())
        .map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    validate_logical_export(&output.stdout, expected_operations).map_err(str::to_owned)
}

#[derive(Clone, Debug)]
struct MigrationEvidence {
    source_commit: String,
    document_counts: DocumentCounts,
    total_document_count: usize,
    logical_export_sha256: String,
    clone_labels: Vec<String>,
    command_plans: Vec<String>,
}

const HISTORICAL_SOURCE_COMMIT: &str = "13c0f68c13743f4db2fb123fef560f3fa12734d1";

fn document_counts(documents: &[ShadowDocument]) -> DocumentCounts {
    let mut counts = DocumentCounts {
        intent: 0,
        spec: 0,
        task: 0,
    };
    for document in documents {
        match document.record.kind {
            DocumentKind::Intent => counts.intent += 1,
            DocumentKind::Spec => counts.spec += 1,
            DocumentKind::Task => counts.task += 1,
        }
    }
    counts
}

fn source_result(case: &str, source_ref: &str, source: &SourceDocuments) -> ContractResult {
    let counts = document_counts(&source.documents);
    let total_document_count = counts.intent + counts.spec + counts.task;
    let mut result = ContractResult::passed(case, Vec::new());
    result.source_ref = Some(source_ref.into());
    result.source_commit = Some(source.source_commit.clone());
    result.document_counts = Some(counts);
    result.total_document_count = Some(total_document_count);
    result.authority_mode = Some("markdown-shadow".into());
    result.command_plans = source.command_plans.clone();
    result
}

fn source_refusal(
    case: &str,
    source_ref: &str,
    code: &str,
    offending_key: Option<String>,
    mismatch: Option<String>,
) -> ContractResult {
    let code = if code == "source_ref_unavailable" {
        "invalid_source_ref"
    } else {
        code
    };
    let mut result = ContractResult::refusal(case, code);
    result.source_ref = Some(source_ref.into());
    result.offending_key = offending_key;
    result.mismatch = mismatch;
    result
}

fn snapshot_refusal(
    case: &str,
    source_ref: &str,
    source: &SourceDocuments,
    code: &str,
    offending_key: Option<String>,
    mismatch: Option<String>,
) -> ContractResult {
    let mut result = source_result(case, source_ref, source);
    result.outcome = "refused".into();
    result.code = code.into();
    result.offending_key = offending_key;
    result.mismatch = mismatch;
    result
}

fn assert_historical_task_count(source: &SourceDocuments) -> Result<(), &'static str> {
    if source.source_commit == HISTORICAL_SOURCE_COMMIT
        && document_counts(&source.documents).task != 39
    {
        return Err("document_mapping_mismatch");
    }
    Ok(())
}

fn shadow_store(fixture: &StoreFixture, binary: &Path) -> ShadowStore {
    ShadowStore::new(
        fixture.label.clone(),
        fixture.clone_root.clone(),
        fixture.repository.clone(),
        fixture.environment.clone(),
        binary.to_path_buf(),
    )
}

fn run_shadow_round_trip<R: CommandRunner>(
    runner: &mut R,
    binary: &Path,
    source: &SourceDocuments,
    first: &StoreFixture,
    second: &StoreFixture,
) -> Result<MigrationEvidence, ShadowError> {
    let first_import = import_shadow_documents(
        runner,
        &shadow_store(first, binary),
        &source.source_commit,
        &source.documents,
    )?;
    let logical_export = canonical_logical_export(&first_import.documents)?;
    let logical_documents = decode_logical_export(&logical_export)?;
    let second_import = import_shadow_documents(
        runner,
        &shadow_store(second, binary),
        &source.source_commit,
        &logical_documents,
    )?;
    for documents in [
        first_import.documents.as_slice(),
        logical_documents.as_slice(),
        second_import.documents.as_slice(),
    ] {
        compare_document_mapping(&source.documents, documents)?;
        compare_shadow_parity(&source.documents, documents)?;
    }
    let mut command_plans = source.command_plans.clone();
    command_plans.extend(first_import.command_plans);
    command_plans.extend(second_import.command_plans);
    let document_counts = document_counts(&source.documents);
    Ok(MigrationEvidence {
        source_commit: source.source_commit.clone(),
        total_document_count: document_counts.intent + document_counts.spec + document_counts.task,
        document_counts,
        logical_export_sha256: logical_export_digest(&logical_export),
        clone_labels: vec![first.label.clone(), second.label.clone()],
        command_plans,
    })
}

fn migration_result(
    case: &str,
    source_ref: &str,
    source: &SourceDocuments,
    evidence: MigrationEvidence,
) -> ContractResult {
    let mut result = source_result(case, source_ref, source);
    result.source_commit = Some(evidence.source_commit);
    result.document_counts = Some(evidence.document_counts);
    result.total_document_count = Some(evidence.total_document_count);
    result.logical_export_sha256 = Some(evidence.logical_export_sha256);
    result.clone_labels = evidence.clone_labels;
    result.command_plans = evidence.command_plans;
    result
}

type TreeSnapshot = BTreeMap<PathBuf, TreeEntry>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct TreeEntry {
    directory: bool,
    mode: u32,
    modified: SystemTime,
    digest: Option<String>,
}

#[derive(Debug)]
struct MirrorFixture {
    mirror: PathBuf,
    first_worktree: PathBuf,
    second_worktree: PathBuf,
    other_worktree: PathBuf,
    command_plans: Vec<String>,
}

#[derive(Debug)]
struct LocalReadEvidence {
    clone_labels: Vec<String>,
    source_commit: String,
    local_generation: String,
    operation_ids: Vec<String>,
    command_plans: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ReadBoundary {
    first_worktree: TreeSnapshot,
    second_worktree: TreeSnapshot,
    first_git: TreeSnapshot,
    second_git: TreeSnapshot,
    shared_state: TreeSnapshot,
    mirror_config: TreeEntry,
    mirror_hooks: TreeSnapshot,
}

fn tree_mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        metadata.mode()
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0
    }
}

fn regular_file_digest(path: &Path) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "cutover_blocked".to_owned())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("cutover_blocked".into());
    }
    let mut file = File::open(path).map_err(|_| "cutover_blocked".to_owned())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|_| "cutover_blocked".to_owned())?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn snapshot_regular_tree(root: &Path) -> Result<TreeSnapshot, String> {
    snapshot_tree(root, false)
}

fn snapshot_source_tree(root: &Path) -> Result<TreeSnapshot, String> {
    snapshot_tree(root, true)
}

fn snapshot_tree(root: &Path, allow_symlinks: bool) -> Result<TreeSnapshot, String> {
    let metadata = fs::symlink_metadata(root).map_err(|_| "cutover_blocked".to_owned())?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("cutover_blocked".into());
    }
    let mut entries = BTreeMap::new();
    snapshot_tree_at(root, Path::new(""), allow_symlinks, &mut entries)?;
    Ok(entries)
}

fn snapshot_tree_at(
    root: &Path,
    relative: &Path,
    allow_symlinks: bool,
    entries: &mut TreeSnapshot,
) -> Result<(), String> {
    let directory = if relative.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    let mut children = fs::read_dir(&directory)
        .map_err(|_| "cutover_blocked".to_owned())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "cutover_blocked".to_owned())?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        let child_relative = relative.join(child.file_name());
        let metadata = fs::symlink_metadata(&path).map_err(|_| "cutover_blocked".to_owned())?;
        if metadata.file_type().is_symlink() {
            if !allow_symlinks {
                return Err("cutover_blocked".into());
            }
            let target = fs::read_link(&path).map_err(|_| "cutover_blocked".to_owned())?;
            entries.insert(
                child_relative,
                TreeEntry {
                    directory: false,
                    mode: tree_mode(&metadata),
                    modified: metadata
                        .modified()
                        .map_err(|_| "cutover_blocked".to_owned())?,
                    digest: Some(format!("symlink:{}", target.display())),
                },
            );
            continue;
        }
        let entry = TreeEntry {
            directory: metadata.file_type().is_dir(),
            mode: tree_mode(&metadata),
            modified: metadata
                .modified()
                .map_err(|_| "cutover_blocked".to_owned())?,
            digest: metadata
                .file_type()
                .is_file()
                .then(|| regular_file_digest(&path))
                .transpose()?,
        };
        if !entry.directory && entry.digest.is_none() {
            return Err("cutover_blocked".into());
        }
        entries.insert(child_relative.clone(), entry);
        if metadata.file_type().is_dir() {
            snapshot_tree_at(root, &child_relative, allow_symlinks, entries)?;
        }
    }
    Ok(())
}

fn snapshot_regular_file(path: &Path) -> Result<TreeEntry, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "cutover_blocked".to_owned())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("cutover_blocked".into());
    }
    Ok(TreeEntry {
        directory: false,
        mode: tree_mode(&metadata),
        modified: metadata
            .modified()
            .map_err(|_| "cutover_blocked".to_owned())?,
        digest: Some(regular_file_digest(path)?),
    })
}

fn copy_regular_tree_contents(
    source: &Path,
    destination: &Path,
    omitted_root_file: Option<&str>,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|_| "cutover_blocked".to_owned())?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("cutover_blocked".into());
    }
    let mut entries = fs::read_dir(source)
        .map_err(|_| "cutover_blocked".to_owned())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "cutover_blocked".to_owned())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        if omitted_root_file.is_some_and(|omitted| name == omitted) {
            continue;
        }
        let source_entry = entry.path();
        let destination_entry = destination.join(&name);
        let metadata =
            fs::symlink_metadata(&source_entry).map_err(|_| "cutover_blocked".to_owned())?;
        if metadata.file_type().is_symlink() {
            return Err("cutover_blocked".into());
        }
        if metadata.file_type().is_dir() {
            fs::create_dir(&destination_entry).map_err(|_| "cutover_blocked".to_owned())?;
            fs::set_permissions(&destination_entry, metadata.permissions())
                .map_err(|_| "cutover_blocked".to_owned())?;
            copy_regular_tree_contents(&source_entry, &destination_entry, None)?;
        } else if metadata.file_type().is_file() {
            fs::copy(&source_entry, &destination_entry)
                .map_err(|_| "cutover_blocked".to_owned())?;
            fs::set_permissions(&destination_entry, metadata.permissions())
                .map_err(|_| "cutover_blocked".to_owned())?;
            File::open(&destination_entry)
                .and_then(|file| file.sync_all())
                .map_err(|_| "cutover_blocked".to_owned())?;
        } else {
            return Err("cutover_blocked".into());
        }
    }
    Ok(())
}

fn contract_command(
    program: impl Into<PathBuf>,
    argv: Vec<String>,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> CommandSpec {
    CommandSpec {
        program: program.into(),
        argv,
        cwd: Some(cwd.to_path_buf()),
        environment: environment.clone(),
        redacted_argv_positions: Vec::new(),
    }
}

fn run_contract_command(
    runner: &mut SystemCommandRunner,
    command: CommandSpec,
) -> Result<CommandOutput, String> {
    let output = runner
        .run(command)
        .map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    Ok(output)
}

fn copy_contract_launcher(worktree: &Path) -> Result<(), String> {
    let source = repository_root().join("tools/work-state");
    let destination = worktree.join("tools/work-state");
    let source_metadata =
        fs::symlink_metadata(&source).map_err(|_| "cutover_blocked".to_owned())?;
    if !source_metadata.file_type().is_file() || source_metadata.file_type().is_symlink() {
        return Err("cutover_blocked".into());
    }
    fs::copy(&source, &destination).map_err(|_| "cutover_blocked".to_owned())?;
    fs::set_permissions(&destination, source_metadata.permissions())
        .map_err(|_| "cutover_blocked".to_owned())?;
    if regular_file_digest(&source)? != regular_file_digest(&destination)? {
        return Err("cutover_blocked".into());
    }
    let source_pin = repository_root().join("tools/work-state-beads-1.1.2.toml");
    let destination_pin = worktree.join("tools/work-state-beads-1.1.2.toml");
    let source_pin_metadata =
        fs::symlink_metadata(&source_pin).map_err(|_| "cutover_blocked".to_owned())?;
    if !source_pin_metadata.file_type().is_file() || source_pin_metadata.file_type().is_symlink() {
        return Err("cutover_blocked".into());
    }
    fs::copy(&source_pin, &destination_pin).map_err(|_| "cutover_blocked".to_owned())?;
    fs::set_permissions(&destination_pin, source_pin_metadata.permissions())
        .map_err(|_| "cutover_blocked".to_owned())?;
    if regular_file_digest(&source_pin)? != regular_file_digest(&destination_pin)? {
        return Err("cutover_blocked".into());
    }
    Ok(())
}

fn create_mirror_fixture(
    runner: &mut SystemCommandRunner,
    root: &Path,
    label: &str,
    source_commit: &str,
) -> Result<MirrorFixture, String> {
    let fixture_root = root.join(label);
    fs::create_dir(&fixture_root).map_err(|_| "cutover_blocked".to_owned())?;
    let environment = isolated_environment(&fixture_root);
    let source = repository_root();
    let mirror = fixture_root.join("mirror");
    let other_mirror = fixture_root.join("other-mirror");
    let first_worktree = fixture_root.join("worktree-a");
    let second_worktree = fixture_root.join("worktree-b");
    let other_worktree = fixture_root.join("other-worktree");
    for destination in [&mirror, &other_mirror] {
        run_contract_command(
            runner,
            contract_command(
                "git",
                vec![
                    "clone".into(),
                    "--mirror".into(),
                    "--no-local".into(),
                    source.display().to_string(),
                    destination.display().to_string(),
                ],
                &fixture_root,
                &environment,
            ),
        )?;
    }
    for (mirror_root, worktree) in [
        (&mirror, &first_worktree),
        (&mirror, &second_worktree),
        (&other_mirror, &other_worktree),
    ] {
        run_contract_command(
            runner,
            contract_command(
                "git",
                vec![
                    "worktree".into(),
                    "add".into(),
                    "--detach".into(),
                    worktree.display().to_string(),
                    source_commit.into(),
                ],
                mirror_root,
                &environment,
            ),
        )?;
    }
    copy_contract_launcher(&first_worktree)?;
    copy_contract_launcher(&second_worktree)?;
    Ok(MirrorFixture {
        mirror,
        first_worktree,
        second_worktree,
        other_worktree,
        command_plans: vec![
            "git clone --mirror --no-local".into(),
            "git worktree add --detach".into(),
        ],
    })
}

fn worktree_git_directory(worktree: &Path) -> Result<PathBuf, String> {
    let dot_git = worktree.join(".git");
    let metadata = fs::symlink_metadata(&dot_git).map_err(|_| "cutover_blocked".to_owned())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("cutover_blocked".into());
    }
    let contents = fs::read_to_string(dot_git).map_err(|_| "cutover_blocked".to_owned())?;
    let path = contents
        .strip_prefix("gitdir: ")
        .and_then(|value| value.strip_suffix('\n'))
        .filter(|value| !value.is_empty() && !value.contains(['\n', '\r']))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "cutover_blocked".to_owned())?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| "cutover_blocked".to_owned())?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("cutover_blocked".into());
    }
    fs::canonicalize(path).map_err(|_| "cutover_blocked".to_owned())
}

fn capture_read_boundary(
    fixture: &MirrorFixture,
    shared_state: &Path,
) -> Result<ReadBoundary, String> {
    Ok(ReadBoundary {
        first_worktree: snapshot_source_tree(&fixture.first_worktree)?,
        second_worktree: snapshot_source_tree(&fixture.second_worktree)?,
        first_git: snapshot_regular_tree(&worktree_git_directory(&fixture.first_worktree)?)?,
        second_git: snapshot_regular_tree(&worktree_git_directory(&fixture.second_worktree)?)?,
        shared_state: snapshot_regular_tree(shared_state)?,
        mirror_config: snapshot_regular_file(&fixture.mirror.join("config"))?,
        mirror_hooks: snapshot_regular_tree(&fixture.mirror.join("hooks"))?,
    })
}

fn launcher_environment() -> BTreeMap<String, String> {
    BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())])
}

fn read_arguments(command: &ReadCommand, json: bool) -> Vec<String> {
    let mut arguments = match command {
        ReadCommand::List => vec!["list".into()],
        ReadCommand::Show(key) => vec!["show".into(), key.clone()],
        ReadCommand::Ready => vec!["ready".into()],
        ReadCommand::Blocked => vec!["blocked".into()],
    };
    if json {
        arguments.push("--json".into());
    }
    arguments
}

fn read_command_name(command: &ReadCommand) -> &'static str {
    match command {
        ReadCommand::List => "list",
        ReadCommand::Show(_) => "show",
        ReadCommand::Ready => "ready",
        ReadCommand::Blocked => "blocked",
    }
}

fn run_launcher_read(
    runner: &mut SystemCommandRunner,
    worktree: &Path,
    command: &ReadCommand,
    json: bool,
) -> Result<String, String> {
    let launcher = worktree.join("tools/work-state");
    let metadata = fs::symlink_metadata(&launcher).map_err(|_| "cutover_blocked".to_owned())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("cutover_blocked".into());
    }
    let output = run_contract_command(
        runner,
        contract_command(
            launcher,
            read_arguments(command, json),
            worktree,
            &launcher_environment(),
        ),
    )?;
    if !output.stderr.is_empty() || !output.stdout.ends_with('\n') {
        return Err("cutover_blocked".into());
    }
    Ok(output.stdout)
}

fn expected_read_response(
    command: ReadCommand,
    generation: &CurrentGeneration,
    snapshot: &crate::store::FencedSnapshot,
) -> Result<(serde_json::Value, String), String> {
    let response = project_read(
        command,
        snapshot,
        &generation.manifest.authority_mode,
        &generation.manifest.source_commit,
    )
    .map_err(|_| "cutover_blocked".to_owned())?;
    let value = serde_json::to_value(&response).map_err(|_| "cutover_blocked".to_owned())?;
    Ok((value, render_human(&response)))
}

fn assert_launcher_read(
    runner: &mut SystemCommandRunner,
    worktree: &Path,
    command: ReadCommand,
    generation: &CurrentGeneration,
    snapshot: &crate::store::FencedSnapshot,
) -> Result<(), String> {
    let (expected_json, expected_human) =
        expected_read_response(command.clone(), generation, snapshot)?;
    let json = run_launcher_read(runner, worktree, &command, true)?;
    let observed_json: serde_json::Value =
        serde_json::from_str(&json).map_err(|_| "cutover_blocked".to_owned())?;
    if observed_json != expected_json {
        return Err("cutover_blocked".into());
    }
    let human = run_launcher_read(runner, worktree, &command, false)?;
    if human != expected_human {
        return Err("cutover_blocked".into());
    }
    Ok(())
}

fn assert_launcher_missing_show(
    runner: &mut SystemCommandRunner,
    worktree: &Path,
) -> Result<(), String> {
    let launcher = worktree.join("tools/work-state");
    for json in [true, false] {
        let output = runner
            .run(contract_command(
                &launcher,
                read_arguments(&ReadCommand::Show("task:999".into()), json),
                worktree,
                &launcher_environment(),
            ))
            .map_err(|_| "cutover_blocked".to_owned())?;
        let expected_stderr = if json {
            "{\"code\":\"document_not_found\",\"document_key\":\"task:999\"}\n"
        } else {
            "error[document_not_found]: document_not_found (task:999)\n"
        };
        if output.status != 1 || !output.stdout.is_empty() || output.stderr != expected_stderr {
            return Err("cutover_blocked".into());
        }
    }
    Ok(())
}

fn assert_launcher_read_refusal(
    runner: &mut SystemCommandRunner,
    fixture: &MirrorFixture,
    location: &crate::store::StoreLocation,
    worktree: &Path,
    code: &str,
) -> Result<(), String> {
    let boundary = capture_read_boundary(fixture, &location.state_root)?;
    let launcher = worktree.join("tools/work-state");
    for json in [true, false] {
        let output = runner
            .run(contract_command(
                &launcher,
                read_arguments(&ReadCommand::List, json),
                worktree,
                &launcher_environment(),
            ))
            .map_err(|_| "cutover_blocked".to_owned())?;
        let expected_stderr = if json {
            format!("{{\"code\":\"{code}\"}}\n")
        } else {
            format!("error[{code}]: {code}\n")
        };
        if output.status != 1 || !output.stdout.is_empty() || output.stderr != expected_stderr {
            return Err("cutover_blocked".into());
        }
    }
    if capture_read_boundary(fixture, &location.state_root)? != boundary {
        return Err("cutover_blocked".into());
    }
    Ok(())
}

fn read_commands_for(snapshot: &crate::store::FencedSnapshot) -> Result<Vec<ReadCommand>, String> {
    let show_key = snapshot
        .documents
        .iter()
        .find(|document| matches!(document.document.record.kind, DocumentKind::Task))
        .or_else(|| snapshot.documents.first())
        .map(|document| document.document.record.document_key.clone())
        .ok_or_else(|| "cutover_blocked".to_owned())?;
    Ok(vec![
        ReadCommand::List,
        ReadCommand::Show(show_key),
        ReadCommand::Ready,
        ReadCommand::Blocked,
    ])
}

fn assert_exact_source_projection(
    source: &SourceDocuments,
    snapshot: &crate::store::FencedSnapshot,
) -> Result<(), String> {
    let documents = snapshot
        .documents
        .iter()
        .map(|document| document.document.clone())
        .collect::<Vec<_>>();
    compare_document_mapping(&source.documents, &documents)
        .map_err(|_| "cutover_blocked".to_owned())?;
    compare_shadow_parity(&source.documents, &documents).map_err(|_| "cutover_blocked".to_owned())
}

fn bootstrap_fixture(
    runner: &mut SystemCommandRunner,
    root: &Path,
    label: &str,
    request: &ContractRequest,
    source: &SourceDocuments,
    pin: &PinManifest,
) -> Result<
    (
        MirrorFixture,
        crate::store::StoreLocation,
        CurrentGeneration,
        BootstrapRequest,
    ),
    String,
> {
    let fixture = create_mirror_fixture(runner, root, label, &source.source_commit)?;
    let environment = locator_environment().map_err(|error| error.code().to_owned())?;
    let first_location = locate_store(runner, &fixture.first_worktree, environment.clone())
        .map_err(|error| error.code().to_owned())?;
    let second_location = locate_store(runner, &fixture.second_worktree, environment.clone())
        .map_err(|error| error.code().to_owned())?;
    let other_location = locate_store(runner, &fixture.other_worktree, environment)
        .map_err(|error| error.code().to_owned())?;
    if first_location.common_dir != second_location.common_dir
        || first_location.state_root != second_location.state_root
        || first_location.state_root == other_location.state_root
    {
        return Err("cutover_blocked".into());
    }
    let wrapper = std::env::current_exe().map_err(|_| "cutover_blocked".to_owned())?;
    let bootstrap_request = BootstrapRequest {
        checkout: fixture.first_worktree.clone(),
        source_root: repository_root(),
        source_ref: request
            .source_ref
            .clone()
            .ok_or_else(|| "invalid_source_ref".to_owned())?,
        archive: request.archive.clone(),
        binary: request.binary.clone(),
        wrapper,
        host_target: host_target().into(),
    };
    let first = bootstrap(&bootstrap_request).map_err(|error| error.code().to_owned())?;
    if first.outcome != BootstrapOutcome::Installed || first.source_commit != source.source_commit {
        return Err("cutover_blocked".into());
    }
    let before_second = snapshot_regular_tree(&first_location.state_root)?;
    let second = bootstrap(&bootstrap_request).map_err(|error| error.code().to_owned())?;
    if second.outcome != BootstrapOutcome::Unchanged
        || second.source_commit != first.source_commit
        || second.local_generation != first.local_generation
        || snapshot_regular_tree(&first_location.state_root)? != before_second
    {
        return Err("cutover_blocked".into());
    }
    let generation =
        current_generation(&first_location).map_err(|error| error.code().to_owned())?;
    let second_generation =
        current_generation(&second_location).map_err(|error| error.code().to_owned())?;
    if generation != second_generation
        || generation.manifest.source_commit != source.source_commit
        || generation.manifest.local_generation != first.local_generation
    {
        return Err("cutover_blocked".into());
    }
    let snapshot = read_disposable_snapshot(runner, &generation, pin, host_target())
        .map_err(|error| error.code().to_owned())?;
    assert_exact_source_projection(source, &snapshot)?;
    Ok((fixture, first_location, generation, bootstrap_request))
}

fn assert_runtime_reinstall(
    runner: &mut SystemCommandRunner,
    location: &crate::store::StoreLocation,
    previous: &CurrentGeneration,
    request: &BootstrapRequest,
    source: &SourceDocuments,
    pin: &PinManifest,
    expected_snapshot: &crate::store::FencedSnapshot,
) -> Result<CurrentGeneration, String> {
    let expected_manifest = previous.manifest.clone();
    let result = bootstrap(request).map_err(|error| error.code().to_owned())?;
    if result.outcome != BootstrapOutcome::Reinstalled
        || result.source_commit != expected_manifest.source_commit
        || result.local_generation != expected_manifest.local_generation
        || result.logical_export_sha256 != expected_manifest.logical_export_sha256
    {
        return Err("cutover_blocked".into());
    }
    let repaired = current_generation(location).map_err(|error| error.code().to_owned())?;
    if repaired.name == previous.name
        || !previous.root.is_dir()
        || repaired.manifest != expected_manifest
        || regular_file_digest(&repaired.root.join("plasmosome-work-state"))?
            != repaired.manifest.wrapper_sha256
    {
        return Err("cutover_blocked".into());
    }
    let repaired_snapshot = read_disposable_snapshot(runner, &repaired, pin, host_target())
        .map_err(|error| error.code().to_owned())?;
    if repaired_snapshot != *expected_snapshot {
        return Err("cutover_blocked".into());
    }
    assert_exact_source_projection(source, &repaired_snapshot)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let wrapper_metadata = fs::symlink_metadata(repaired.root.join("plasmosome-work-state"))
            .map_err(|_| "cutover_blocked".to_owned())?;
        if wrapper_metadata.permissions().mode() & 0o7777 != 0o700 {
            return Err("cutover_blocked".into());
        }
    }
    Ok(repaired)
}

fn assert_runtime_repair(
    runner: &mut SystemCommandRunner,
    fixture: &MirrorFixture,
    location: &crate::store::StoreLocation,
    base: &CurrentGeneration,
    request: &BootstrapRequest,
    source: &SourceDocuments,
    pin: &PinManifest,
) -> Result<(CurrentGeneration, Vec<String>), String> {
    let before_snapshot = read_disposable_snapshot(runner, base, pin, host_target())
        .map_err(|error| error.code().to_owned())?;
    let before_manifest = base.manifest.clone();
    let has_nondefault_operational_state = before_snapshot.documents.iter().any(|document| {
        document.operational.as_ref().is_some_and(|operational| {
            operational.active_owner.is_some() && operational.task_dependencies.len() == 2
        })
    });
    if !has_nondefault_operational_state
        || before_manifest.remote_relation != RemoteRelation::Ahead
        || before_manifest.remote_generation.as_deref() != Some(CONTRACT_REMOTE_GENERATION)
        || before_manifest.remote_observed_at.as_deref() != Some(CONTRACT_OBSERVED_AT)
        || before_manifest.observed_local_generation.as_deref()
            == Some(before_manifest.local_generation.as_str())
        || before_manifest.last_successful_sync_at.is_some()
        || before_manifest.pending_operation_ids != [CONTRACT_PENDING_OPERATION]
    {
        return Err("cutover_blocked".into());
    }
    let installed_binary = base.root.join("bd");
    let missing_binary = base.root.join("bd-removed-for-contract");
    fs::rename(&installed_binary, &missing_binary).map_err(|_| "cutover_blocked".to_owned())?;
    assert_launcher_read_refusal(
        runner,
        fixture,
        location,
        &fixture.first_worktree,
        "installed_beads_missing",
    )?;
    let missing_repaired = assert_runtime_reinstall(
        runner,
        location,
        base,
        request,
        source,
        pin,
        &before_snapshot,
    )?;

    let corrupted_binary = missing_repaired.root.join("bd");
    fs::write(&corrupted_binary, "corrupted installed Beads binary")
        .map_err(|_| "cutover_blocked".to_owned())?;
    assert_launcher_read_refusal(
        runner,
        fixture,
        location,
        &fixture.first_worktree,
        "beads_checksum_mismatch",
    )?;
    let checksum_repaired = assert_runtime_reinstall(
        runner,
        location,
        &missing_repaired,
        request,
        source,
        pin,
        &before_snapshot,
    )?;
    #[cfg(unix)]
    let repaired = {
        use std::os::unix::fs::PermissionsExt;

        let wrapper = checksum_repaired.root.join("plasmosome-work-state");
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o600))
            .map_err(|_| "cutover_blocked".to_owned())?;
        assert_launcher_read_refusal(
            runner,
            fixture,
            location,
            &fixture.first_worktree,
            "invalid_store",
        )?;
        assert_runtime_reinstall(
            runner,
            location,
            &checksum_repaired,
            request,
            source,
            pin,
            &before_snapshot,
        )?
    };
    #[cfg(not(unix))]
    let repaired = checksum_repaired;
    let mut plans = vec![
        "installed Beads missing refusal".into(),
        "bootstrap missing-Beads runtime repair".into(),
        "installed Beads checksum refusal".into(),
        "bootstrap checksum runtime repair".into(),
    ];
    #[cfg(unix)]
    plans.extend([
        "installed wrapper-mode refusal".into(),
        "bootstrap wrapper-mode repair".into(),
    ]);
    plans.extend(exercise_all_local_reads(
        runner, fixture, location, &repaired, source, pin,
    )?);
    Ok((repaired, plans))
}

fn resolve_changed_source_ref<R: CommandRunner>(
    runner: &mut R,
    source_root: &Path,
    selected_source_commit: &str,
) -> Result<String, String> {
    let preferred = if selected_source_commit == HISTORICAL_SOURCE_COMMIT {
        "origin/main"
    } else {
        HISTORICAL_SOURCE_COMMIT
    };
    let mut environment = isolated_environment(source_root);
    environment.insert("GIT_NO_LAZY_FETCH".into(), "1".into());
    environment.insert("GIT_OPTIONAL_LOCKS".into(), "0".into());
    let output = runner
        .run(contract_command(
            "git",
            vec![
                "rev-parse".into(),
                "--verify".into(),
                "--end-of-options".into(),
                format!("{preferred}^{{commit}}"),
            ],
            source_root,
            &environment,
        ))
        .map_err(|_| "cutover_blocked".to_owned())?;
    let resolved = output
        .stdout
        .strip_suffix('\n')
        .filter(|value| !value.contains(['\n', '\r']))
        .filter(|value| is_lower_hex_sha(value))
        .filter(|value| *value != selected_source_commit)
        .map(str::to_owned);
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    resolved.ok_or_else(|| "cutover_blocked".into())
}

fn assert_changed_source_refusal(
    runner: &mut SystemCommandRunner,
    location: &crate::store::StoreLocation,
    request: &BootstrapRequest,
    source: &SourceDocuments,
) -> Result<(), String> {
    let alternate_ref =
        resolve_changed_source_ref(runner, &request.source_root, &source.source_commit)?;
    let mut alternate_request = request.clone();
    alternate_request.source_ref = alternate_ref;
    let before = snapshot_regular_tree(&location.state_root)?;
    let Err(error) = bootstrap(&alternate_request) else {
        return Err("cutover_blocked".into());
    };
    if error.code() != "source_commit_mismatch"
        || snapshot_regular_tree(&location.state_root)? != before
    {
        return Err("cutover_blocked".into());
    }
    Ok(())
}

fn assert_bootstrap_contention(
    runner: &mut SystemCommandRunner,
    location: &crate::store::StoreLocation,
    worktree: &Path,
    request: &BootstrapRequest,
) -> Result<(), String> {
    let held_lock = BootstrapLock::acquire(location).map_err(|error| error.code().to_owned())?;
    let before = snapshot_regular_tree(&location.state_root)?;
    let executable = std::env::current_exe().map_err(|_| "cutover_blocked".to_owned())?;
    let output = runner.run(CommandSpec {
        program: executable,
        argv: vec![
            "bootstrap".into(),
            "--source-ref".into(),
            request.source_ref.clone(),
            "--archive".into(),
            request.archive.display().to_string(),
            "--bd".into(),
            request.binary.display().to_string(),
            "--json".into(),
        ],
        cwd: Some(worktree.to_path_buf()),
        environment: launcher_environment(),
        redacted_argv_positions: Vec::new(),
    });
    drop(held_lock);
    let output = output.map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 1
        || !output.stdout.is_empty()
        || output.stderr != "{\"code\":\"bootstrap_busy\"}\n"
        || snapshot_regular_tree(&location.state_root)? != before
    {
        return Err("cutover_blocked".into());
    }
    Ok(())
}

fn exercise_all_local_reads(
    runner: &mut SystemCommandRunner,
    fixture: &MirrorFixture,
    location: &crate::store::StoreLocation,
    generation: &CurrentGeneration,
    source: &SourceDocuments,
    pin: &PinManifest,
) -> Result<Vec<String>, String> {
    let snapshot = read_disposable_snapshot(runner, generation, pin, host_target())
        .map_err(|error| error.code().to_owned())?;
    assert_exact_source_projection(source, &snapshot)?;
    let boundary = capture_read_boundary(fixture, &location.state_root)?;
    let commands = read_commands_for(&snapshot)?;
    for worktree in [&fixture.first_worktree, &fixture.second_worktree] {
        for command in &commands {
            assert_launcher_read(runner, worktree, command.clone(), generation, &snapshot)?;
        }
        assert_launcher_missing_show(runner, worktree)?;
    }
    if capture_read_boundary(fixture, &location.state_root)? != boundary {
        return Err("cutover_blocked".into());
    }
    let mut plans = Vec::new();
    for command in &commands {
        plans.push(format!(
            "tools/work-state {} --json",
            read_command_name(command)
        ));
        plans.push(format!("tools/work-state {}", read_command_name(command)));
    }
    plans.push("tools/work-state show task:999 refusal".into());
    Ok(plans)
}

const CONTRACT_OBSERVED_AT: &str = "2026-09-02T12:34:56Z";
const CONTRACT_REMOTE_GENERATION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const CONTRACT_MOVED_REMOTE_GENERATION: &str = "cccccccccccccccccccccccccccccccccccccccc";
const CONTRACT_PENDING_OPERATION: &str = "operation-contract-046";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContractParityMismatch {
    Authority,
    Source,
    Logical,
    Operational,
    Missing,
    Extra,
    UnknownKeyValue,
}

type ContractParityProjection = (
    Vec<ShadowDocument>,
    BTreeMap<String, OperationalMetadata>,
    Option<(&'static str, String)>,
);

fn parity_candidate_projection(
    mismatch: ContractParityMismatch,
    documents: &[ShadowDocument],
    operational: &BTreeMap<String, OperationalMetadata>,
) -> Result<ContractParityProjection, String> {
    let mut documents = documents.to_vec();
    let mut operational = operational.clone();
    let mut key_value = None;
    match mismatch {
        ContractParityMismatch::Authority => {
            key_value = Some(("plasmosome.authority-mode", "ledger".into()));
        }
        ContractParityMismatch::Source => {
            key_value = Some(("plasmosome.source-commit", "b".repeat(40)));
        }
        ContractParityMismatch::Logical => {
            let document = documents
                .iter_mut()
                .find(|document| document.record.document_key == "task:001")
                .ok_or_else(|| "cutover_blocked".to_owned())?;
            document.record.title = "Changed remotely".into();
        }
        ContractParityMismatch::Operational => {
            let metadata = operational
                .get_mut("task:001")
                .ok_or_else(|| "cutover_blocked".to_owned())?;
            metadata.active_owner = Some(ActiveOwner {
                actor: "remote-owner".into(),
                session_id: "remote-session".into(),
                ownership_token: "remote-token".into(),
                claim_operation_id: "remote-claim".into(),
                acquired_at: "2026-09-02T12:00:00Z".into(),
                expires_at: "2026-09-02T13:00:00Z".into(),
            });
        }
        ContractParityMismatch::Missing => {
            let index = documents
                .iter()
                .position(|document| document.record.document_key == "task:002")
                .ok_or_else(|| "cutover_blocked".to_owned())?;
            documents.remove(index);
            operational.remove("task:002");
        }
        ContractParityMismatch::Extra => {
            let mut extra = documents
                .iter()
                .find(|document| document.record.document_key == "task:002")
                .cloned()
                .ok_or_else(|| "cutover_blocked".to_owned())?;
            extra.record.document_key = "task:999".into();
            extra.record.document_id = "999".into();
            extra.record.document_path = "tasks/999-extra.md".into();
            extra.record.title = "Extra remote task".into();
            let metadata = operational
                .get("task:002")
                .cloned()
                .ok_or_else(|| "cutover_blocked".to_owned())?;
            operational.insert("task:999".into(), metadata);
            documents.push(extra);
        }
        ContractParityMismatch::UnknownKeyValue => {
            key_value = Some(("plasmosome.writer", "forbidden".into()));
        }
    }
    Ok((documents, operational, key_value))
}

fn contract_generation_name(label: &str) -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "cutover_blocked".to_owned())?
        .as_nanos();
    Ok(format!(
        "generation-contract-{label}-{}-{nanos}",
        std::process::id()
    ))
}

/// Requires a real new embedded generation whenever a fixture reports pending local work.
///
/// This makes the freshness contract evidence distinguish a committed local change from a
/// manifest-only pending-operation marker.
pub fn validate_freshness_fixture_generations(
    base_generation: &str,
    fixture_generation: &str,
    pending_operation_ids: &[&str],
) -> Result<(), &'static str> {
    if pending_operation_ids.is_empty() || base_generation != fixture_generation {
        Ok(())
    } else {
        Err("cutover_blocked")
    }
}

/// Builds the sole real Beads update plan used to create a committed pending-local fixture.
///
/// The full metadata object is deliberate: `--set-metadata` serializes nested JSON as a string,
/// which would corrupt the typed task operational sibling in a subsequent export.
pub fn pending_fixture_update_arguments(
    native_issue_id: &str,
    full_metadata: &str,
) -> Result<Vec<String>, &'static str> {
    if native_issue_id.trim().is_empty() || native_issue_id.contains(['\n', '\r']) {
        return Err("cutover_blocked");
    }
    if !serde_json::from_str::<serde_json::Value>(full_metadata)
        .ok()
        .is_some_and(|metadata| metadata.is_object())
    {
        return Err("cutover_blocked");
    }
    Ok(vec![
        "--sandbox".into(),
        "--dolt-auto-commit=batch".into(),
        "update".into(),
        native_issue_id.into(),
        "--metadata".into(),
        full_metadata.into(),
    ])
}

fn pending_fixture_metadata(
    base_snapshot: &crate::store::FencedSnapshot,
    target: &OperationalDocument,
    replacement: OperationalMetadata,
) -> Result<String, String> {
    let logical_documents = base_snapshot
        .documents
        .iter()
        .map(|document| document.document.clone())
        .collect::<Vec<_>>();
    let mut operational = base_snapshot
        .documents
        .iter()
        .filter_map(|document| {
            document
                .operational
                .as_ref()
                .cloned()
                .map(|metadata| (document.document.record.document_key.clone(), metadata))
        })
        .collect::<BTreeMap<_, _>>();
    operational.insert(target.document.record.document_key.clone(), replacement);
    let encoded = to_operational_beads_jsonl(&logical_documents, &operational)
        .map_err(|_| "cutover_blocked".to_owned())?;
    let native = native_id(&target.document.record);
    let metadata = encoded
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find_map(|row| {
            (row.get("id").and_then(serde_json::Value::as_str) == Some(native.as_str()))
                .then(|| row.get("metadata").cloned())
                .flatten()
        })
        .filter(serde_json::Value::is_object)
        .ok_or_else(|| "cutover_blocked".to_owned())?;
    serde_json::to_string(&metadata).map_err(|_| "cutover_blocked".to_owned())
}

fn fixture_runtime_environment(staging: &Path) -> Result<BTreeMap<String, String>, String> {
    let runtime = staging.join("runtime");
    let directories = [
        ("HOME", runtime.join("home")),
        ("XDG_CONFIG_HOME", runtime.join("xdg_config")),
        ("XDG_CACHE_HOME", runtime.join("xdg_cache")),
        ("XDG_DATA_HOME", runtime.join("xdg_data")),
        ("TMPDIR", runtime.join("tmp")),
    ];
    let mut environment = BTreeMap::new();
    for (key, path) in directories {
        let metadata = fs::symlink_metadata(&path).map_err(|_| "cutover_blocked".to_owned())?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err("cutover_blocked".into());
        }
        environment.insert(key.into(), path.display().to_string());
    }
    let git_config = runtime.join("git_config_global");
    let metadata = fs::symlink_metadata(&git_config).map_err(|_| "cutover_blocked".to_owned())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("cutover_blocked".into());
    }
    environment.insert("GIT_CONFIG_GLOBAL".into(), git_config.display().to_string());
    for (key, value) in [
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_NO_LAZY_FETCH", "1"),
        ("GIT_OPTIONAL_LOCKS", "0"),
        ("BD_DISABLE_METRICS", "1"),
        ("BD_DISABLE_EVENT_FLUSH", "1"),
        ("BD_NON_INTERACTIVE", "1"),
        ("CI", "true"),
    ] {
        environment.insert(key.into(), value.into());
    }
    let path = std::env::var_os("PATH").ok_or_else(|| "cutover_blocked".to_owned())?;
    environment.insert("PATH".into(), path.to_string_lossy().into_owned());
    Ok(environment)
}

fn fixture_status_commit(value: &str) -> Result<String, String> {
    let status: serde_json::Value =
        serde_json::from_str(value).map_err(|_| "cutover_blocked".to_owned())?;
    let commit = status
        .get("commit")
        .and_then(serde_json::Value::as_str)
        .filter(|commit| !commit.trim().is_empty() && *commit == commit.trim())
        .ok_or_else(|| "cutover_blocked".to_owned())?;
    if status
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
        || status.get("branch").and_then(serde_json::Value::as_str) != Some("main")
    {
        return Err("cutover_blocked".into());
    }
    Ok(commit.into())
}

fn commit_pending_fixture_change(
    runner: &mut SystemCommandRunner,
    staging: &Path,
    base_snapshot: &crate::store::FencedSnapshot,
) -> Result<(String, Vec<OperationalDocument>), String> {
    let target = base_snapshot
        .documents
        .iter()
        .find(|document| matches!(document.document.record.kind, DocumentKind::Task))
        .ok_or_else(|| "cutover_blocked".to_owned())?;
    let mut operational: OperationalMetadata = target
        .operational
        .clone()
        .ok_or_else(|| "cutover_blocked".to_owned())?;
    operational.active_owner = Some(ActiveOwner {
        actor: "contract-fixture".into(),
        session_id: "freshness-fixture".into(),
        ownership_token: "fixture-token".into(),
        claim_operation_id: CONTRACT_PENDING_OPERATION.into(),
        acquired_at: CONTRACT_OBSERVED_AT.into(),
        expires_at: "2026-09-02T13:34:56Z".into(),
    });
    operational.task_dependencies = base_snapshot
        .documents
        .iter()
        .filter(|document| {
            matches!(document.document.record.kind, DocumentKind::Task)
                && document.document.record.document_key != target.document.record.document_key
        })
        .map(|document| document.document.record.document_key.clone())
        .take(2)
        .collect();
    if operational.task_dependencies.len() != 2 {
        return Err("cutover_blocked".into());
    }
    let metadata = pending_fixture_metadata(base_snapshot, target, operational)?;
    let update = pending_fixture_update_arguments(&native_id(&target.document.record), &metadata)
        .map_err(str::to_owned)?;
    let repository = staging.join("repository");
    let binary = staging.join("bd");
    let environment = fixture_runtime_environment(staging)?;
    run_contract_command(
        runner,
        contract_command(&binary, update, &repository, &environment),
    )?;
    run_contract_command(
        runner,
        contract_command(
            &binary,
            vec![
                "--sandbox".into(),
                "dolt".into(),
                "commit".into(),
                "-m".into(),
                "contract committed pending local change".into(),
            ],
            &repository,
            &environment,
        ),
    )?;
    let status = run_contract_command(
        runner,
        contract_command(
            &binary,
            vec![
                "--readonly".into(),
                "--sandbox".into(),
                "--json".into(),
                "vc".into(),
                "status".into(),
            ],
            &repository,
            &environment,
        ),
    )?;
    let export = run_contract_command(
        runner,
        contract_command(
            &binary,
            vec!["--readonly".into(), "--sandbox".into(), "export".into()],
            &repository,
            &environment,
        ),
    )?;
    let generation = fixture_status_commit(&status.stdout)?;
    let documents =
        decode_operational_beads_jsonl(&export.stdout).map_err(|_| "cutover_blocked".to_owned())?;
    Ok((generation, documents))
}

fn activate_freshness_fixture(
    runner: &mut SystemCommandRunner,
    location: &crate::store::StoreLocation,
    base: &CurrentGeneration,
    pin: &PinManifest,
    relation: RemoteRelation,
    preserve_remote_observation: bool,
    pending_operation_ids: &[&str],
) -> Result<CurrentGeneration, String> {
    let generation_name = contract_generation_name("freshness")?;
    let staging = location.generations_dir.join(format!(
        ".staging-{}",
        generation_name
            .strip_prefix("generation-")
            .unwrap_or_default()
    ));
    fs::create_dir(&staging).map_err(|_| "cutover_blocked".to_owned())?;
    copy_regular_tree_contents(&base.root, &staging, Some("state.json"))?;
    let mut manifest: StateManifest = base.manifest.clone();
    let observed_local_base = manifest.local_generation.clone();
    if !pending_operation_ids.is_empty() {
        let base_snapshot = read_disposable_snapshot(runner, base, pin, host_target())
            .map_err(|error| error.code().to_owned())?;
        let (local_generation, documents) =
            commit_pending_fixture_change(runner, &staging, &base_snapshot)?;
        validate_freshness_fixture_generations(
            &observed_local_base,
            &local_generation,
            pending_operation_ids,
        )?;
        let logical_documents = documents
            .iter()
            .map(|document| document.document.clone())
            .collect::<Vec<_>>();
        let logical = canonical_logical_export(&logical_documents)
            .map_err(|_| "cutover_blocked".to_owned())?;
        let operational = canonical_operational_projection(&documents)
            .map_err(|_| "cutover_blocked".to_owned())?;
        manifest.local_generation = local_generation;
        manifest.logical_export_sha256 = logical_export_digest(&logical);
        manifest.operational_projection_sha256 = operational_projection_digest(&operational);
    }
    manifest.remote_relation = relation.clone();
    manifest.pending_operation_ids = pending_operation_ids
        .iter()
        .map(|operation| (*operation).to_owned())
        .collect();
    let known_observation =
        !matches!(relation, RemoteRelation::Unknown) || preserve_remote_observation;
    if known_observation {
        manifest.remote_generation = Some(CONTRACT_REMOTE_GENERATION.into());
        manifest.remote_observed_at = Some(CONTRACT_OBSERVED_AT.into());
        manifest.observed_local_generation = Some(if pending_operation_ids.is_empty() {
            manifest.local_generation.clone()
        } else {
            observed_local_base
        });
    } else {
        manifest.remote_generation = None;
        manifest.remote_observed_at = None;
        manifest.observed_local_generation = None;
    }
    manifest.last_successful_sync_at = match relation {
        RemoteRelation::Equivalent => Some(CONTRACT_OBSERVED_AT.into()),
        RemoteRelation::Unknown if preserve_remote_observation => Some(CONTRACT_OBSERVED_AT.into()),
        RemoteRelation::Ahead | RemoteRelation::Unknown => None,
    };
    let contents = serde_json::to_vec(&manifest).map_err(|_| "cutover_blocked".to_owned())?;
    let state_path = staging.join("state.json");
    let mut state = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(state_path)
        .map_err(|_| "cutover_blocked".to_owned())?;
    state
        .write_all(&contents)
        .and_then(|()| state.sync_all())
        .map_err(|_| "cutover_blocked".to_owned())?;
    drop(state);
    activate_staged_generation(location, &staging, &generation_name, None)
        .map_err(|error| error.code().to_owned())?;
    current_generation(location).map_err(|error| error.code().to_owned())
}

fn assert_freshness_projection(
    runner: &mut SystemCommandRunner,
    worktree: &Path,
    generation: &CurrentGeneration,
    pin: &PinManifest,
    expected: Freshness,
) -> Result<(), String> {
    let snapshot = read_disposable_snapshot(runner, generation, pin, host_target())
        .map_err(|error| error.code().to_owned())?;
    let freshness = &snapshot.freshness;
    if freshness.freshness != expected
        || freshness.local_generation != generation.manifest.local_generation
        || freshness.remote_generation != generation.manifest.remote_generation
        || freshness.remote_observed_at != generation.manifest.remote_observed_at
        || freshness.last_successful_sync_at != generation.manifest.last_successful_sync_at
        || freshness.pending_mutations.operation_ids != generation.manifest.pending_operation_ids
    {
        return Err("cutover_blocked".into());
    }
    assert_launcher_read(runner, worktree, ReadCommand::List, generation, &snapshot)?;
    let (_, human) = expected_read_response(ReadCommand::List, generation, &snapshot)?;
    if matches!(expected, Freshness::SynchronizedAsOf)
        && !human.contains(&format!("synchronized as of {CONTRACT_OBSERVED_AT}"))
    {
        return Err("cutover_blocked".into());
    }
    if human.contains("up to date") || human.contains("freshness: current") {
        return Err("cutover_blocked".into());
    }
    Ok(())
}

fn local_read_contract_case(
    runner: &mut SystemCommandRunner,
    root: &Path,
    case: &str,
    request: &ContractRequest,
    source: &SourceDocuments,
    pin: &PinManifest,
) -> Result<LocalReadEvidence, String> {
    let (fixture, location, base, bootstrap_request) =
        bootstrap_fixture(runner, root, case, request, source, pin)?;
    let mut command_plans = fixture.command_plans.clone();
    command_plans.push("bootstrap installed".into());
    command_plans.push("bootstrap unchanged".into());
    let mut operation_ids = Vec::new();
    match case {
        "local-reads" => {
            assert_bootstrap_contention(
                runner,
                &location,
                &fixture.first_worktree,
                &bootstrap_request,
            )?;
            command_plans
                .push("held bootstrap lock refuses a second compiled CLI bootstrap".into());
            command_plans.extend(exercise_all_local_reads(
                runner, &fixture, &location, &base, source, pin,
            )?);
            let repair_candidate = activate_freshness_fixture(
                runner,
                &location,
                &base,
                pin,
                RemoteRelation::Ahead,
                false,
                &[CONTRACT_PENDING_OPERATION],
            )?;
            operation_ids.push(CONTRACT_PENDING_OPERATION.into());
            command_plans.push("committed non-default runtime repair fixture".into());
            let (repaired, repair_plans) = assert_runtime_repair(
                runner,
                &fixture,
                &location,
                &repair_candidate,
                &bootstrap_request,
                source,
                pin,
            )?;
            command_plans.extend(repair_plans);
            assert_changed_source_refusal(runner, &location, &bootstrap_request, source)?;
            command_plans.push("bootstrap changed-source refusal".into());
            return Ok(LocalReadEvidence {
                clone_labels: vec![format!("{case}-worktree-a"), format!("{case}-worktree-b")],
                source_commit: repaired.manifest.source_commit,
                local_generation: repaired.manifest.local_generation,
                operation_ids,
                command_plans,
            });
        }
        "freshness" => {
            assert_freshness_projection(
                runner,
                &fixture.first_worktree,
                &base,
                pin,
                Freshness::Unknown,
            )?;
            for (relation, pending, expected) in [
                (
                    RemoteRelation::Equivalent,
                    Vec::new(),
                    Freshness::SynchronizedAsOf,
                ),
                (RemoteRelation::Ahead, Vec::new(), Freshness::Stale),
                (
                    RemoteRelation::Equivalent,
                    vec![CONTRACT_PENDING_OPERATION],
                    Freshness::Unpublished,
                ),
            ] {
                let generation = activate_freshness_fixture(
                    runner, &location, &base, pin, relation, false, &pending,
                )?;
                assert_freshness_projection(
                    runner,
                    &fixture.first_worktree,
                    &generation,
                    pin,
                    expected,
                )?;
                operation_ids.extend(pending.into_iter().map(str::to_owned));
            }
            command_plans.push("tools/work-state list freshness fixtures".into());
        }
        "combined-freshness" => {
            for relation in [RemoteRelation::Ahead, RemoteRelation::Unknown] {
                let generation = activate_freshness_fixture(
                    runner,
                    &location,
                    &base,
                    pin,
                    relation.clone(),
                    matches!(relation, RemoteRelation::Unknown),
                    &[CONTRACT_PENDING_OPERATION],
                )?;
                let expected = match relation {
                    RemoteRelation::Ahead => Freshness::StaleWithUnpublished,
                    RemoteRelation::Unknown => Freshness::UnknownWithUnpublished,
                    RemoteRelation::Equivalent => return Err("cutover_blocked".into()),
                };
                assert_freshness_projection(
                    runner,
                    &fixture.first_worktree,
                    &generation,
                    pin,
                    expected,
                )?;
                operation_ids.push(CONTRACT_PENDING_OPERATION.into());
            }
            command_plans.push("tools/work-state list combined freshness fixtures".into());
        }
        _ => return Err("invalid_command".into()),
    }
    Ok(LocalReadEvidence {
        clone_labels: vec![format!("{case}-worktree-a"), format!("{case}-worktree-b")],
        source_commit: base.manifest.source_commit,
        local_generation: base.manifest.local_generation,
        operation_ids,
        command_plans,
    })
}

fn local_read_contract_result(
    case: &str,
    source_ref: &str,
    source: &SourceDocuments,
    evidence: LocalReadEvidence,
) -> ContractResult {
    let mut result = source_result(case, source_ref, source);
    result.clone_labels = evidence.clone_labels;
    result.source_commit = Some(evidence.source_commit.clone());
    result.final_generation = Some(evidence.local_generation.clone());
    result.operation_ids = evidence.operation_ids.clone();
    result.command_plans = evidence.command_plans.clone();
    result.scenarios = vec![ScenarioEvidence {
        case: case.into(),
        observed_base: evidence.source_commit,
        final_generation: evidence.local_generation,
        operation_ids: evidence.operation_ids,
        command_plans: evidence.command_plans,
    }];
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnlineSyncContractPhase {
    AwaitFirstObservation,
    AwaitInit,
    AwaitRemoteList,
    AwaitSecondObservation,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnlineSyncContractScenario {
    Stable,
    FirstTransport,
    FirstMalformed,
    FirstMoved,
    InitTransport,
    RemoteListTransport,
    RemoteListMismatch,
    SecondTransport,
    SecondNoMatch,
    SecondMalformed,
    SecondMoved,
}

struct OnlineSyncContractTransport {
    local: SystemCommandRunner,
    project: ProjectConfig,
    remote_candidate: PathBuf,
    remote_generation: String,
    scenario: OnlineSyncContractScenario,
    phase: OnlineSyncContractPhase,
    staging_root: Option<PathBuf>,
    environment: Option<BTreeMap<String, String>>,
    commands: Vec<CommandSpec>,
}

impl OnlineSyncContractTransport {
    fn stable(
        project: ProjectConfig,
        remote_candidate: PathBuf,
        remote_generation: String,
    ) -> Self {
        Self {
            local: SystemCommandRunner,
            project,
            remote_candidate,
            remote_generation,
            scenario: OnlineSyncContractScenario::Stable,
            phase: OnlineSyncContractPhase::AwaitFirstObservation,
            staging_root: None,
            environment: None,
            commands: Vec::new(),
        }
    }

    fn for_scenario(
        project: ProjectConfig,
        remote_candidate: PathBuf,
        scenario: OnlineSyncContractScenario,
    ) -> Self {
        Self {
            local: SystemCommandRunner,
            project,
            remote_candidate,
            remote_generation: CONTRACT_REMOTE_GENERATION.into(),
            scenario,
            phase: OnlineSyncContractPhase::AwaitFirstObservation,
            staging_root: None,
            environment: None,
            commands: Vec::new(),
        }
    }

    fn is_first_observation(&self, command: &CommandSpec) -> bool {
        command.program == Path::new("git")
            && command.argv
                == [
                    "ls-remote",
                    "--exit-code",
                    self.project.git_observation_url(),
                    self.project.data_ref(),
                ]
            && command
                .cwd
                .as_ref()
                .is_some_and(|root| root.is_absolute() && root.file_name().is_some())
            && !command.environment.is_empty()
            && command.redacted_argv_positions == [2]
    }

    fn is_init(&self, command: &CommandSpec) -> bool {
        let Some(staging_root) = self.staging_root.as_deref() else {
            return false;
        };
        command.program == staging_root.join("bd")
            && command.argv
                == [
                    "--sandbox",
                    "init",
                    "--remote",
                    self.project.dolt_remote_url(),
                    "--stealth",
                    "--skip-agents",
                    "--skip-hooks",
                    "--non-interactive",
                ]
            && command.cwd.as_deref() == Some(staging_root.join("repository").as_path())
            && self.environment.as_ref() == Some(&command.environment)
            && command.redacted_argv_positions == [3]
    }

    fn is_remote_list(&self, command: &CommandSpec) -> bool {
        let Some(staging_root) = self.staging_root.as_deref() else {
            return false;
        };
        command.program == staging_root.join("bd")
            && command.argv == ["--sandbox", "--json", "dolt", "remote", "list"]
            && command.cwd.as_deref() == Some(staging_root.join("repository").as_path())
            && self.environment.as_ref() == Some(&command.environment)
            && command.redacted_argv_positions.is_empty()
    }

    fn is_second_observation(&self, command: &CommandSpec) -> bool {
        let Some(staging_root) = self.staging_root.as_deref() else {
            return false;
        };
        command.program == Path::new("git")
            && command.argv
                == [
                    "ls-remote",
                    "--exit-code",
                    self.project.git_observation_url(),
                    self.project.data_ref(),
                ]
            && command.cwd.as_deref() == Some(staging_root)
            && self.environment.as_ref() == Some(&command.environment)
            && command.redacted_argv_positions == [2]
    }

    fn is_metadata_version_transition(&self, command: &CommandSpec) -> bool {
        let Some(staging_root) = self.staging_root.as_deref() else {
            return false;
        };
        let Some(generations) = staging_root.parent() else {
            return false;
        };
        let Some(metadata_stage) = command.program.parent() else {
            return false;
        };
        command.program.file_name().is_some_and(|name| name == "bd")
            && metadata_stage.parent() == Some(generations)
            && metadata_stage
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".staging-"))
            && command.argv == ["--version"]
            && command.cwd.is_none()
            && command.redacted_argv_positions.is_empty()
    }

    fn remote_list(&self) -> String {
        format!(
            "[{{\"name\":\"{}\",\"url\":\"{}\",\"sql_url\":\"{}\",\"status\":\"ok\"}}]",
            self.project.remote_name(),
            self.project.dolt_remote_url(),
            self.project.dolt_remote_url(),
        )
    }

    fn materialize_candidate(&self, repository: &Path) -> Result<(), String> {
        if fs::read_dir(repository)
            .map_err(|_| "cutover_blocked".to_owned())?
            .next()
            .is_some()
        {
            return Err("cutover_blocked".into());
        }
        copy_regular_tree_contents(&self.remote_candidate, repository, None)
    }

    fn observation_output(&self, generation: &str) -> CommandOutput {
        CommandOutput::success(format!("{generation}\trefs/dolt/data\n"))
    }
}

impl CommandRunner for OnlineSyncContractTransport {
    fn run(&mut self, command: CommandSpec) -> Result<CommandOutput, String> {
        match self.phase {
            OnlineSyncContractPhase::AwaitFirstObservation
                if command.program == Path::new("git") =>
            {
                if !self.is_first_observation(&command) {
                    return Err("cutover_blocked".into());
                }
                self.staging_root = command.cwd.clone();
                self.environment = Some(command.environment.clone());
                self.commands.push(command);
                match self.scenario {
                    OnlineSyncContractScenario::FirstTransport => {
                        self.phase = OnlineSyncContractPhase::Complete;
                        Ok(CommandOutput {
                            status: 1,
                            stdout: String::new(),
                            stderr: "recorded first-observation transport failure".into(),
                        })
                    }
                    OnlineSyncContractScenario::FirstMalformed => {
                        self.phase = OnlineSyncContractPhase::Complete;
                        Ok(CommandOutput::success("recorded malformed observation\n"))
                    }
                    OnlineSyncContractScenario::FirstMoved => {
                        self.phase = OnlineSyncContractPhase::AwaitInit;
                        Ok(self.observation_output(CONTRACT_MOVED_REMOTE_GENERATION))
                    }
                    _ => {
                        self.phase = OnlineSyncContractPhase::AwaitInit;
                        Ok(self.observation_output(&self.remote_generation))
                    }
                }
            }
            OnlineSyncContractPhase::AwaitFirstObservation => self.local.run(command),
            OnlineSyncContractPhase::AwaitInit => {
                if self.is_metadata_version_transition(&command) {
                    self.phase = OnlineSyncContractPhase::Complete;
                    return self.local.run(command);
                }
                if !self.is_init(&command) {
                    return Err("cutover_blocked".into());
                }
                let repository = command
                    .cwd
                    .clone()
                    .ok_or_else(|| "cutover_blocked".to_owned())?;
                self.commands.push(command);
                if self.scenario == OnlineSyncContractScenario::InitTransport {
                    self.phase = OnlineSyncContractPhase::Complete;
                    return Ok(CommandOutput {
                        status: 1,
                        stdout: String::new(),
                        stderr: "recorded init transport failure".into(),
                    });
                }
                self.materialize_candidate(&repository)?;
                self.phase = OnlineSyncContractPhase::AwaitRemoteList;
                Ok(CommandOutput::success(""))
            }
            OnlineSyncContractPhase::AwaitRemoteList => {
                if !self.is_remote_list(&command) {
                    return Err("cutover_blocked".into());
                }
                self.commands.push(command);
                match self.scenario {
                    OnlineSyncContractScenario::RemoteListTransport => {
                        self.phase = OnlineSyncContractPhase::Complete;
                        Ok(CommandOutput {
                            status: 1,
                            stdout: String::new(),
                            stderr: "recorded remote-list transport failure".into(),
                        })
                    }
                    OnlineSyncContractScenario::RemoteListMismatch => {
                        self.phase = OnlineSyncContractPhase::Complete;
                        Ok(CommandOutput::success("[]"))
                    }
                    _ => {
                        self.phase = OnlineSyncContractPhase::AwaitSecondObservation;
                        Ok(CommandOutput::success(self.remote_list()))
                    }
                }
            }
            OnlineSyncContractPhase::AwaitSecondObservation => {
                if !self.is_second_observation(&command) {
                    return Err("cutover_blocked".into());
                }
                self.phase = OnlineSyncContractPhase::Complete;
                self.commands.push(command);
                match self.scenario {
                    OnlineSyncContractScenario::SecondTransport => Ok(CommandOutput {
                        status: 1,
                        stdout: String::new(),
                        stderr: "recorded second-observation transport failure".into(),
                    }),
                    OnlineSyncContractScenario::SecondNoMatch => Ok(CommandOutput {
                        status: 2,
                        stdout: String::new(),
                        stderr: "recorded second-observation no-match".into(),
                    }),
                    OnlineSyncContractScenario::SecondMalformed => {
                        Ok(CommandOutput::success("recorded malformed observation\n"))
                    }
                    OnlineSyncContractScenario::SecondMoved => {
                        Ok(self.observation_output(CONTRACT_MOVED_REMOTE_GENERATION))
                    }
                    _ => Ok(self.observation_output(&self.remote_generation)),
                }
            }
            OnlineSyncContractPhase::Complete if command.program == Path::new("git") => {
                Err("cutover_blocked".into())
            }
            OnlineSyncContractPhase::Complete => self.local.run(command),
        }
    }
}

fn assert_recorded_remote_failure_inventory(
    root: &Path,
    location: &crate::store::StoreLocation,
    selected: &CurrentGeneration,
    pin: &PinManifest,
) -> Result<CurrentGeneration, String> {
    let project = compiled_project_config().map_err(|_| "cutover_blocked".to_owned())?;
    let marker_name = "active-only";
    let marker_contents = "the recorded remote candidate must never replace active local history";
    let mut selected = selected.clone();
    fs::write(
        selected.root.join("repository").join(marker_name),
        marker_contents,
    )
    .map_err(|_| "cutover_blocked".to_owned())?;
    let cases = [
        (
            OnlineSyncContractScenario::FirstTransport,
            "remote_transport",
            1,
        ),
        (
            OnlineSyncContractScenario::FirstMalformed,
            "invalid_remote_observation",
            1,
        ),
        (
            OnlineSyncContractScenario::InitTransport,
            "remote_transport",
            2,
        ),
        (
            OnlineSyncContractScenario::RemoteListTransport,
            "remote_transport",
            3,
        ),
        (
            OnlineSyncContractScenario::RemoteListMismatch,
            "remote_configuration_mismatch",
            3,
        ),
        (
            OnlineSyncContractScenario::SecondTransport,
            "remote_transport",
            4,
        ),
        (
            OnlineSyncContractScenario::SecondNoMatch,
            "remote_changed",
            4,
        ),
        (
            OnlineSyncContractScenario::SecondMalformed,
            "invalid_remote_observation",
            4,
        ),
        (OnlineSyncContractScenario::SecondMoved, "remote_changed", 4),
    ];
    for (scenario, expected_code, expected_remote_commands) in cases {
        let current_before = fs::read(location.state_root.join("current"))
            .map_err(|_| "cutover_blocked".to_owned())?;
        let candidate = root.join(format!("online-sync-recorded-{scenario:?}"));
        fs::create_dir(&candidate).map_err(|_| "cutover_blocked".to_owned())?;
        copy_regular_tree_contents(&selected.root.join("repository"), &candidate, None)?;
        let mut transport =
            OnlineSyncContractTransport::for_scenario(project.clone(), candidate, scenario);
        let error = synchronize(&mut transport, location, &selected, pin, host_target())
            .expect_err("the recorded remote failure scenario must refuse");
        if error.code() != expected_code
            || transport.phase != OnlineSyncContractPhase::Complete
            || transport.commands.len() != expected_remote_commands
            || transport.commands.iter().any(|command| {
                command.argv.iter().any(|argument| {
                    matches!(
                        argument.as_str(),
                        "add" | "pull" | "bootstrap" | "push" | "fetch" | "force" | "update-ref"
                    )
                })
            })
        {
            return Err("cutover_blocked".into());
        }
        let current_after = fs::read(location.state_root.join("current"))
            .map_err(|_| "cutover_blocked".to_owned())?;
        if error.state_changed() {
            if current_after == current_before {
                return Err("cutover_blocked".into());
            }
            let activated =
                current_generation(location).map_err(|error| error.code().to_owned())?;
            if activated.name == selected.name
                || fs::read_to_string(activated.root.join("repository").join(marker_name))
                    .map_err(|_| "cutover_blocked".to_owned())?
                    != marker_contents
            {
                return Err("cutover_blocked".into());
            }
            selected = activated;
        } else if current_after != current_before
            || fs::read_to_string(selected.root.join("repository").join(marker_name))
                .map_err(|_| "cutover_blocked".to_owned())?
                != marker_contents
        {
            return Err("cutover_blocked".into());
        }
    }
    Ok(selected)
}

fn parity_candidate_fixture(
    runner: &mut SystemCommandRunner,
    root: &Path,
    label: &str,
    selected: &CurrentGeneration,
    snapshot: &crate::store::FencedSnapshot,
    mismatch: ContractParityMismatch,
) -> Result<StoreFixture, String> {
    let candidate = init_store(&selected.root.join("bd"), root, label).map_err(str::to_owned)?;
    let documents = snapshot
        .documents
        .iter()
        .map(|document| document.document.clone())
        .collect::<Vec<_>>();
    let operational = snapshot
        .documents
        .iter()
        .filter_map(|document| {
            document
                .operational
                .clone()
                .map(|metadata| (document.document.record.document_key.clone(), metadata))
        })
        .collect::<BTreeMap<_, _>>();
    let (documents, operational, key_value) =
        parity_candidate_projection(mismatch, &documents, &operational)?;
    import_operational_shadow_documents(
        runner,
        &shadow_store(&candidate, &selected.root.join("bd")),
        &selected.manifest.source_commit,
        &documents,
        &operational,
    )
    .map_err(|_| "cutover_blocked".to_owned())?;
    if let Some((key, value)) = key_value {
        run_contract_command(
            runner,
            contract_command(
                selected.root.join("bd"),
                vec![
                    "--sandbox".into(),
                    "kv".into(),
                    "set".into(),
                    key.into(),
                    value,
                ],
                &candidate.repository,
                &candidate.environment,
            ),
        )?;
    }
    Ok(candidate)
}

fn assert_representative_parity_inventory(
    runner: &mut SystemCommandRunner,
    root: &Path,
    location: &crate::store::StoreLocation,
    selected: &CurrentGeneration,
    snapshot: &crate::store::FencedSnapshot,
    pin: &PinManifest,
) -> Result<(), String> {
    let project = compiled_project_config().map_err(|_| "cutover_blocked".to_owned())?;
    let mut selected = selected.clone();
    for (mismatch, label) in [
        (ContractParityMismatch::Authority, "authority"),
        (ContractParityMismatch::Source, "source"),
        (ContractParityMismatch::Logical, "logical"),
        (ContractParityMismatch::Operational, "operational"),
        (ContractParityMismatch::Missing, "missing"),
        (ContractParityMismatch::Extra, "extra"),
        (ContractParityMismatch::UnknownKeyValue, "unknown-key-value"),
    ] {
        let candidate = parity_candidate_fixture(
            runner,
            root,
            &format!("online-sync-parity-{label}"),
            &selected,
            snapshot,
            mismatch,
        )?;
        let candidate_marker = candidate.repository.join("candidate-only");
        fs::write(&candidate_marker, label).map_err(|_| "cutover_blocked".to_owned())?;
        let before = snapshot_regular_tree(&location.state_root)?;
        let mut transport = OnlineSyncContractTransport::stable(
            project.clone(),
            candidate.repository.clone(),
            CONTRACT_REMOTE_GENERATION.into(),
        );
        let error = synchronize(&mut transport, location, &selected, pin, host_target())
            .expect_err("a remote projection mismatch must never activate");
        if error.code() != "remote_shadow_mismatch"
            || transport.phase != OnlineSyncContractPhase::Complete
            || transport.commands.len() != 4
            || transport.commands.iter().any(|command| {
                command.argv.iter().any(|argument| {
                    matches!(
                        argument.as_str(),
                        "add" | "pull" | "bootstrap" | "push" | "fetch" | "force" | "update-ref"
                    )
                })
            })
        {
            return Err("cutover_blocked".into());
        }
        if error.state_changed() {
            let activated =
                current_generation(location).map_err(|error| error.code().to_owned())?;
            let activated_snapshot =
                read_disposable_snapshot(runner, &activated, pin, host_target())
                    .map_err(|error| error.code().to_owned())?;
            if snapshot_regular_tree(&location.state_root)? == before
                || activated.root.join("repository/candidate-only").exists()
                || activated_snapshot.documents != snapshot.documents
            {
                return Err("cutover_blocked".into());
            }
            selected = activated;
        } else if snapshot_regular_tree(&location.state_root)? != before {
            return Err("cutover_blocked".into());
        }
    }
    Ok(())
}

fn assert_cleanup_before_remote_inventory(
    root: &Path,
    location: &crate::store::StoreLocation,
    selected: &CurrentGeneration,
    pin: &PinManifest,
) -> Result<(), String> {
    let candidate = root.join("online-sync-cleanup-candidate");
    fs::create_dir(&candidate).map_err(|_| "cutover_blocked".to_owned())?;
    let project = compiled_project_config().map_err(|_| "cutover_blocked".to_owned())?;
    let before = snapshot_regular_tree(&location.state_root)?;
    let mut transport = OnlineSyncContractTransport::for_scenario(
        project,
        candidate,
        OnlineSyncContractScenario::FirstTransport,
    );
    let error = synchronize_after_disposable_cleanup_failure_for_contract(
        &mut transport,
        location,
        selected,
        pin,
        host_target(),
    )
    .expect_err("the disposable cleanup refusal must precede remote observation");
    if error.code() != "temporary_cleanup_failed"
        || error.state_changed()
        || transport.phase != OnlineSyncContractPhase::AwaitFirstObservation
        || !transport.commands.is_empty()
        || snapshot_regular_tree(&location.state_root)? != before
    {
        return Err("cutover_blocked".into());
    }
    Ok(())
}

fn assert_activation_boundary_inventory(
    runner: &mut SystemCommandRunner,
    root: &Path,
    request: &ContractRequest,
    source: &SourceDocuments,
    pin: &PinManifest,
) -> Result<(), String> {
    let (_fixture, location, base, _bootstrap_request) =
        bootstrap_fixture(runner, root, "online-sync-activation", request, source, pin)?;
    let baseline = read_disposable_snapshot(runner, &base, pin, host_target())
        .map_err(|error| error.code().to_owned())?;
    let original_pointer =
        fs::read(location.state_root.join("current")).map_err(|_| "cutover_blocked".to_owned())?;
    let lock = GenerationActivationLock::acquire_for_sync(&location)
        .map_err(|error| error.code().to_owned())?;
    for (fault, label) in [
        (
            ActivationFault::BeforeGenerationRename,
            "before-generation-rename",
        ),
        (ActivationFault::BeforePointerWrite, "before-pointer-write"),
        (
            ActivationFault::BeforePointerRename,
            "before-pointer-rename",
        ),
    ] {
        let generation_name = contract_generation_name(&format!("activation-{label}"))?;
        let staging = location.generations_dir.join(format!(
            ".staging-{}",
            generation_name
                .strip_prefix("generation-")
                .ok_or_else(|| "cutover_blocked".to_owned())?
        ));
        fs::create_dir(&staging).map_err(|_| "cutover_blocked".to_owned())?;
        copy_regular_tree_contents(&base.root, &staging, None)?;
        let error = activate_staged_generation(&location, &staging, &generation_name, Some(fault))
            .expect_err("an injected activation interruption must preserve the old reader state");
        if error.code() != "bootstrap_interrupted"
            || current_generation(&location).map_err(|error| error.code().to_owned())? != base
            || fs::read(location.state_root.join("current"))
                .map_err(|_| "cutover_blocked".to_owned())?
                != original_pointer
        {
            return Err("cutover_blocked".into());
        }
    }
    let generation_name = contract_generation_name("activation-success")?;
    let staging = location.generations_dir.join(format!(
        ".staging-{}",
        generation_name
            .strip_prefix("generation-")
            .ok_or_else(|| "cutover_blocked".to_owned())?
    ));
    fs::create_dir(&staging).map_err(|_| "cutover_blocked".to_owned())?;
    copy_regular_tree_contents(&base.root, &staging, None)?;
    activate_staged_generation(&location, &staging, &generation_name, None)
        .map_err(|error| error.code().to_owned())?;
    let activated = current_generation(&location).map_err(|error| error.code().to_owned())?;
    let activated_snapshot = read_disposable_snapshot(runner, &activated, pin, host_target())
        .map_err(|error| error.code().to_owned())?;
    if activated.name == base.name
        || fs::read(location.state_root.join("current"))
            .map_err(|_| "cutover_blocked".to_owned())?
            != format!("{generation_name}\n").into_bytes()
        || activated_snapshot != baseline
    {
        return Err("cutover_blocked".into());
    }
    drop(lock);
    Ok(())
}

fn assert_pending_remote_observation_inventory(
    runner: &mut SystemCommandRunner,
    root: &Path,
    request: &ContractRequest,
    source: &SourceDocuments,
    pin: &PinManifest,
) -> Result<(), String> {
    let (_fixture, location, base, _bootstrap_request) =
        bootstrap_fixture(runner, root, "online-sync-pending", request, source, pin)?;
    let project = compiled_project_config().map_err(|_| "cutover_blocked".to_owned())?;
    let pending = [CONTRACT_PENDING_OPERATION];
    let equivalent = activate_freshness_fixture(
        runner,
        &location,
        &base,
        pin,
        RemoteRelation::Equivalent,
        true,
        &pending,
    )?;
    let same_candidate = root.join("online-sync-pending-same-candidate");
    fs::create_dir(&same_candidate).map_err(|_| "cutover_blocked".to_owned())?;
    copy_regular_tree_contents(&equivalent.root.join("repository"), &same_candidate, None)?;
    let same_before =
        fs::read(location.state_root.join("current")).map_err(|_| "cutover_blocked".to_owned())?;
    let mut same_transport = OnlineSyncContractTransport::for_scenario(
        project.clone(),
        same_candidate,
        OnlineSyncContractScenario::Stable,
    );
    let same_error = synchronize(
        &mut same_transport,
        &location,
        &equivalent,
        pin,
        host_target(),
    )
    .expect_err("pending work must stop before the remote clone");
    if same_error.code() != "pending_mutations"
        || !same_error.state_changed()
        || same_transport.commands.len() != 1
        || same_transport.commands.iter().any(|command| {
            command.argv.iter().any(|argument| {
                matches!(
                    argument.as_str(),
                    "init"
                        | "add"
                        | "pull"
                        | "bootstrap"
                        | "push"
                        | "fetch"
                        | "force"
                        | "update-ref"
                )
            })
        })
        || fs::read(location.state_root.join("current"))
            .map_err(|_| "cutover_blocked".to_owned())?
            == same_before
    {
        return Err("cutover_blocked".into());
    }
    let same = current_generation(&location).map_err(|error| error.code().to_owned())?;
    let same_snapshot = read_disposable_snapshot(runner, &same, pin, host_target())
        .map_err(|error| error.code().to_owned())?;
    if same.manifest.remote_relation != RemoteRelation::Equivalent
        || same.manifest.remote_generation.as_deref() != Some(CONTRACT_REMOTE_GENERATION)
        || same.manifest.last_successful_sync_at.as_deref() != Some(CONTRACT_OBSERVED_AT)
        || same.manifest.pending_operation_ids != pending
        || same_snapshot.freshness.freshness != Freshness::Unpublished
        || same_snapshot.freshness.pending_mutations.operation_ids != pending
    {
        return Err("cutover_blocked".into());
    }

    let different_seed = activate_freshness_fixture(
        runner,
        &location,
        &same,
        pin,
        RemoteRelation::Equivalent,
        true,
        &pending,
    )?;
    let different_candidate = root.join("online-sync-pending-different-candidate");
    fs::create_dir(&different_candidate).map_err(|_| "cutover_blocked".to_owned())?;
    copy_regular_tree_contents(
        &different_seed.root.join("repository"),
        &different_candidate,
        None,
    )?;
    let different_before =
        fs::read(location.state_root.join("current")).map_err(|_| "cutover_blocked".to_owned())?;
    let mut different_transport = OnlineSyncContractTransport::for_scenario(
        project,
        different_candidate,
        OnlineSyncContractScenario::FirstMoved,
    );
    let different_error = synchronize(
        &mut different_transport,
        &location,
        &different_seed,
        pin,
        host_target(),
    )
    .expect_err("pending work at a changed remote must stop before the remote clone");
    if different_error.code() != "pending_mutations"
        || !different_error.state_changed()
        || different_transport.commands.len() != 1
        || different_transport.commands.iter().any(|command| {
            command.argv.iter().any(|argument| {
                matches!(
                    argument.as_str(),
                    "init"
                        | "add"
                        | "pull"
                        | "bootstrap"
                        | "push"
                        | "fetch"
                        | "force"
                        | "update-ref"
                )
            })
        })
        || fs::read(location.state_root.join("current"))
            .map_err(|_| "cutover_blocked".to_owned())?
            == different_before
    {
        return Err("cutover_blocked".into());
    }
    let different = current_generation(&location).map_err(|error| error.code().to_owned())?;
    let different_snapshot = read_disposable_snapshot(runner, &different, pin, host_target())
        .map_err(|error| error.code().to_owned())?;
    if different.manifest.remote_relation != RemoteRelation::Unknown
        || different.manifest.remote_generation.as_deref() != Some(CONTRACT_MOVED_REMOTE_GENERATION)
        || different.manifest.last_successful_sync_at.as_deref() != Some(CONTRACT_OBSERVED_AT)
        || different.manifest.pending_operation_ids != pending
        || different_snapshot.freshness.freshness != Freshness::UnknownWithUnpublished
        || different_snapshot.freshness.pending_mutations.operation_ids != pending
    {
        return Err("cutover_blocked".into());
    }
    Ok(())
}

fn online_sync_contract_case(
    runner: &mut SystemCommandRunner,
    root: &Path,
    request: &ContractRequest,
    source: &SourceDocuments,
    pin: &PinManifest,
) -> Result<LocalReadEvidence, String> {
    let (fixture, location, base, _bootstrap_request) =
        bootstrap_fixture(runner, root, "online-sync", request, source, pin)?;
    let baseline = read_disposable_snapshot(runner, &base, pin, host_target())
        .map_err(|_| "cutover_blocked".to_owned())?;
    assert_installed_sync_config_and_lock(runner, root, &fixture, &location, &base, &baseline)
        .map_err(|_| "cutover_blocked".to_owned())?;
    let remote_candidate = root.join("online-sync-recorded-remote-candidate");
    fs::create_dir(&remote_candidate).map_err(|_| "cutover_blocked".to_owned())?;
    copy_regular_tree_contents(&base.root.join("repository"), &remote_candidate, None)?;
    let selected = assert_recorded_remote_failure_inventory(root, &location, &base, pin)
        .map_err(|_| "cutover_blocked".to_owned())?;
    assert_representative_parity_inventory(runner, root, &location, &selected, &baseline, pin)
        .map_err(|_| "cutover_blocked".to_owned())?;
    let selected = current_generation(&location).map_err(|_| "cutover_blocked".to_owned())?;
    assert_cleanup_before_remote_inventory(root, &location, &selected, pin)
        .map_err(|_| "cutover_blocked".to_owned())?;
    let active_state_before = snapshot_regular_tree(&location.state_root)?;
    let project = compiled_project_config().map_err(|_| "cutover_blocked".to_owned())?;
    let remote_generation = CONTRACT_REMOTE_GENERATION.to_owned();
    let mut transport =
        OnlineSyncContractTransport::stable(project, remote_candidate, remote_generation.clone());
    let result = synchronize(&mut transport, &location, &selected, pin, host_target())
        .map_err(|_| "cutover_blocked".to_owned())?;
    let activated = current_generation(&location).map_err(|_| "cutover_blocked".to_owned())?;
    if transport.phase != OnlineSyncContractPhase::Complete
        || transport.commands.len() != 4
        || transport.commands.iter().any(|command| {
            command.argv.iter().any(|argument| {
                matches!(
                    argument.as_str(),
                    "add" | "pull" | "bootstrap" | "push" | "fetch" | "force" | "update-ref"
                )
            })
        })
        || !result.state_changed
        || activated.name == selected.name
        || !base.root.is_dir()
        || activated.root.join("repository/active-only").exists()
        || !selected.root.join("repository/active-only").is_file()
        || activated.manifest.source_commit != selected.manifest.source_commit
        || activated.manifest.logical_export_sha256 != selected.manifest.logical_export_sha256
        || activated.manifest.operational_projection_sha256
            != selected.manifest.operational_projection_sha256
        || activated.manifest.remote_generation.as_deref() != Some(remote_generation.as_str())
        || activated.manifest.remote_observed_at.is_none()
        || activated.manifest.remote_observed_at != activated.manifest.last_successful_sync_at
        || activated.manifest.pending_operation_ids != Vec::<String>::new()
    {
        return Err("cutover_blocked".into());
    }
    let activated_snapshot = read_disposable_snapshot(runner, &activated, pin, host_target())
        .map_err(|error| error.code().to_owned())?;
    if activated_snapshot.documents != baseline.documents
        || activated_snapshot.freshness.freshness != Freshness::SynchronizedAsOf
        || snapshot_regular_tree(&location.state_root)? == active_state_before
    {
        return Err("cutover_blocked".into());
    }
    assert_exact_source_projection(source, &activated_snapshot)?;
    assert_pending_remote_observation_inventory(runner, root, request, source, pin)
        .map_err(|_| "cutover_blocked".to_owned())?;
    assert_activation_boundary_inventory(runner, root, request, source, pin)
        .map_err(|_| "cutover_blocked".to_owned())?;
    Ok(LocalReadEvidence {
        clone_labels: vec![
            "online-sync-worktree-a".into(),
            "online-sync-worktree-b".into(),
        ],
        source_commit: activated.manifest.source_commit,
        local_generation: activated.manifest.local_generation,
        operation_ids: Vec::new(),
        command_plans: vec![
            "real local bootstrap/current/runtime/fence/parity/activation".into(),
            "recorded Git observation and exact remote-clone failure boundary".into(),
            "recorded R0/init/list/R1 refusal inventory with real local metadata fencing".into(),
            "real representative authority/source/logical/operational/missing/extra/unknown-KV parity refusals".into(),
            "real pending local generations with recorded same/different R0 observations".into(),
            "contract cleanup-before-remote and activation-boundary inventory".into(),
            "no remote add/pull/bootstrap/push/fetch/force/update-ref".into(),
        ],
    })
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
enum GitShimCategory {
    LocatorTop,
    LocatorCommon,
    Observation,
    LocalBeadsDiscovery,
    Unexpected,
}

#[cfg(unix)]
impl GitShimCategory {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "locator_top" => Ok(Self::LocatorTop),
            "locator_common" => Ok(Self::LocatorCommon),
            "observation" => Ok(Self::Observation),
            "local_beads_discovery" => Ok(Self::LocalBeadsDiscovery),
            "unexpected" => Ok(Self::Unexpected),
            _ => Err("cutover_blocked".into()),
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct GitShimRecord {
    cwd: PathBuf,
    argv: Vec<String>,
    category: GitShimCategory,
}

#[cfg(unix)]
struct InstalledGitShimBinding {
    real_git: PathBuf,
    capture: PathBuf,
    worktree: PathBuf,
    common_dir: PathBuf,
    locator_path: String,
    project: ProjectConfig,
}

#[cfg(unix)]
fn resolve_absolute_git(path: &std::ffi::OsStr) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;

    for directory in std::env::split_paths(path) {
        let candidate = directory.join("git");
        let Ok(metadata) = fs::metadata(&candidate) else {
            continue;
        };
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            continue;
        }
        let canonical = fs::canonicalize(&candidate).map_err(|_| "cutover_blocked".to_owned())?;
        let metadata = fs::metadata(&canonical).map_err(|_| "cutover_blocked".to_owned())?;
        if canonical.is_absolute()
            && metadata.is_file()
            && metadata.permissions().mode() & 0o111 != 0
        {
            return Ok(canonical);
        }
    }
    Err("cutover_blocked".into())
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
fn shell_quote_path(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(shell_quote)
        .ok_or_else(|| "cutover_blocked".into())
}

#[cfg(unix)]
fn write_installed_config_git_shim(
    shim: &Path,
    binding: &InstalledGitShimBinding,
) -> Result<(), String> {
    let replacements = [
        ("@@CAPTURE@@", shell_quote_path(&binding.capture)?),
        ("@@REAL_GIT@@", shell_quote_path(&binding.real_git)?),
        ("@@WORKTREE@@", shell_quote_path(&binding.worktree)?),
        ("@@COMMON@@", shell_quote_path(&binding.common_dir)?),
        ("@@LOCATOR_PATH@@", shell_quote(&binding.locator_path)),
        (
            "@@OBSERVATION_URL@@",
            shell_quote(binding.project.git_observation_url()),
        ),
        ("@@DATA_REF@@", shell_quote(binding.project.data_ref())),
    ];
    let mut script = r#"#!/usr/bin/env bash
set -eu
capture=@@CAPTURE@@
real_git=@@REAL_GIT@@
worktree=@@WORKTREE@@
common_dir=@@COMMON@@
locator_path=@@LOCATOR_PATH@@
observation_url=@@OBSERVATION_URL@@
data_ref=@@DATA_REF@@
cwd=$(pwd -P)

record() {
  local category=$1
  shift
  {
    printf '%s\0' "$#"
    printf '%s\0' "$cwd"
    for argument in "$@"; do
      printf '%s\0' "$argument"
    done
    printf '%s\0' "$category"
  } >> "$capture"
}

forbidden_environment_is_absent() {
  [[ -z "${GIT_ASKPASS+x}" && -z "${SSH_ASKPASS+x}" && -z "${SSH_AUTH_SOCK+x}" && -z "${GIT_SSH+x}" && -z "${GIT_SSH_COMMAND+x}" && -z "${GIT_PROXY_COMMAND+x}" && -z "${GIT_CREDENTIAL_HELPER+x}" && -z "${GIT_CONFIG_SYSTEM+x}" && -z "${GIT_CONFIG_COUNT+x}" && -z "${GIT_CONFIG_PARAMETERS+x}" && -z "${HTTP_PROXY+x}" && -z "${HTTPS_PROXY+x}" && -z "${ALL_PROXY+x}" && -z "${NO_PROXY+x}" && -z "${http_proxy+x}" && -z "${https_proxy+x}" && -z "${all_proxy+x}" && -z "${no_proxy+x}" && -z "${GITHUB_TOKEN+x}" && -z "${GH_TOKEN+x}" && -z "${BEADS_TOKEN+x}" && -z "${BEADS_API_KEY+x}" && -z "${DOLT_TOKEN+x}" && -z "${DOLT_CREDENTIAL+x}" ]]
}

locator_environment_is_bound() {
  [[ "$PATH" == "$locator_path" && "${GIT_CONFIG_NOSYSTEM-}" == 1 && "${GIT_TERMINAL_PROMPT-}" == 0 && "${GIT_NO_LAZY_FETCH-}" == 1 && "${GIT_OPTIONAL_LOCKS-}" == 0 && -z "${HOME+x}" && -z "${XDG_CONFIG_HOME+x}" && -z "${XDG_CACHE_HOME+x}" && -z "${XDG_DATA_HOME+x}" && -z "${TMPDIR+x}" && -z "${GIT_CONFIG_GLOBAL+x}" ]] && forbidden_environment_is_absent
}

runtime_root() {
  case "${TMPDIR-}" in
    */runtime/tmp) ;;
    *) return 1 ;;
  esac
  local root=${TMPDIR%/runtime/tmp}
  [[ -n "$root" ]] || return 1
  local physical tmp
  physical=$(cd "$root" 2>/dev/null && pwd -P) || return 1
  tmp=$(cd "$TMPDIR" 2>/dev/null && pwd -P) || return 1
  [[ "$tmp" == "$physical/runtime/tmp" ]] || return 1
  printf '%s\n' "$physical"
}

runtime_environment_is_bound() {
  local root=$1
  local home config cache data tmp config_parent config_name
  home=$(cd "${HOME-}" 2>/dev/null && pwd -P) || return 1
  config=$(cd "${XDG_CONFIG_HOME-}" 2>/dev/null && pwd -P) || return 1
  cache=$(cd "${XDG_CACHE_HOME-}" 2>/dev/null && pwd -P) || return 1
  data=$(cd "${XDG_DATA_HOME-}" 2>/dev/null && pwd -P) || return 1
  tmp=$(cd "${TMPDIR-}" 2>/dev/null && pwd -P) || return 1
  config_parent=${GIT_CONFIG_GLOBAL%/*}
  config_name=${GIT_CONFIG_GLOBAL##*/}
  config_parent=$(cd "$config_parent" 2>/dev/null && pwd -P) || return 1
  [[ "$PATH" == "$locator_path" && "$home" == "$root/runtime/home" && "$config" == "$root/runtime/xdg_config" && "$cache" == "$root/runtime/xdg_cache" && "$data" == "$root/runtime/xdg_data" && "$tmp" == "$root/runtime/tmp" && "$config_parent/$config_name" == "$root/runtime/git_config_global" && "${GIT_CONFIG_NOSYSTEM-}" == 1 && "${GIT_TERMINAL_PROMPT-}" == 0 && "${GIT_NO_LAZY_FETCH-}" == 1 && "${GIT_OPTIONAL_LOCKS-}" == 0 && "${BD_DISABLE_METRICS-}" == 1 && "${BD_DISABLE_EVENT_FLUSH-}" == 1 && "${BD_NON_INTERACTIVE-}" == 1 && "${CI-}" == true ]] && forbidden_environment_is_absent
}

if [[ "$cwd" == "$worktree" ]] && locator_environment_is_bound; then
  if [[ "$#" -eq 2 && "$1" == rev-parse && "$2" == --show-toplevel ]]; then
    record locator_top "$@"
    printf '%s\n' "$worktree"
    exit 0
  fi
  if [[ "$#" -eq 3 && "$1" == rev-parse && "$2" == --path-format=absolute && "$3" == --git-common-dir ]]; then
    record locator_common "$@"
    printf '%s\n' "$common_dir"
    exit 0
  fi
fi

root=
if root=$(runtime_root) && runtime_environment_is_bound "$root"; then
  if [[ "$#" -eq 4 && "$1" == ls-remote && "$2" == --exit-code && "$3" == "$observation_url" && "$4" == "$data_ref" && "$cwd" == "$root" ]]; then
    record observation "$@"
    exit 2
  fi
  if [[ "$#" -eq 5 && "$1" == -C && "$3" == rev-parse && "$4" == --git-dir && "$5" == --git-common-dir ]]; then
    repository=
    if repository=$(cd "$2" 2>/dev/null && pwd -P) && [[ "$repository" == "$cwd" && "$repository" == "$root/repository" ]]; then
      record local_beads_discovery "$@"
      exec "$real_git" "$@"
    fi
  fi
fi

record unexpected "$@"
exit 97
"#
    .to_owned();
    for (needle, value) in replacements {
        script = script.replace(needle, &value);
    }
    fs::write(shim, script).map_err(|_| "cutover_blocked".to_owned())
}

#[cfg(unix)]
fn read_installed_git_shim_records(path: &Path) -> Result<Vec<GitShimRecord>, String> {
    fn field(bytes: &[u8], cursor: &mut usize) -> Result<String, String> {
        let end = bytes[*cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|offset| *cursor + offset)
            .ok_or_else(|| "cutover_blocked".to_owned())?;
        let value = String::from_utf8(bytes[*cursor..end].to_vec())
            .map_err(|_| "cutover_blocked".to_owned())?;
        *cursor = end + 1;
        Ok(value)
    }

    let bytes = fs::read(path).map_err(|_| "cutover_blocked".to_owned())?;
    let mut cursor = 0;
    let mut records = Vec::new();
    while cursor < bytes.len() {
        let argc = field(&bytes, &mut cursor)?
            .parse::<usize>()
            .map_err(|_| "cutover_blocked".to_owned())?;
        let cwd = PathBuf::from(field(&bytes, &mut cursor)?);
        let mut argv = Vec::with_capacity(argc);
        for _ in 0..argc {
            argv.push(field(&bytes, &mut cursor)?);
        }
        let category = GitShimCategory::parse(&field(&bytes, &mut cursor)?)?;
        records.push(GitShimRecord {
            cwd,
            argv,
            category,
        });
    }
    Ok(records)
}

#[cfg(unix)]
fn assert_installed_git_shim_records(
    records: &[GitShimRecord],
    binding: &InstalledGitShimBinding,
) -> Result<(), String> {
    let locator_top = ["rev-parse", "--show-toplevel"];
    let locator_common = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
    let observation = [
        "ls-remote",
        "--exit-code",
        binding.project.git_observation_url(),
        binding.project.data_ref(),
    ];
    let matches = |category: GitShimCategory, argv: &[&str]| {
        records
            .iter()
            .filter(|record| {
                record.category == category
                    && record
                        .argv
                        .iter()
                        .map(String::as_str)
                        .eq(argv.iter().copied())
            })
            .count()
    };
    if records
        .iter()
        .any(|record| record.category == GitShimCategory::Unexpected)
        || matches(GitShimCategory::LocatorTop, &locator_top) != 4
        || matches(GitShimCategory::LocatorCommon, &locator_common) != 4
        || matches(GitShimCategory::Observation, &observation) != 2
        || records
            .iter()
            .filter(|record| record.category == GitShimCategory::LocalBeadsDiscovery)
            .count()
            < 2
    {
        return Err("cutover_blocked".into());
    }
    Ok(())
}

fn assert_installed_sync_config_and_lock(
    runner: &mut SystemCommandRunner,
    root: &Path,
    fixture: &MirrorFixture,
    location: &crate::store::StoreLocation,
    base: &CurrentGeneration,
    baseline: &crate::store::FencedSnapshot,
) -> Result<(), String> {
    #[cfg(not(unix))]
    {
        let _ = (runner, root, fixture, location, base, baseline);
        return Err("cutover_blocked".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let project = compiled_project_config().map_err(|_| "cutover_blocked".to_owned())?;
        let worktree = &fixture.first_worktree;
        let poison = worktree.join("tools/work-state-project.toml");
        fs::write(
            &poison,
            "schema_version = 1\nproject_id = \"poisoned\"\nremote_name = \"other\"\ngit_observation_url = \"https://example.invalid/poisoned.git\"\ndolt_remote_url = \"git+https://example.invalid/poisoned.git\"\ndata_ref = \"refs/poisoned/data\"\n",
        )
        .map_err(|_| "cutover_blocked".to_owned())?;
        let fake_bin = root.join("online-sync-installed-bin");
        fs::create_dir(&fake_bin).map_err(|_| "cutover_blocked".to_owned())?;
        let cargo_marker = root.join("online-sync-cargo-marker");
        let rustup_marker = root.join("online-sync-rustup-marker");
        let original_path = std::env::var_os("PATH").ok_or_else(|| "cutover_blocked".to_owned())?;
        let real_git = resolve_absolute_git(&original_path)?;
        let locator_path = std::env::join_paths([fake_bin.as_os_str(), original_path.as_os_str()])
            .map_err(|_| "cutover_blocked".to_owned())?
            .into_string()
            .map_err(|_| "cutover_blocked".to_owned())?;
        let canonical_worktree =
            fs::canonicalize(worktree).map_err(|_| "cutover_blocked".to_owned())?;
        let canonical_common =
            fs::canonicalize(&location.common_dir).map_err(|_| "cutover_blocked".to_owned())?;
        let git = fake_bin.join("git");
        let binding = InstalledGitShimBinding {
            real_git,
            capture: root.join("online-sync-installed-git-capture"),
            worktree: canonical_worktree,
            common_dir: canonical_common,
            locator_path,
            project: project.clone(),
        };
        write_installed_config_git_shim(&git, &binding)?;
        for (name, marker_path) in [("cargo", &cargo_marker), ("rustup", &rustup_marker)] {
            let path = fake_bin.join(name);
            fs::write(
                &path,
                format!(
                    "#!/usr/bin/env bash\nprintf '{name}\\n' > '{}'\nexit 97\n",
                    marker_path.display()
                ),
            )
            .map_err(|_| "cutover_blocked".to_owned())?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|_| "cutover_blocked".to_owned())?;
        }
        fs::set_permissions(&git, fs::Permissions::from_mode(0o700))
            .map_err(|_| "cutover_blocked".to_owned())?;
        let environment = BTreeMap::from([("PATH".into(), binding.locator_path.clone())]);
        let launcher = worktree.join("tools/work-state");
        let current_before = fs::read(location.state_root.join("current"))
            .map_err(|_| "cutover_blocked".to_owned())?;
        if regular_file_digest(&base.root.join("plasmosome-work-state"))?
            != base.manifest.wrapper_sha256
        {
            return Err("cutover_blocked".into());
        }
        for json in [false, true] {
            let argv = if json {
                vec!["sync".into(), "--json".into()]
            } else {
                vec!["sync".into()]
            };
            let output = runner
                .run(contract_command(&launcher, argv, worktree, &environment))
                .map_err(|_| "cutover_blocked".to_owned())?;
            let expected = if json {
                "{\"code\":\"remote_uninitialized\",\"state_changed\":false}\n"
            } else {
                "error[remote_uninitialized]: remote_uninitialized state_changed=false\n"
            };
            if output.status != 1 {
                return Err(if json {
                    "installed-json-status"
                } else {
                    "installed-human-status"
                }
                .into());
            }
            if !output.stdout.is_empty() {
                return Err(if json {
                    "installed-json-stdout"
                } else {
                    "installed-human-stdout"
                }
                .into());
            }
            if output.stderr != expected {
                return Err(if json {
                    "installed-json-stderr"
                } else {
                    "installed-human-stderr"
                }
                .into());
            }
        }
        let records = read_installed_git_shim_records(&binding.capture)?;
        assert_installed_git_shim_records(&records, &binding)?;
        if cargo_marker.exists() || rustup_marker.exists() {
            return Err("installed-cargo-route".into());
        }
        if fs::read(location.state_root.join("current"))
            .map_err(|_| "installed-current-read".to_owned())?
            != current_before
        {
            return Err("installed-current-change".into());
        }
        let held = GenerationActivationLock::acquire_for_sync(location)
            .map_err(|error| error.code().to_owned())?;
        let records_before_busy = records.len();
        let busy = runner
            .run(contract_command(
                &launcher,
                vec!["sync".into(), "--json".into()],
                worktree,
                &environment,
            ))
            .map_err(|_| "cutover_blocked".to_owned())?;
        if busy.status != 1
            || !busy.stdout.is_empty()
            || busy.stderr != "{\"code\":\"sync_busy\",\"state_changed\":false}\n"
        {
            return Err("cutover_blocked".into());
        }
        assert_launcher_read(runner, worktree, ReadCommand::List, base, baseline)?;
        let post_lock_records = read_installed_git_shim_records(&binding.capture)?;
        if post_lock_records.len() < records_before_busy
            || post_lock_records[records_before_busy..]
                .iter()
                .any(|record| {
                    !matches!(
                        record.category,
                        GitShimCategory::LocatorTop | GitShimCategory::LocatorCommon
                    )
                })
            || fs::read(location.state_root.join("current"))
                .map_err(|_| "cutover_blocked".to_owned())?
                != current_before
        {
            return Err("cutover_blocked".into());
        }
        drop(held);
        Ok(())
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn run_contract(request: &ContractRequest) -> Result<ContractResult, Box<ContractResult>> {
    let root = tempfile::tempdir()
        .map_err(|_| Box::new(ContractResult::refusal(&request.case, "cutover_blocked")))?;
    let result = (|| {
        let manifest = PinManifest::load(manifest_path())
            .map_err(|error| Box::new(ContractResult::refusal(&request.case, error.code())))?;
        let mut runner = SystemCommandRunner;
        VerifiedBeads::verify_with_environment(
            &manifest,
            host_target(),
            &request.archive,
            &request.binary,
            isolated_environment(root.path()),
            &mut runner,
        )
        .map_err(|error| Box::new(ContractResult::refusal(&request.case, error.code())))?;
        if request.case == "version-pin" {
            return Ok(ContractResult::passed(&request.case, Vec::new()));
        }
        if matches!(request.case.as_str(), "stealth-init" | "hermetic") {
            init_store(&request.binary, root.path(), "clone-a")
                .map_err(|code| Box::new(ContractResult::refusal(&request.case, code)))?;
            return Ok(ContractResult::passed(
                &request.case,
                vec!["clone-a".into()],
            ));
        }
        if requires_shadow_round_trip(&request.case) {
            let source_ref = request.source_ref.as_deref().ok_or_else(|| {
                Box::new(ContractResult::refusal(&request.case, "invalid_source_ref"))
            })?;
            let source = load_documents(
                &mut runner,
                &repository_root(),
                &isolated_environment(root.path()),
                source_ref,
            )
            .map_err(|error: DocumentError| {
                Box::new(source_refusal(
                    &request.case,
                    source_ref,
                    error.code(),
                    error.offending_key,
                    None,
                ))
            })?;
            assert_historical_task_count(&source).map_err(|code| {
                Box::new(snapshot_refusal(
                    &request.case,
                    source_ref,
                    &source,
                    code,
                    None,
                    Some("different".into()),
                ))
            })?;
            let first = init_store(&request.binary, root.path(), "clone-a").map_err(|code| {
                Box::new(snapshot_refusal(
                    &request.case,
                    source_ref,
                    &source,
                    code,
                    None,
                    None,
                ))
            })?;
            let second = init_store(&request.binary, root.path(), "clone-b").map_err(|code| {
                Box::new(snapshot_refusal(
                    &request.case,
                    source_ref,
                    &source,
                    code,
                    None,
                    None,
                ))
            })?;
            validate_independent_stores(&first, &second).map_err(|code| {
                Box::new(snapshot_refusal(
                    &request.case,
                    source_ref,
                    &source,
                    code,
                    None,
                    None,
                ))
            })?;
            let evidence =
                run_shadow_round_trip(&mut runner, &request.binary, &source, &first, &second)
                    .map_err(|error| {
                        Box::new(snapshot_refusal(
                            &request.case,
                            source_ref,
                            &source,
                            error.code(),
                            error.offending_key,
                            error.mismatch,
                        ))
                    })?;
            let mut result = migration_result(&request.case, source_ref, &source, evidence);
            if request.case == "online-sync" {
                let online = online_sync_contract_case(
                    &mut runner,
                    root.path(),
                    request,
                    &source,
                    &manifest,
                )
                .map_err(|code| {
                    Box::new(snapshot_refusal(
                        &request.case,
                        source_ref,
                        &source,
                        &code,
                        None,
                        None,
                    ))
                })?;
                return Ok(local_read_contract_result(
                    &request.case,
                    source_ref,
                    &source,
                    online,
                ));
            }
            if request.case == "all" {
                for case in local_read_cases(&request.case) {
                    let local = local_read_contract_case(
                        &mut runner,
                        root.path(),
                        case,
                        request,
                        &source,
                        &manifest,
                    )
                    .map_err(|code| {
                        Box::new(snapshot_refusal(
                            &request.case,
                            source_ref,
                            &source,
                            &code,
                            None,
                            None,
                        ))
                    })?;
                    result.clone_labels.extend(local.clone_labels.clone());
                    result.command_plans.extend(local.command_plans.clone());
                    result.scenarios.push(ScenarioEvidence {
                        case: (*case).into(),
                        observed_base: local.source_commit,
                        final_generation: local.local_generation,
                        operation_ids: local.operation_ids,
                        command_plans: local.command_plans,
                    });
                }
                for case in online_sync_contract_cases(&request.case) {
                    let online = online_sync_contract_case(
                        &mut runner,
                        root.path(),
                        request,
                        &source,
                        &manifest,
                    )
                    .map_err(|code| {
                        Box::new(snapshot_refusal(
                            &request.case,
                            source_ref,
                            &source,
                            &code,
                            None,
                            None,
                        ))
                    })?;
                    result.clone_labels.extend(online.clone_labels.clone());
                    result.command_plans.extend(online.command_plans.clone());
                    result.scenarios.push(ScenarioEvidence {
                        case: (*case).into(),
                        observed_base: online.source_commit,
                        final_generation: online.local_generation,
                        operation_ids: online.operation_ids,
                        command_plans: online.command_plans,
                    });
                }
                let transport = run_scripted_cases("transport")
                    .map_err(|code| Box::new(ContractResult::refusal(&request.case, code)))?;
                result.observed_base = transport.observed_base;
                result.final_generation = transport.final_generation;
                result.operation_ids = transport.operation_ids;
                result.command_plans.extend(transport.command_plans);
                result.scenarios.extend(transport.scenarios);
            }
            return Ok(result);
        }
        if requires_local_read_contract(&request.case) {
            let source_ref = request.source_ref.as_deref().ok_or_else(|| {
                Box::new(ContractResult::refusal(&request.case, "invalid_source_ref"))
            })?;
            let source = load_documents(
                &mut runner,
                &repository_root(),
                &isolated_environment(root.path()),
                source_ref,
            )
            .map_err(|error: DocumentError| {
                Box::new(source_refusal(
                    &request.case,
                    source_ref,
                    error.code(),
                    error.offending_key,
                    None,
                ))
            })?;
            let local = local_read_contract_case(
                &mut runner,
                root.path(),
                &request.case,
                request,
                &source,
                &manifest,
            )
            .map_err(|code| {
                Box::new(snapshot_refusal(
                    &request.case,
                    source_ref,
                    &source,
                    &code,
                    None,
                    None,
                ))
            })?;
            return Ok(local_read_contract_result(
                &request.case,
                source_ref,
                &source,
                local,
            ));
        }
        let first = init_store(&request.binary, root.path(), "clone-a")
            .map_err(|code| Box::new(ContractResult::refusal(&request.case, code)))?;
        let second = init_store(&request.binary, root.path(), "clone-b")
            .map_err(|code| Box::new(ContractResult::refusal(&request.case, code)))?;
        validate_independent_stores(&first, &second)
            .map_err(|code| Box::new(ContractResult::refusal(&request.case, code)))?;
        let mut result = run_scripted_cases(&request.case)
            .map_err(|code| Box::new(ContractResult::refusal(&request.case, code)))?;
        result.clone_labels = vec!["clone-a".into(), "clone-b".into()];
        Ok(result)
    })();
    finish_contract(result, dispose_fixture_root(root), &request.case)
}

fn manifest_path() -> PathBuf {
    repository_root().join("tools/work-state-beads-1.1.2.toml")
}

fn finish_contract(
    result: Result<ContractResult, Box<ContractResult>>,
    cleanup: Result<(), &'static str>,
    case: &str,
) -> Result<ContractResult, Box<ContractResult>> {
    match cleanup {
        Ok(()) => result,
        Err(code) => Err(Box::new(ContractResult::refusal(case, code))),
    }
}
pub fn run_scripted_cases(case: &str) -> Result<ContractResult, &'static str> {
    let cases: &[&str] = if matches!(case, "transport" | "all") {
        &[
            "stale-base-fence",
            "push-conflict-recovery",
            "transport-retries",
        ]
    } else {
        &[case]
    };
    let mut results = Vec::new();
    for case in cases {
        let mut runner = RecordingCommandRunner::scripted(
            scripted_outcomes(case).map_err(|_| "cutover_blocked")?,
        );
        let result =
            run_scripted_contract_case(case, &mut runner).map_err(|_| "cutover_blocked")?;
        runner.finish().map_err(|_| "cutover_blocked")?;
        results.push(result);
    }
    let final_result = results.last().cloned().ok_or("cutover_blocked")?;
    let operation_ids = results
        .iter()
        .flat_map(|result| result.operation_ids.iter().cloned())
        .collect();
    let command_plans = results
        .iter()
        .flat_map(|result| result.command_plans.iter().cloned())
        .collect();
    let scenarios = results
        .iter()
        .map(|result| {
            Ok(ScenarioEvidence {
                case: result.case.clone(),
                observed_base: result.observed_base.clone().ok_or("cutover_blocked")?,
                final_generation: result.final_generation.clone().ok_or("cutover_blocked")?,
                operation_ids: result.operation_ids.clone(),
                command_plans: result.command_plans.clone(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ContractResult {
        case: case.into(),
        operation_ids,
        command_plans,
        scenarios,
        ..final_result
    })
}

pub fn scripted_outcomes(case: &str) -> Result<Vec<Result<CommandOutput, String>>, String> {
    let g0 = "0000000000000000000000000000000000000000";
    let g1 = "1111111111111111111111111111111111111111";
    let g2 = "2222222222222222222222222222222222222222";
    match case {
        "stale-base-fence" => Ok(vec![
            Ok(observation_output(g0)),
            Ok(observation_output(g0)),
            Ok(CommandOutput::success("wrote winner")),
            Ok(CommandOutput::success("committed winner")),
            Ok(CommandOutput::success("winner")),
            Ok(observation_output(g1)),
            Ok(CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "non-fast-forward".into(),
            }),
            Ok(observation_output(g1)),
            Ok(CommandOutput::success("refreshed")),
            Ok(CommandOutput::success("wrote replay")),
            Ok(CommandOutput::success("committed replay")),
            Ok(CommandOutput::success("replayed")),
            Ok(observation_output(g2)),
            Ok(CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "non-fast-forward".into(),
            }),
            Ok(observation_output(g2)),
            Ok(logical_export_output(&["winner", "replay"])),
            Ok(CommandOutput::success(format!(
                "{g0}\tbase\n{g1}\toperation:winner\n{g2}\toperation:replay\n"
            ))),
        ]),
        "push-conflict-recovery" => Ok(vec![
            Ok(observation_output(g0)),
            Ok(observation_output(g0)),
            Ok(CommandOutput::success("wrote winner")),
            Ok(CommandOutput::success("committed winner")),
            Ok(CommandOutput::success("winner")),
            Ok(observation_output(g1)),
            Ok(CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "non-fast-forward".into(),
            }),
            Ok(observation_output(g1)),
            Ok(CommandOutput::success("refreshed")),
            Ok(CommandOutput::success("wrote replay")),
            Ok(CommandOutput::success("committed replay")),
            Ok(CommandOutput::success("replayed")),
            Ok(observation_output(g2)),
            Ok(logical_export_output(&["winner", "replay"])),
            Ok(CommandOutput::success(format!(
                "{g0}\tbase\n{g1}\toperation:winner\n{g2}\toperation:replay\n"
            ))),
        ]),
        "transport-retries" => Ok(vec![
            Ok(CommandOutput::success("wrote retry")),
            Ok(CommandOutput::success("committed retry")),
            Ok(observation_output(g0)),
            Err("connection reset".into()),
            Ok(observation_output(g0)),
            Ok(CommandOutput::success("published")),
            Ok(observation_output(g1)),
            Ok(CommandOutput::success(format!(
                "{g0}\tbase\n{g1}\toperation:retry\n"
            ))),
            Ok(logical_export_output(&["retry"])),
            Ok(CommandOutput::success("wrote lost-response")),
            Ok(CommandOutput::success("committed lost-response")),
            Ok(observation_output(g0)),
            Err("connection reset".into()),
            Ok(observation_output(g1)),
            Ok(CommandOutput::success(format!(
                "{g0}\tbase\n{g1}\toperation:lost-response\n"
            ))),
            Ok(logical_export_output(&["lost-response"])),
        ]),
        _ => Err("cutover_blocked".into()),
    }
}
fn observation_output(generation: &str) -> CommandOutput {
    CommandOutput::success(format!("{generation}\trefs/dolt/data\n"))
}
fn logical_export_output(operations: &[&str]) -> CommandOutput {
    CommandOutput::success(
        operations
            .iter()
            .map(|operation| {
                format!(
                    "{{\"id\":\"issue-{operation}\",\"title\":\"operation:{operation}\",\"description\":\"issue:{operation}\"}}"
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

pub fn run_scripted_case<R: CommandRunner>(
    case: &str,
    runner: &mut R,
) -> Result<ScriptEvidence, String> {
    match case {
        "stale-base-fence" => stale_base_recovery(runner, true),
        "push-conflict-recovery" => stale_base_recovery(runner, false),
        "transport-retries" => {
            replay_operation(runner, "retry")?;
            let (observed_base, result) = retry_after_transport_with_base(runner, "retry")?;
            let Publication::Published { .. } = result else {
                return Err("cutover_blocked".into());
            };
            validate_observed_export(runner, &["retry"])?;
            replay_operation(runner, "lost-response")?;
            let (_, recovered) = recover_after_lost_response_with_base(runner, "lost-response")?;
            let Publication::Recovered {
                generation: recovered_generation,
                ..
            } = recovered
            else {
                return Err("cutover_blocked".into());
            };
            validate_observed_export(runner, &["lost-response"])?;
            Ok(ScriptEvidence {
                observed_base,
                final_generation: recovered_generation,
                operation_ids: vec!["retry".into(), "lost-response".into()],
            })
        }
        _ => Err("cutover_blocked".into()),
    }
}

fn stale_base_recovery<R: CommandRunner>(
    runner: &mut R,
    includes_paused_holder: bool,
) -> Result<ScriptEvidence, String> {
    let winner_base = observe(runner)?;
    let stale_base = observe(runner)?;
    if winner_base != stale_base {
        return Err("cutover_blocked".into());
    }
    replay_operation(runner, "winner")?;
    let winner_generation = publish_after_observation(runner, &winner_base)?;
    let stale_push = execute_publication_command(runner, publish_command(), &stale_base)
        .map_err(|_| "cutover_blocked".to_owned())?;
    if stale_push.status == 0 || classify_push(&stale_push.stderr) != PushFailure::StaleBase {
        return Err("cutover_blocked".into());
    }
    let observed_after_stale = observe(runner)?;
    if observed_after_stale != winner_generation {
        return Err("cutover_blocked".into());
    }
    let refresh = runner
        .run(refresh_command())
        .map_err(|_| "cutover_blocked".to_owned())?;
    if refresh.status != 0 {
        return Err("cutover_blocked".into());
    }
    replay_operation(runner, "replay")?;
    let recovery_generation = publish_after_observation(runner, &observed_after_stale)?;
    if includes_paused_holder {
        let paused_push =
            execute_publication_command(runner, publish_command(), &winner_generation)
                .map_err(|_| "cutover_blocked".to_owned())?;
        if paused_push.status == 0 || classify_push(&paused_push.stderr) != PushFailure::StaleBase {
            return Err("cutover_blocked".into());
        }
        if observe(runner)? != recovery_generation {
            return Err("cutover_blocked".into());
        }
    }
    validate_observed_export(runner, &["winner", "replay"])?;
    validate_scripted_history(
        runner,
        &[&winner_base, &winner_generation, &recovery_generation],
        &["winner", "replay"],
    )?;
    Ok(ScriptEvidence {
        observed_base: winner_base,
        final_generation: recovery_generation,
        operation_ids: vec!["winner".into(), "replay".into()],
    })
}

pub fn run_scripted_contract_case(
    case: &str,
    runner: &mut RecordingCommandRunner,
) -> Result<ContractResult, String> {
    let evidence = run_scripted_case(case, runner)?;
    let command_plans = runner
        .commands()
        .iter()
        .map(CommandSpec::display)
        .collect::<Vec<_>>();
    Ok(ContractResult {
        case: case.into(),
        outcome: "passed".into(),
        code: "ok".into(),
        beads_version: "1.1.2".into(),
        clone_labels: vec!["clone-a".into(), "clone-b".into()],
        observed_base: Some(evidence.observed_base),
        final_generation: Some(evidence.final_generation),
        operation_ids: evidence.operation_ids,
        command_plans,
        scenarios: Vec::new(),
        source_ref: None,
        source_commit: None,
        document_counts: None,
        total_document_count: None,
        logical_export_sha256: None,
        authority_mode: None,
        offending_key: None,
        mismatch: None,
    })
}

fn publish_after_observation<R: CommandRunner>(
    runner: &mut R,
    observed_base: &str,
) -> Result<String, String> {
    let output = execute_publication_command(runner, publish_command(), observed_base)
        .map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    observe(runner)
}

pub fn embedded_cleanup_commands() -> Vec<CommandSpec> {
    Vec::new()
}

pub fn assert_no_ls_remote(commands: &[CommandSpec]) -> Result<(), &'static str> {
    if commands.iter().any(|command| {
        command.program == Path::new("git")
            && command.argv.first().map(String::as_str) == Some("ls-remote")
    }) {
        Err("cutover_blocked")
    } else {
        Ok(())
    }
}

fn run_hermetic_command(
    runner: &mut SystemCommandRunner,
    executed: &mut Vec<CommandSpec>,
    command: CommandSpec,
) -> Result<CommandOutput, &'static str> {
    assert_no_ls_remote(std::slice::from_ref(&command))?;
    executed.push(command.clone());
    runner.run(command).map_err(|_| "cutover_blocked")
}

fn init_store(binary: &Path, root: &Path, label: &str) -> Result<StoreFixture, &'static str> {
    let mut fixture = prepare_store_fixture(root, label)?;
    let repository = fixture.repository.clone();
    let environment = fixture.environment.clone();
    let mut runner = SystemCommandRunner;
    let mut executed = Vec::new();
    for argv in [
        vec!["init".into()],
        vec![
            "config".into(),
            "user.email".into(),
            "fixture@example.invalid".into(),
        ],
        vec!["config".into(), "user.name".into(), "fixture".into()],
        vec![
            "add".into(),
            "tracked.txt".into(),
            "AGENTS.md".into(),
            "CLAUDE.md".into(),
        ],
        vec![
            "commit".into(),
            "--quiet".into(),
            "-m".into(),
            "fixture".into(),
        ],
    ] {
        let output = run_hermetic_command(
            &mut runner,
            &mut executed,
            CommandSpec {
                program: PathBuf::from("git"),
                argv,
                cwd: Some(repository.clone()),
                environment: environment.clone(),
                redacted_argv_positions: Vec::new(),
            },
        )?;
        if output.status != 0 {
            return Err("cutover_blocked");
        }
    }
    let hook = repository.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 0\n").map_err(|_| "cutover_blocked")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
            .map_err(|_| "cutover_blocked")?;
    }
    fixture.snapshot_git_state()?;
    let mut init_environment = environment.clone();
    init_environment.insert("PWD".into(), repository.display().to_string());
    let output = run_hermetic_command(
        &mut runner,
        &mut executed,
        CommandSpec {
            program: binary.to_path_buf(),
            argv: vec![
                "--sandbox".into(),
                "init".into(),
                "--stealth".into(),
                "--skip-agents".into(),
                "--skip-hooks".into(),
                "--non-interactive".into(),
            ],
            cwd: Some(repository.clone()),
            environment: init_environment,
            redacted_argv_positions: Vec::new(),
        },
    )?;
    if output.status != 0 {
        return Err("cutover_blocked");
    }
    fixture.assert_after_stealth_init()?;
    let output = run_hermetic_command(
        &mut runner,
        &mut executed,
        CommandSpec {
            program: PathBuf::from("git"),
            argv: vec!["config".into(), "dolt.auto-push".into(), "false".into()],
            cwd: Some(repository.clone()),
            environment: environment.clone(),
            redacted_argv_positions: Vec::new(),
        },
    )?;
    if output.status != 0 {
        return Err("cutover_blocked");
    }
    let output = run_hermetic_command(
        &mut runner,
        &mut executed,
        CommandSpec {
            program: PathBuf::from("git"),
            argv: vec!["config".into(), "--get".into(), "dolt.auto-push".into()],
            cwd: Some(repository.clone()),
            environment: environment.clone(),
            redacted_argv_positions: Vec::new(),
        },
    )?;
    if output.status != 0 || output.stdout.trim() != "false" {
        return Err("cutover_blocked");
    }
    for argv in [
        vec!["status".into(), "--porcelain".into()],
        vec!["diff".into(), "--cached".into(), "--quiet".into()],
    ] {
        let output = run_hermetic_command(
            &mut runner,
            &mut executed,
            CommandSpec {
                program: PathBuf::from("git"),
                argv,
                cwd: Some(repository.clone()),
                environment: environment.clone(),
                redacted_argv_positions: Vec::new(),
            },
        )?;
        if output.status != 0 || !output.stdout.is_empty() {
            return Err("cutover_blocked");
        }
    }
    if !fixture.store_root.is_dir() || contains_metrics_or_events(&fixture.clone_root)? {
        return Err("cutover_blocked");
    }
    assert_no_ls_remote(&executed)?;
    Ok(fixture)
}

fn contains_metrics_or_events(root: &Path) -> Result<bool, &'static str> {
    for entry in std::fs::read_dir(root).map_err(|_| "cutover_blocked")? {
        let entry = entry.map_err(|_| "cutover_blocked")?;
        let name = entry.file_name();
        let name = name.to_string_lossy().to_ascii_lowercase();
        if name.contains("metrics") || name.contains("event") {
            return Ok(true);
        }
        if entry.file_type().map_err(|_| "cutover_blocked")?.is_dir()
            && contains_metrics_or_events(&entry.path())?
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn host_target() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else {
        "unsupported"
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ContractParityMismatch, ContractRequest, ContractResult, HISTORICAL_SOURCE_COMMIT,
        OnlineSyncContractScenario, OnlineSyncContractTransport, contract_refusal_exit_code,
        finish_contract, manifest_path, parity_candidate_projection, resolve_changed_source_ref,
        run_contract, source_refusal,
    };
    use crate::command::{CommandOutput, CommandRunner, CommandSpec, RecordingCommandRunner};
    use crate::document::parse_document;
    use crate::shadow::{ActiveOwner, initial_operational_metadata, to_operational_beads_jsonl};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static PROCESS_WORKING_DIRECTORY: Mutex<()> = Mutex::new(());

    #[test]
    fn changed_source_refusal_requires_a_locally_available_different_commit() {
        let source_root = PathBuf::from("/contract-source");
        let selected = "a".repeat(40);
        let alternate = "b".repeat(40);
        let mut runner = RecordingCommandRunner::scripted(vec![Ok(CommandOutput::success(
            format!("{alternate}\n"),
        ))]);

        assert_eq!(
            resolve_changed_source_ref(&mut runner, &source_root, &selected).unwrap(),
            alternate
        );
        assert_eq!(
            runner.commands(),
            &[CommandSpec {
                program: PathBuf::from("git"),
                argv: vec![
                    "rev-parse".into(),
                    "--verify".into(),
                    "--end-of-options".into(),
                    format!("{HISTORICAL_SOURCE_COMMIT}^{{commit}}"),
                ],
                cwd: Some(source_root.clone()),
                environment: BTreeMap::from([
                    ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                    ("GIT_TERMINAL_PROMPT".into(), "0".into()),
                    ("GIT_NO_LAZY_FETCH".into(), "1".into()),
                    ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
                    ("BD_DISABLE_METRICS".into(), "1".into()),
                    ("BD_DISABLE_EVENT_FLUSH".into(), "1".into()),
                    ("BD_NON_INTERACTIVE".into(), "1".into()),
                    ("CI".into(), "true".into()),
                    ("HOME".into(), "/contract-source/home".into()),
                    (
                        "XDG_CONFIG_HOME".into(),
                        "/contract-source/xdg_config_home".into(),
                    ),
                    (
                        "XDG_CACHE_HOME".into(),
                        "/contract-source/xdg_cache_home".into(),
                    ),
                    (
                        "XDG_DATA_HOME".into(),
                        "/contract-source/xdg_data_home".into(),
                    ),
                    ("TMPDIR".into(), "/contract-source/tmpdir".into()),
                    (
                        "GIT_CONFIG_GLOBAL".into(),
                        "/contract-source/git_config_global".into(),
                    ),
                    ("PATH".into(), std::env::var("PATH").unwrap()),
                ]),
                redacted_argv_positions: Vec::new(),
            }],
        );
        assert!(runner.finish().is_ok());

        let mut selected_historical = RecordingCommandRunner::scripted(vec![Ok(
            CommandOutput::success(format!("{alternate}\n")),
        )]);
        assert_eq!(
            resolve_changed_source_ref(
                &mut selected_historical,
                &source_root,
                HISTORICAL_SOURCE_COMMIT,
            )
            .unwrap(),
            alternate
        );
        assert_eq!(
            selected_historical.commands()[0].argv.last(),
            Some(&"origin/main^{commit}".to_owned())
        );
        assert!(selected_historical.finish().is_ok());

        for output in [
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "missing".into(),
            },
            CommandOutput::success(format!("{selected}\n")),
            CommandOutput::success(format!("{}\n", "B".repeat(40))),
            CommandOutput::success(format!("{alternate}\nextra\n")),
        ] {
            let mut refusal = RecordingCommandRunner::scripted(vec![Ok(output)]);
            assert_eq!(
                resolve_changed_source_ref(&mut refusal, &source_root, &selected).unwrap_err(),
                "cutover_blocked"
            );
            assert_eq!(refusal.commands().len(), 1);
            assert!(refusal.finish().is_ok());
        }
    }

    #[test]
    fn fixture_cleanup_failure_takes_precedence_over_an_operation_refusal() {
        let result = finish_contract(
            Err(Box::new(ContractResult::refusal(
                "version-pin",
                "beads_checksum_mismatch",
            ))),
            Err("fixture_cleanup_failed"),
            "version-pin",
        )
        .unwrap_err();

        assert_eq!(result.code, "fixture_cleanup_failed");
    }

    #[test]
    fn cleanup_failure_refuses_an_otherwise_successful_contract() {
        let result = finish_contract(
            Ok(ContractResult::passed("version-pin", Vec::new())),
            Err("fixture_cleanup_failed"),
            "version-pin",
        )
        .unwrap_err();

        assert_eq!(result.code, "fixture_cleanup_failed");
    }

    #[test]
    fn source_refusal_serializes_an_offending_document_key() {
        let result = source_refusal(
            "document-mapping",
            "selected-ref",
            "invalid_document",
            Some("task:045".into()),
            None,
        );
        let value = serde_json::to_value(&result).unwrap();

        assert_eq!(value["source_ref"], "selected-ref");
        assert_eq!(value["offending_key"], "task:045");
        assert_eq!(contract_refusal_exit_code(&result.code), 1);
    }

    #[test]
    fn legacy_contract_normalizes_an_unavailable_source_ref() {
        let result = source_refusal(
            "document-mapping",
            "refs/heads/definitely-missing",
            "source_ref_unavailable",
            None,
            None,
        );

        assert_eq!(result.code, "invalid_source_ref");
        assert_eq!(contract_refusal_exit_code(&result.code), 1);
    }

    #[test]
    fn pin_manifest_path_is_independent_of_the_process_working_directory() {
        let path = manifest_path();

        assert!(path.is_absolute());
        assert!(path.ends_with("tools/work-state-beads-1.1.2.toml"));
        assert!(path.is_file());
    }

    #[test]
    fn run_contract_loads_the_pin_outside_the_process_working_directory() {
        let _working_directory = PROCESS_WORKING_DIRECTORY.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("wrong-archive");
        std::fs::write(&archive, "wrong archive").unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(root.path()).unwrap();
        let result = run_contract(&ContractRequest {
            case: "version-pin".into(),
            source_ref: None,
            archive,
            binary: root.path().join("bd"),
        });
        std::env::set_current_dir(original).unwrap();

        assert_eq!(result.unwrap_err().code, "beads_checksum_mismatch");
    }

    #[test]
    fn online_sync_contract_transport_materializes_only_an_exact_fresh_init() {
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join("staging");
        let repository = staging.join("repository");
        let binary = staging.join("bd");
        let candidate = root.path().join("recorded-remote-candidate");
        std::fs::create_dir_all(&repository).unwrap();
        std::fs::create_dir_all(&candidate).unwrap();
        std::fs::write(&binary, "staged bd").unwrap();
        std::fs::write(candidate.join("remote-only"), "recorded candidate").unwrap();
        let project = crate::project::compiled_project_config().unwrap();
        let mut transport =
            OnlineSyncContractTransport::stable(project.clone(), candidate, "b".repeat(40));
        let environment = BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]);
        let observation = CommandSpec {
            program: PathBuf::from("git"),
            argv: vec![
                "ls-remote".into(),
                "--exit-code".into(),
                project.git_observation_url().into(),
                project.data_ref().into(),
            ],
            cwd: Some(staging.clone()),
            environment: environment.clone(),
            redacted_argv_positions: vec![2],
        };
        assert_eq!(
            transport.run(observation).unwrap(),
            CommandOutput::success(format!("{}\trefs/dolt/data\n", "b".repeat(40)))
        );
        let mut argv = vec![
            "--sandbox".into(),
            "init".into(),
            "--remote".into(),
            "git+https://example.invalid/plasmosome.git".into(),
            "--stealth".into(),
            "--skip-agents".into(),
            "--skip-hooks".into(),
            "--non-interactive".into(),
        ];
        let wrong = CommandSpec {
            program: binary.clone(),
            argv: argv.clone(),
            cwd: Some(repository.clone()),
            environment: environment.clone(),
            redacted_argv_positions: vec![3],
        };
        assert!(transport.run(wrong).is_err());
        assert!(std::fs::read_dir(&repository).unwrap().next().is_none());

        argv[3] = project.dolt_remote_url().into();
        let exact = CommandSpec {
            program: binary,
            argv,
            cwd: Some(repository.clone()),
            environment,
            redacted_argv_positions: vec![3],
        };
        assert_eq!(transport.run(exact).unwrap(), CommandOutput::success(""));
        assert_eq!(
            std::fs::read_to_string(repository.join("remote-only")).unwrap(),
            "recorded candidate"
        );
    }

    #[test]
    fn online_sync_contract_transport_records_only_fixed_admitted_remote_outcomes() {
        fn command(
            program: PathBuf,
            argv: Vec<String>,
            cwd: PathBuf,
            environment: BTreeMap<String, String>,
            redacted_argv_positions: Vec<usize>,
        ) -> CommandSpec {
            CommandSpec {
                program,
                argv,
                cwd: Some(cwd),
                environment,
                redacted_argv_positions,
            }
        }

        let root = tempfile::tempdir().unwrap();
        let project = crate::project::compiled_project_config().unwrap();
        let environment = BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]);
        for (scenario, expected_first_status, expected_init_status, expected_list, expected_r1) in [
            (
                OnlineSyncContractScenario::FirstTransport,
                1,
                None,
                None,
                None,
            ),
            (
                OnlineSyncContractScenario::FirstMalformed,
                0,
                None,
                None,
                None,
            ),
            (OnlineSyncContractScenario::FirstMoved, 0, None, None, None),
            (
                OnlineSyncContractScenario::InitTransport,
                0,
                Some(1),
                None,
                None,
            ),
            (
                OnlineSyncContractScenario::RemoteListTransport,
                0,
                Some(0),
                Some(1),
                None,
            ),
            (
                OnlineSyncContractScenario::RemoteListMismatch,
                0,
                Some(0),
                Some(0),
                None,
            ),
            (
                OnlineSyncContractScenario::SecondTransport,
                0,
                Some(0),
                Some(0),
                Some(1),
            ),
            (
                OnlineSyncContractScenario::SecondNoMatch,
                0,
                Some(0),
                Some(0),
                Some(2),
            ),
            (
                OnlineSyncContractScenario::SecondMalformed,
                0,
                Some(0),
                Some(0),
                Some(0),
            ),
            (
                OnlineSyncContractScenario::SecondMoved,
                0,
                Some(0),
                Some(0),
                Some(0),
            ),
        ] {
            let staging = root.path().join(format!("staging-{scenario:?}"));
            let repository = staging.join("repository");
            let binary = staging.join("bd");
            let candidate = root.path().join(format!("candidate-{scenario:?}"));
            std::fs::create_dir_all(&repository).unwrap();
            std::fs::create_dir_all(&candidate).unwrap();
            std::fs::write(&binary, "staged bd").unwrap();
            std::fs::write(candidate.join("remote-only"), "recorded candidate").unwrap();
            let mut transport =
                OnlineSyncContractTransport::for_scenario(project.clone(), candidate, scenario);
            let observation = || {
                command(
                    PathBuf::from("git"),
                    vec![
                        "ls-remote".into(),
                        "--exit-code".into(),
                        project.git_observation_url().into(),
                        project.data_ref().into(),
                    ],
                    staging.clone(),
                    environment.clone(),
                    vec![2],
                )
            };
            let first = transport.run(observation()).unwrap();
            assert_eq!(first.status, expected_first_status, "{scenario:?}");
            if expected_init_status.is_none() {
                assert!(transport.run(observation()).is_err(), "{scenario:?}");
                assert!(std::fs::read_dir(&repository).unwrap().next().is_none());
                continue;
            }
            let init = command(
                binary.clone(),
                vec![
                    "--sandbox".into(),
                    "init".into(),
                    "--remote".into(),
                    project.dolt_remote_url().into(),
                    "--stealth".into(),
                    "--skip-agents".into(),
                    "--skip-hooks".into(),
                    "--non-interactive".into(),
                ],
                repository.clone(),
                environment.clone(),
                vec![3],
            );
            let init_output = transport.run(init).unwrap();
            assert_eq!(
                init_output.status,
                expected_init_status.unwrap(),
                "{scenario:?}"
            );
            if expected_list.is_none() {
                assert!(transport.run(observation()).is_err(), "{scenario:?}");
                assert!(std::fs::read_dir(&repository).unwrap().next().is_none());
                continue;
            }
            let list = command(
                binary,
                vec![
                    "--sandbox".into(),
                    "--json".into(),
                    "dolt".into(),
                    "remote".into(),
                    "list".into(),
                ],
                repository.clone(),
                environment.clone(),
                Vec::new(),
            );
            let list_output = transport.run(list).unwrap();
            assert_eq!(list_output.status, expected_list.unwrap(), "{scenario:?}");
            if expected_r1.is_none() {
                assert!(transport.run(observation()).is_err(), "{scenario:?}");
                continue;
            }
            let second = transport.run(observation()).unwrap();
            assert_eq!(second.status, expected_r1.unwrap(), "{scenario:?}");
            assert!(std::fs::read_dir(&repository).unwrap().next().is_some());
        }
    }

    #[cfg(unix)]
    #[test]
    fn online_sync_contract_transport_admits_only_metadata_version_after_pending_refusal() {
        use std::os::unix::fs::PermissionsExt;

        let _working_directory = PROCESS_WORKING_DIRECTORY.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let generations = root.path().join("generations");
        let staging = generations.join(".staging-observation");
        let repository = staging.join("repository");
        let binary = staging.join("bd");
        let candidate = root.path().join("recorded-remote-candidate");
        std::fs::create_dir_all(&repository).unwrap();
        std::fs::create_dir_all(&candidate).unwrap();
        std::fs::write(&binary, "staged bd").unwrap();
        let project = crate::project::compiled_project_config().unwrap();
        let environment = BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]);
        let mut transport = OnlineSyncContractTransport::for_scenario(
            project.clone(),
            candidate,
            OnlineSyncContractScenario::FirstMoved,
        );
        let observation = CommandSpec {
            program: PathBuf::from("git"),
            argv: vec![
                "ls-remote".into(),
                "--exit-code".into(),
                project.git_observation_url().into(),
                project.data_ref().into(),
            ],
            cwd: Some(staging),
            environment: environment.clone(),
            redacted_argv_positions: vec![2],
        };
        assert_eq!(transport.run(observation).unwrap().status, 0);

        let metadata_binary = generations.join(".staging-pending-metadata/bd");
        std::fs::create_dir_all(metadata_binary.parent().unwrap()).unwrap();
        std::fs::write(
            &metadata_binary,
            "#!/bin/sh\nprintf 'bd version 1.1.2 (test)\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&metadata_binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let wrong = CommandSpec {
            program: metadata_binary.clone(),
            argv: vec!["--readonly".into(), "--sandbox".into(), "export".into()],
            cwd: None,
            environment: environment.clone(),
            redacted_argv_positions: Vec::new(),
        };
        assert!(transport.run(wrong).is_err());
        let version = CommandSpec {
            program: metadata_binary,
            argv: vec!["--version".into()],
            cwd: None,
            environment,
            redacted_argv_positions: Vec::new(),
        };
        assert_eq!(
            transport.run(version).unwrap(),
            CommandOutput::success("bd version 1.1.2 (test)\n")
        );
    }

    #[cfg(unix)]
    #[test]
    fn installed_config_git_shim_admits_only_bound_local_beads_discovery() {
        use super::{
            GitShimCategory, InstalledGitShimBinding, read_installed_git_shim_records,
            resolve_absolute_git, write_installed_config_git_shim,
        };
        use crate::command::SystemCommandRunner;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        fn runtime_environment(root: &std::path::Path, path: String) -> BTreeMap<String, String> {
            let runtime = root.join("runtime");
            for name in ["home", "xdg_config", "xdg_cache", "xdg_data", "tmp"] {
                fs::create_dir_all(runtime.join(name)).unwrap();
            }
            fs::write(runtime.join("git_config_global"), "").unwrap();
            BTreeMap::from([
                ("PATH".into(), path),
                ("HOME".into(), runtime.join("home").display().to_string()),
                (
                    "XDG_CONFIG_HOME".into(),
                    runtime.join("xdg_config").display().to_string(),
                ),
                (
                    "XDG_CACHE_HOME".into(),
                    runtime.join("xdg_cache").display().to_string(),
                ),
                (
                    "XDG_DATA_HOME".into(),
                    runtime.join("xdg_data").display().to_string(),
                ),
                ("TMPDIR".into(), runtime.join("tmp").display().to_string()),
                (
                    "GIT_CONFIG_GLOBAL".into(),
                    runtime.join("git_config_global").display().to_string(),
                ),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                ("GIT_TERMINAL_PROMPT".into(), "0".into()),
                ("GIT_NO_LAZY_FETCH".into(), "1".into()),
                ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
                ("BD_DISABLE_METRICS".into(), "1".into()),
                ("BD_DISABLE_EVENT_FLUSH".into(), "1".into()),
                ("BD_NON_INTERACTIVE".into(), "1".into()),
                ("CI".into(), "true".into()),
            ])
        }

        fn command(
            argv: Vec<String>,
            cwd: PathBuf,
            environment: BTreeMap<String, String>,
        ) -> CommandSpec {
            CommandSpec {
                program: PathBuf::from("git"),
                argv,
                cwd: Some(cwd),
                environment,
                redacted_argv_positions: Vec::new(),
            }
        }

        let root = tempfile::tempdir().unwrap();
        let fake_bin = root.path().join("bin");
        let worktree = root.path().join("worktree");
        let common = root.path().join("common");
        let disposable = root.path().join("disposable");
        let repository = disposable.join("repository");
        fs::create_dir_all(&fake_bin).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::create_dir_all(&common).unwrap();
        fs::create_dir_all(&repository).unwrap();
        let original_path = std::env::var_os("PATH").unwrap();
        let real_git = resolve_absolute_git(&original_path).unwrap();
        let original_path = original_path.to_string_lossy().into_owned();
        let path = format!("{}:{original_path}", fake_bin.display());
        let mut runner = SystemCommandRunner;
        assert_eq!(
            runner
                .run(CommandSpec {
                    program: real_git.clone(),
                    argv: vec!["init".into(), "--quiet".into()],
                    cwd: Some(repository.clone()),
                    environment: BTreeMap::from([("PATH".into(), original_path.clone())]),
                    redacted_argv_positions: Vec::new(),
                })
                .unwrap()
                .status,
            0
        );
        let environment = runtime_environment(&disposable, path.clone());
        let shim = fake_bin.join("git");
        let capture = root.path().join("capture");
        let binding = InstalledGitShimBinding {
            real_git,
            capture: capture.clone(),
            worktree: fs::canonicalize(&worktree).unwrap(),
            common_dir: fs::canonicalize(&common).unwrap(),
            locator_path: path,
            project: crate::project::compiled_project_config().unwrap(),
        };
        write_installed_config_git_shim(&shim, &binding).unwrap();
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o700)).unwrap();

        let discovery = vec![
            "-C".into(),
            repository.display().to_string(),
            "rev-parse".into(),
            "--git-dir".into(),
            "--git-common-dir".into(),
        ];
        let output = runner
            .run(command(
                discovery.clone(),
                repository.clone(),
                environment.clone(),
            ))
            .unwrap();
        assert_eq!(output.status, 0);
        assert!(!output.stdout.is_empty());

        let mut wrong_environment = environment.clone();
        wrong_environment.remove("CI");
        let mut wrong_runtime = environment.clone();
        wrong_runtime.insert(
            "TMPDIR".into(),
            root.path().join("other/runtime/tmp").display().to_string(),
        );
        let mut proxy_environment = environment.clone();
        proxy_environment.insert("HTTPS_PROXY".into(), "sentinel".into());
        let cases = vec![
            command(discovery.clone(), disposable.clone(), environment.clone()),
            command(
                vec![
                    "-C".into(),
                    disposable.join("other").display().to_string(),
                    "rev-parse".into(),
                    "--git-dir".into(),
                    "--git-common-dir".into(),
                ],
                repository.clone(),
                environment.clone(),
            ),
            command(
                discovery[..4].to_vec(),
                repository.clone(),
                environment.clone(),
            ),
            command(
                vec![
                    "-C".into(),
                    repository.display().to_string(),
                    "rev-parse".into(),
                    "--git-common-dir".into(),
                    "--git-dir".into(),
                ],
                repository.clone(),
                environment.clone(),
            ),
            command(
                [discovery.clone(), vec!["extra".into()]].concat(),
                repository.clone(),
                environment.clone(),
            ),
            command(discovery.clone(), repository.clone(), wrong_environment),
            command(discovery.clone(), repository.clone(), wrong_runtime),
            command(discovery.clone(), repository.clone(), proxy_environment),
            command(
                vec![
                    "ls-remote".into(),
                    "--exit-code".into(),
                    "https://example.invalid/plasmosome.git".into(),
                    "refs/dolt/data".into(),
                ],
                disposable.clone(),
                environment.clone(),
            ),
            command(
                vec![
                    "-C".into(),
                    repository.display().to_string(),
                    "config".into(),
                    "--get".into(),
                    "remote.origin.url".into(),
                ],
                repository.clone(),
                environment.clone(),
            ),
            command(
                vec!["remote".into()],
                repository.clone(),
                environment.clone(),
            ),
            command(
                vec!["fetch".into()],
                repository.clone(),
                environment.clone(),
            ),
            command(vec!["pull".into()], repository.clone(), environment.clone()),
            command(vec!["push".into()], repository.clone(), environment.clone()),
            command(
                vec!["update-ref".into(), "refs/dolt/data".into(), "a".repeat(40)],
                repository.clone(),
                environment,
            ),
        ];
        for case in cases {
            assert_eq!(runner.run(case).unwrap().status, 97);
        }
        let records = read_installed_git_shim_records(&capture).unwrap();
        assert_eq!(records.len(), 1 + 15);
        assert_eq!(records[0].category, GitShimCategory::LocalBeadsDiscovery);
        assert_eq!(records[0].cwd, fs::canonicalize(&repository).unwrap());
        assert_eq!(records[0].argv, discovery);
        assert!(
            records[1..]
                .iter()
                .all(|record| record.category == GitShimCategory::Unexpected)
        );
    }

    #[test]
    fn contract_parity_candidates_are_valid_shadows_with_one_representative_difference() {
        let source = "a".repeat(40);
        let documents = vec![
            parse_document(
                "docs/intents/001-intent.md",
                "---\nid: 001\ntitle: Intent\nstatus: approved\n---\n",
                &source,
            )
            .unwrap(),
            parse_document(
                "docs/specs/001-spec.md",
                "---\nid: 001\ntitle: Spec\nstatus: accepted\nintents: [001]\n---\n",
                &source,
            )
            .unwrap(),
            parse_document(
                "tasks/001-first.md",
                "---\nid: 001\ntitle: First Task\nstatus: planned\npriority: 1\nintents: [001]\nspecs: [001]\n---\n",
                &source,
            )
            .unwrap(),
            parse_document(
                "tasks/002-second.md",
                "---\nid: 002\ntitle: Second Task\nstatus: planned\npriority: 2\nintents: [001]\nspecs: [001]\n---\n",
                &source,
            )
            .unwrap(),
        ];
        let operational = initial_operational_metadata(&documents).unwrap();
        for mismatch in [
            ContractParityMismatch::Authority,
            ContractParityMismatch::Source,
            ContractParityMismatch::Logical,
            ContractParityMismatch::Operational,
            ContractParityMismatch::Missing,
            ContractParityMismatch::Extra,
            ContractParityMismatch::UnknownKeyValue,
        ] {
            let (candidate_documents, candidate_operational, key_value) =
                parity_candidate_projection(mismatch, &documents, &operational).unwrap();
            to_operational_beads_jsonl(&candidate_documents, &candidate_operational).unwrap();
            match mismatch {
                ContractParityMismatch::Authority => assert_eq!(
                    key_value,
                    Some(("plasmosome.authority-mode", "ledger".into()))
                ),
                ContractParityMismatch::Source => assert_eq!(
                    key_value,
                    Some(("plasmosome.source-commit", "b".repeat(40)))
                ),
                ContractParityMismatch::Logical => {
                    assert_eq!(candidate_documents[2].record.title, "Changed remotely")
                }
                ContractParityMismatch::Operational => assert_eq!(
                    candidate_operational["task:001"].active_owner,
                    Some(ActiveOwner {
                        actor: "remote-owner".into(),
                        session_id: "remote-session".into(),
                        ownership_token: "remote-token".into(),
                        claim_operation_id: "remote-claim".into(),
                        acquired_at: "2026-09-02T12:00:00Z".into(),
                        expires_at: "2026-09-02T13:00:00Z".into(),
                    })
                ),
                ContractParityMismatch::Missing => assert_eq!(
                    candidate_documents
                        .iter()
                        .map(|document| document.record.document_key.as_str())
                        .collect::<Vec<_>>(),
                    vec!["intent:001", "spec:001", "task:001"]
                ),
                ContractParityMismatch::Extra => assert_eq!(
                    candidate_documents.last().unwrap().record.document_key,
                    "task:999"
                ),
                ContractParityMismatch::UnknownKeyValue => {
                    assert_eq!(key_value, Some(("plasmosome.writer", "forbidden".into())))
                }
            }
        }
    }
}
