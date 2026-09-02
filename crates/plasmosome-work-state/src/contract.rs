use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use tempfile::TempDir;

use crate::command::{
    CommandOutput, CommandRunner, CommandSpec, RecordingCommandRunner, SystemCommandRunner,
};
use crate::document::{
    DocumentError, DocumentKind, ShadowDocument, SourceDocuments, load_documents,
};
use crate::pin::{PinManifest, VerifiedBeads};
use crate::shadow::{
    ShadowError, ShadowStore, canonical_logical_export, compare_document_mapping,
    compare_shadow_parity, decode_logical_export, import_shadow_documents, logical_export_digest,
};

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
    /// Numeric intent documents in the selected source commit.
    pub intent: usize,
    /// Numeric spec documents in the selected source commit.
    pub spec: usize,
    /// Numeric task documents in the selected source commit.
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

/// Lists the evidence categories that a requested contract case must execute.
pub fn contract_case_names(case: &str) -> Vec<&'static str> {
    match case {
        "all" => vec!["hermetic", "transport", "document-mapping", "shadow-parity"],
        "transport" => vec!["transport"],
        "document-mapping" => vec!["document-mapping"],
        "shadow-parity" => vec!["shadow-parity"],
        "hermetic" => vec!["hermetic"],
        "version-pin" => vec!["version-pin"],
        "stealth-init" => vec!["stealth-init"],
        "stale-base-fence" => vec!["stale-base-fence"],
        "push-conflict-recovery" => vec!["push-conflict-recovery"],
        "transport-retries" => vec!["transport-retries"],
        _ => Vec::new(),
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
        if value.is_empty() || value.starts_with("--") {
            return Err("invalid_command".into());
        }
        match flag.as_str() {
            "--archive" if archive.is_none() => archive = Some(PathBuf::from(value)),
            "--bd" if binary.is_none() => binary = Some(PathBuf::from(value)),
            "--source-ref"
                if source_ref.is_none()
                    && matches!(case.as_str(), "all" | "document-mapping" | "shadow-parity") =>
            {
                source_ref = Some(value.to_owned())
            }
            _ => return Err("invalid_command".into()),
        };
        index += 2;
    }
    let source_ref = match case.as_str() {
        "document-mapping" | "shadow-parity" => {
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

fn run_shadow_round_trip(
    runner: &mut SystemCommandRunner,
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
        if matches!(
            request.case.as_str(),
            "document-mapping" | "shadow-parity" | "all"
        ) {
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
            if request.case == "all" {
                let transport = run_scripted_cases("transport")
                    .map_err(|code| Box::new(ContractResult::refusal(&request.case, code)))?;
                result.observed_base = transport.observed_base;
                result.final_generation = transport.final_generation;
                result.operation_ids = transport.operation_ids;
                result.command_plans.extend(transport.command_plans);
                result.scenarios = transport.scenarios;
            }
            return Ok(result);
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
    match (cleanup, result) {
        (Ok(()), result) => result,
        (Err(_), Err(result)) => Err(result),
        (Err(code), Ok(_)) => Err(Box::new(ContractResult::refusal(case, code))),
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
        ContractRequest, ContractResult, contract_refusal_exit_code, finish_contract,
        manifest_path, run_contract, source_refusal,
    };
    use std::sync::Mutex;

    static PROCESS_WORKING_DIRECTORY: Mutex<()> = Mutex::new(());

    #[test]
    fn cleanup_failure_preserves_an_earlier_refusal() {
        let result = finish_contract(
            Err(Box::new(ContractResult::refusal(
                "version-pin",
                "beads_checksum_mismatch",
            ))),
            Err("fixture_cleanup_failed"),
            "version-pin",
        )
        .unwrap_err();

        assert_eq!(result.code, "beads_checksum_mismatch");
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
}
