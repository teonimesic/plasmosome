use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::command::{CommandRunner, CommandSpec, SystemCommandRunner};
use crate::document::{SourceDocuments, is_lower_hex_sha, load_documents};
use crate::freshness::{
    FreshnessEnvelope, ObservationState, PendingMutations, RemoteRelation, classify,
    full_nonblank_commit, validate,
};
use crate::pin::{InstalledBeads, PinManifest, VerifiedBeads};
use crate::shadow::{
    OperationalDocument, ShadowStore, canonical_logical_export, canonical_operational_projection,
    compare_shadow_parity, decode_operational_beads_jsonl, import_operational_shadow_documents,
    initial_operational_metadata, logical_export_digest, operational_projection_digest,
};
use sha2::{Digest, Sha256};

const STORE_DIRECTORY: &str = "plasmosome-work-state";
const CURRENT_POINTER: &str = "current";
const GENERATIONS_DIRECTORY: &str = "generations";

/// A stable refusal raised while locating or loading a clone-local shadow generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreError {
    code: &'static str,
}

impl StoreError {
    /// Returns the stable machine-readable refusal code.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for StoreError {}

fn refusal(code: &'static str) -> StoreError {
    StoreError { code }
}

/// The immutable locations shared by every linked worktree in one Git clone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreLocation {
    /// The canonical invoking worktree root.
    pub worktree_root: PathBuf,
    /// The canonical Git common directory for this clone.
    pub common_dir: PathBuf,
    /// The clone-local state root below the Git common directory.
    pub state_root: PathBuf,
    /// The directory containing immutable generation directories.
    pub generations_dir: PathBuf,
}

/// The strict persisted observation state for one immutable generation.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateManifest {
    /// The manifest format version.
    pub schema_version: u64,
    /// The Beads authority mode for this shadow.
    pub authority_mode: String,
    /// The selected Markdown source commit.
    pub source_commit: String,
    /// The canonical logical shadow export digest.
    pub logical_export_sha256: String,
    /// The canonical document-plus-operational projection digest.
    pub operational_projection_sha256: String,
    /// The committed embedded-Dolt generation used by every read.
    pub local_generation: String,
    /// The exact pinned host target installed in this generation.
    pub host_target: String,
    /// The SHA-256 of the installed wrapper executable.
    pub wrapper_sha256: String,
    /// The SHA-256 of the installed Beads executable.
    pub beads_binary_sha256: String,
    /// The last known relation between local and remote generations.
    pub remote_relation: RemoteRelation,
    /// The last observed remote `refs/dolt/data` generation, if known.
    pub remote_generation: Option<String>,
    /// The UTC time at which the remote generation was observed, if known.
    pub remote_observed_at: Option<String>,
    /// The local generation that was compared to the remote one, if known.
    pub observed_local_generation: Option<String>,
    /// The UTC time of a known successful synchronization, if any.
    pub last_successful_sync_at: Option<String>,
    /// Semantic operation ids awaiting remote publication in recorded order.
    pub pending_operation_ids: Vec<String>,
}

/// One validated immutable generation selected by the `current` pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentGeneration {
    /// The safe basename stored in `current`.
    pub name: String,
    /// The immutable generation directory.
    pub root: PathBuf,
    /// The strict state manifest bound to the selected generation.
    pub manifest: StateManifest,
}

/// The caller-supplied bootstrap inputs; artifact paths are never persisted in the store.
#[derive(Clone, Debug)]
pub struct BootstrapRequest {
    /// The worktree from which the clone-local common directory is located.
    pub checkout: PathBuf,
    /// The source repository that owns the pin manifest and requested Git ref.
    pub source_root: PathBuf,
    /// The locally available Markdown source ref to resolve once.
    pub source_ref: String,
    /// The caller-supplied verified-release archive candidate.
    pub archive: PathBuf,
    /// The caller-supplied extracted pinned Beads executable candidate.
    pub binary: PathBuf,
    /// The currently running wrapper executable to install into the generation.
    pub wrapper: PathBuf,
    /// The compiled host target selected from the pinned manifest.
    pub host_target: String,
}

/// The outcome category of an explicit clone-local bootstrap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapOutcome {
    /// A first complete generation was atomically installed.
    Installed,
    /// A broken installed runtime was replaced without reimporting Markdown.
    Reinstalled,
    /// The existing validated generation already matched the requested source.
    Unchanged,
}

/// Stable evidence returned after an explicit bootstrap completes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BootstrapResult {
    /// Whether bootstrap installed, reinstalled, or reused the active generation.
    pub outcome: BootstrapOutcome,
    /// The immutable resolved Markdown source commit.
    pub source_commit: String,
    /// The committed embedded-Dolt generation read from `bd vc status`.
    pub local_generation: String,
    /// Counts grouped by Markdown namespace.
    pub document_counts: BootstrapDocumentCounts,
    /// The canonical logical export digest.
    pub logical_export_sha256: String,
}

/// The logical corpus counts reported by bootstrap without copying its Markdown contents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BootstrapDocumentCounts {
    /// Number of intent documents.
    pub intent: usize,
    /// Number of spec documents.
    pub spec: usize,
    /// Number of task documents.
    pub task: usize,
}

/// Renders every bootstrap success field without retaining caller artifact paths.
pub fn render_bootstrap_human(result: &BootstrapResult) -> String {
    let outcome = match result.outcome {
        BootstrapOutcome::Installed => "installed",
        BootstrapOutcome::Reinstalled => "reinstalled",
        BootstrapOutcome::Unchanged => "unchanged",
    };
    format!(
        "bootstrap: {outcome}\nsource commit: {}\nlocal generation: {}\ndocument counts: intent={} spec={} task={}\nlogical export sha256: {}\n",
        result.source_commit,
        result.local_generation,
        result.document_counts.intent,
        result.document_counts.spec,
        result.document_counts.task,
        result.logical_export_sha256,
    )
}

/// One complete read of a committed disposable Beads snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FencedSnapshot {
    /// The strict typed documents and task operational siblings from the export.
    pub documents: Vec<OperationalDocument>,
    /// The complete validated freshness envelope recorded in the manifest.
    pub freshness: FreshnessEnvelope,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionControlStatus {
    schema_version: u64,
    branch: String,
    commit: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyValueList {
    schema_version: u64,
    #[serde(rename = "plasmosome.authority-mode")]
    authority_mode: String,
    #[serde(rename = "plasmosome.source-commit")]
    source_commit: String,
}

/// A deterministic interruption point used by the atomic-activation contract tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationFault {
    /// Refuse before the staged generation becomes visible under its final name.
    BeforeGenerationRename,
    /// Refuse after the complete generation is named but before a new pointer is written.
    BeforePointerWrite,
    /// Refuse after the temporary pointer is durable but before it replaces `current`.
    BeforePointerRename,
}

/// A process-scoped nonblocking lock that serializes bootstrap preparation only.
#[derive(Debug)]
pub struct BootstrapLock {
    file: File,
}

impl BootstrapLock {
    /// Acquires the clone-local bootstrap lock without waiting for another installer.
    pub fn acquire(location: &StoreLocation) -> Result<Self, StoreError> {
        match fs::symlink_metadata(&location.state_root) {
            Ok(_) => directory_without_symlink(&location.state_root)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&location.state_root).map_err(|_| refusal("invalid_store"))?;
                directory_without_symlink(&location.state_root)?;
            }
            Err(_) => return Err(refusal("invalid_store")),
        }
        let path = location.state_root.join("bootstrap.lock");
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && metadata.file_type().is_symlink()
        {
            return Err(refusal("invalid_store"));
        }
        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::OpenOptionsExt;

            OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .map_err(|_| refusal("invalid_store"))?
        };
        #[cfg(not(unix))]
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|_| refusal("invalid_store"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                let code = std::io::Error::last_os_error().raw_os_error();
                return Err(
                    if code == Some(libc::EAGAIN) || code == Some(libc::EWOULDBLOCK) {
                        refusal("bootstrap_busy")
                    } else {
                        refusal("invalid_store")
                    },
                );
            }
        }
        Ok(Self { file })
    }
}

impl Drop for BootstrapLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

fn directory_without_symlink(path: &Path) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| refusal("invalid_store"))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(refusal("invalid_store"));
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), StoreError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| refusal("invalid_store"))
}

/// Activates one complete staged generation by replacing only the `current` pointer atomically.
pub fn activate_staged_generation(
    location: &StoreLocation,
    staging: &Path,
    generation_name: &str,
    fault: Option<ActivationFault>,
) -> Result<(), StoreError> {
    if !safe_generation_name(generation_name)
        || staging.parent() != Some(location.generations_dir.as_path())
        || !staging
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".staging-"))
    {
        return Err(refusal("invalid_store"));
    }
    directory_without_symlink(&location.state_root)?;
    directory_without_symlink(&location.generations_dir)?;
    directory_without_symlink(staging)?;
    if matches!(fault, Some(ActivationFault::BeforeGenerationRename)) {
        return Err(refusal("bootstrap_interrupted"));
    }
    sync_directory(staging)?;
    let final_generation = location.generations_dir.join(generation_name);
    if final_generation.exists() || fs::symlink_metadata(&final_generation).is_ok() {
        return Err(refusal("invalid_store"));
    }
    fs::rename(staging, &final_generation).map_err(|_| refusal("invalid_store"))?;
    sync_directory(&location.generations_dir)?;
    if matches!(fault, Some(ActivationFault::BeforePointerWrite)) {
        return Err(refusal("bootstrap_interrupted"));
    }
    let pointer = location.state_root.join(CURRENT_POINTER);
    if let Ok(metadata) = fs::symlink_metadata(&pointer)
        && metadata.file_type().is_symlink()
    {
        return Err(refusal("invalid_store"));
    }
    let mut pointer_file = tempfile::Builder::new()
        .prefix(".current-")
        .suffix(".tmp")
        .tempfile_in(&location.state_root)
        .map_err(|_| refusal("invalid_store"))?;
    pointer_file
        .write_all(format!("{generation_name}\n").as_bytes())
        .and_then(|()| pointer_file.as_file().sync_all())
        .map_err(|_| refusal("invalid_store"))?;
    let (_pointer_file, temporary_pointer) =
        pointer_file.keep().map_err(|_| refusal("invalid_store"))?;
    if matches!(fault, Some(ActivationFault::BeforePointerRename)) {
        return Err(refusal("bootstrap_interrupted"));
    }
    fs::rename(&temporary_pointer, pointer).map_err(|_| refusal("invalid_store"))?;
    sync_directory(&location.state_root)?;
    Ok(())
}

fn one_absolute_path(value: &str) -> Option<PathBuf> {
    let value = value.strip_suffix('\n')?;
    if value.is_empty() || value.contains(['\n', '\r']) {
        return None;
    }
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

fn canonical_existing_directory(path: &Path) -> Result<PathBuf, StoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| refusal("invalid_store_location"))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(refusal("invalid_store_location"));
    }
    let canonical = fs::canonicalize(path).map_err(|_| refusal("invalid_store_location"))?;
    Ok(canonical)
}

fn canonical_reported_directory(path: &Path) -> Result<PathBuf, StoreError> {
    let canonical = canonical_existing_directory(path)?;
    if canonical != path {
        return Err(refusal("invalid_store_location"));
    }
    Ok(canonical)
}

fn locator_command(
    checkout: &Path,
    environment: &BTreeMap<String, String>,
    argv: Vec<String>,
) -> CommandSpec {
    let mut environment = environment.clone();
    environment.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    environment.insert("GIT_NO_LAZY_FETCH".into(), "1".into());
    environment.insert("GIT_OPTIONAL_LOCKS".into(), "0".into());
    environment.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    CommandSpec {
        program: PathBuf::from("git"),
        argv,
        cwd: Some(checkout.to_path_buf()),
        environment,
        redacted_argv_positions: Vec::new(),
    }
}

fn run_locator<R: CommandRunner>(
    runner: &mut R,
    checkout: &Path,
    environment: &BTreeMap<String, String>,
    argv: Vec<String>,
) -> Result<PathBuf, StoreError> {
    let command = locator_command(checkout, environment, argv);
    validate_read_locator_command(&command, checkout)
        .map_err(|_| refusal("invalid_store_location"))?;
    let output = runner
        .run(command)
        .map_err(|_| refusal("invalid_store_location"))?;
    if output.status != 0 {
        return Err(refusal("invalid_store_location"));
    }
    one_absolute_path(&output.stdout).ok_or_else(|| refusal("invalid_store_location"))
}

/// Resolves the one state root shared by linked worktrees without creating it.
pub fn locate_store<R: CommandRunner>(
    runner: &mut R,
    checkout: &Path,
    environment: BTreeMap<String, String>,
) -> Result<StoreLocation, StoreError> {
    let supplied_checkout = canonical_existing_directory(checkout)?;
    let top_level = run_locator(
        runner,
        &supplied_checkout,
        &environment,
        vec!["rev-parse".into(), "--show-toplevel".into()],
    )?;
    let worktree_root = canonical_reported_directory(&top_level)?;
    if worktree_root != supplied_checkout {
        return Err(refusal("invalid_store_location"));
    }
    let common = run_locator(
        runner,
        &worktree_root,
        &environment,
        vec![
            "rev-parse".into(),
            "--path-format=absolute".into(),
            "--git-common-dir".into(),
        ],
    )?;
    let common_dir = canonical_reported_directory(&common)?;
    let state_root = common_dir.join(STORE_DIRECTORY);
    Ok(StoreLocation {
        worktree_root,
        common_dir,
        generations_dir: state_root.join(GENERATIONS_DIRECTORY),
        state_root,
    })
}

fn regular_file(path: &Path) -> Result<File, StoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| refusal("invalid_store"))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(refusal("invalid_store"));
    }
    File::open(path).map_err(|_| refusal("invalid_store"))
}

fn safe_generation_name(value: &str) -> bool {
    value.starts_with("generation-")
        && value.len() > "generation-".len()
        && value
            .bytes()
            .skip("generation-".len())
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn strict_manifest(value: &str) -> Result<StateManifest, StoreError> {
    let manifest: StateManifest =
        serde_json::from_str(value).map_err(|_| refusal("invalid_store"))?;
    let hashes = [
        &manifest.logical_export_sha256,
        &manifest.operational_projection_sha256,
        &manifest.wrapper_sha256,
        &manifest.beads_binary_sha256,
    ];
    if manifest.schema_version != 1
        || manifest.authority_mode != "markdown-shadow"
        || !is_lower_hex_sha(&manifest.source_commit)
        || hashes.iter().any(|value| {
            value.len() != 64
                || !value
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
        || !full_nonblank_commit(&manifest.local_generation)
        || manifest.host_target.trim().is_empty()
        || !valid_pending_ids(&manifest.pending_operation_ids)
        || !valid_observation(&manifest)
    {
        return Err(refusal("invalid_store"));
    }
    Ok(manifest)
}

fn valid_pending_ids(ids: &[String]) -> bool {
    let mut unique = BTreeSet::new();
    ids.iter()
        .all(|id| !id.trim().is_empty() && unique.insert(id.as_str()))
}

fn pin_refusal(error: crate::pin::PinError) -> StoreError {
    refusal(error.code())
}

fn required_runtime_paths(root: &Path) -> [PathBuf; 5] {
    [
        root.join("home"),
        root.join("xdg_config"),
        root.join("xdg_cache"),
        root.join("xdg_data"),
        root.join("tmp"),
    ]
}

fn environment_for_runtime(
    root: &Path,
    create: bool,
) -> Result<BTreeMap<String, String>, StoreError> {
    if create {
        match fs::symlink_metadata(root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(root).map_err(|_| refusal("invalid_store"))?;
            }
            Ok(_) => {}
            Err(_) => return Err(refusal("invalid_store")),
        }
    }
    directory_without_symlink(root)?;
    if create {
        for path in required_runtime_paths(root) {
            fs::create_dir_all(&path).map_err(|_| refusal("invalid_store"))?;
        }
    }
    for path in required_runtime_paths(root) {
        directory_without_symlink(&path)?;
    }
    let git_config = root.join("git_config_global");
    if create {
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&git_config)
            .and_then(|file| file.sync_all())
            .map_err(|_| refusal("invalid_store"))?;
    }
    regular_file(&git_config)?;
    let mut environment = BTreeMap::new();
    let paths = required_runtime_paths(root);
    for (key, path) in [
        ("HOME", &paths[0]),
        ("XDG_CONFIG_HOME", &paths[1]),
        ("XDG_CACHE_HOME", &paths[2]),
        ("XDG_DATA_HOME", &paths[3]),
        ("TMPDIR", &paths[4]),
    ] {
        environment.insert(key.to_owned(), path.display().to_string());
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
    let path = std::env::var_os("PATH").ok_or_else(|| refusal("invalid_store"))?;
    environment.insert("PATH".into(), path.to_string_lossy().into_owned());
    Ok(environment)
}

fn copy_regular_file(source: &Path, destination: &Path) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(source).map_err(|_| refusal("invalid_store"))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(refusal("invalid_store"));
    }
    fs::copy(source, destination).map_err(|_| refusal("invalid_store"))?;
    fs::set_permissions(destination, metadata.permissions())
        .map_err(|_| refusal("invalid_store"))?;
    Ok(())
}

fn copy_private_tree(source: &Path, destination: &Path) -> Result<(), StoreError> {
    directory_without_symlink(source)?;
    fs::create_dir(destination).map_err(|_| refusal("invalid_store"))?;
    for entry in fs::read_dir(source).map_err(|_| refusal("invalid_store"))? {
        let entry = entry.map_err(|_| refusal("invalid_store"))?;
        let source_entry = entry.path();
        let destination_entry = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_entry).map_err(|_| refusal("invalid_store"))?;
        if metadata.file_type().is_symlink() {
            return Err(refusal("invalid_store"));
        }
        if metadata.file_type().is_dir() {
            copy_private_tree(&source_entry, &destination_entry)?;
        } else if metadata.file_type().is_file() {
            copy_regular_file(&source_entry, &destination_entry)?;
        } else {
            return Err(refusal("invalid_store"));
        }
    }
    Ok(())
}

fn sha256(path: &Path) -> Result<String, StoreError> {
    let file = regular_file(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    std::io::copy(&mut reader, &mut hasher).map_err(|_| refusal("invalid_store"))?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn owner_private_executable(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| refusal("invalid_store"))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn write_new_sync(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| refusal("invalid_store"))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| refusal("invalid_store"))
}

fn bootstrap_command(
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

fn run_bootstrap_command<R: CommandRunner>(
    runner: &mut R,
    command: CommandSpec,
) -> Result<String, StoreError> {
    let output = runner.run(command).map_err(|_| refusal("invalid_store"))?;
    if output.status != 0 {
        return Err(refusal("invalid_store"));
    }
    Ok(output.stdout)
}

fn bootstrap_counts(documents: &SourceDocuments) -> BootstrapDocumentCounts {
    let mut counts = BootstrapDocumentCounts {
        intent: 0,
        spec: 0,
        task: 0,
    };
    for document in &documents.documents {
        match document.record.kind {
            crate::document::DocumentKind::Intent => counts.intent += 1,
            crate::document::DocumentKind::Spec => counts.spec += 1,
            crate::document::DocumentKind::Task => counts.task += 1,
        }
    }
    counts
}

fn source_refusal(error: crate::document::DocumentError) -> StoreError {
    refusal(error.code())
}

fn valid_observation(manifest: &StateManifest) -> bool {
    validate(&ObservationState {
        last_successful_sync_at: manifest.last_successful_sync_at.clone(),
        local_generation: manifest.local_generation.clone(),
        remote_generation: manifest.remote_generation.clone(),
        remote_observed_at: manifest.remote_observed_at.clone(),
        observed_local_generation: manifest.observed_local_generation.clone(),
        remote_relation: manifest.remote_relation.clone(),
        pending_mutations: PendingMutations {
            operation_ids: manifest.pending_operation_ids.clone(),
        },
    })
    .is_ok()
}

fn version_control_commit(value: &str) -> Result<String, StoreError> {
    let status: VersionControlStatus =
        serde_json::from_str(value).map_err(|_| refusal("invalid_store"))?;
    if status.schema_version != 1
        || status.branch != "main"
        || !full_nonblank_commit(&status.commit)
    {
        return Err(refusal("invalid_store"));
    }
    Ok(status.commit)
}

fn key_value_list(value: &str) -> Result<KeyValueList, StoreError> {
    let values: KeyValueList = serde_json::from_str(value).map_err(|_| refusal("invalid_store"))?;
    if values.schema_version != 1 {
        return Err(refusal("invalid_store"));
    }
    Ok(values)
}

/// Validates the exact two-status fenced snapshot protocol without opening a Beads repository.
pub fn validate_fenced_snapshot(
    manifest: &StateManifest,
    before_status: &str,
    export: &str,
    key_values: &str,
    after_status: &str,
) -> Result<FencedSnapshot, StoreError> {
    let manifest =
        strict_manifest(&serde_json::to_string(manifest).map_err(|_| refusal("invalid_store"))?)?;
    let before = version_control_commit(before_status)?;
    let after = version_control_commit(after_status)?;
    if before != after || before != manifest.local_generation {
        return Err(refusal("store_changed"));
    }
    let key_values = key_value_list(key_values)?;
    if key_values.authority_mode != "markdown-shadow"
        || key_values.source_commit != manifest.source_commit
    {
        return Err(refusal("store_changed"));
    }
    let mut documents =
        decode_operational_beads_jsonl(export).map_err(|_| refusal("invalid_store"))?;
    documents.sort_by(|left, right| {
        left.document
            .record
            .document_key
            .cmp(&right.document.record.document_key)
    });
    let logical_documents = documents
        .iter()
        .map(|document| document.document.clone())
        .collect::<Vec<_>>();
    let logical =
        canonical_logical_export(&logical_documents).map_err(|_| refusal("invalid_store"))?;
    let operational =
        canonical_operational_projection(&documents).map_err(|_| refusal("invalid_store"))?;
    if logical_export_digest(&logical) != manifest.logical_export_sha256
        || operational_projection_digest(&operational) != manifest.operational_projection_sha256
    {
        return Err(refusal("store_changed"));
    }
    let freshness = classify(ObservationState {
        last_successful_sync_at: manifest.last_successful_sync_at,
        local_generation: manifest.local_generation,
        remote_generation: manifest.remote_generation,
        remote_observed_at: manifest.remote_observed_at,
        observed_local_generation: manifest.observed_local_generation,
        remote_relation: manifest.remote_relation,
        pending_mutations: PendingMutations {
            operation_ids: manifest.pending_operation_ids,
        },
    })
    .map_err(|_| refusal("invalid_store"))?;
    Ok(FencedSnapshot {
        documents,
        freshness,
    })
}

/// Refuses every command outside the four read-only disposable Beads forms.
pub fn validate_read_command(
    command: &CommandSpec,
    temporary_repository: &Path,
    copied_binary: &Path,
) -> Result<(), StoreError> {
    let temporary_root = temporary_repository
        .parent()
        .ok_or_else(|| refusal("invalid_read_command"))?;
    if copied_binary != temporary_root.join("bd") {
        return Err(refusal("invalid_read_command"));
    }
    let expected_environment = environment_for_runtime(&temporary_root.join("runtime"), false)
        .map_err(|_| refusal("invalid_read_command"))?;
    if command.environment != expected_environment {
        return Err(refusal("invalid_read_command"));
    }
    if command.program != copied_binary {
        return Err(refusal("invalid_read_command"));
    }
    if command.argv == ["--version"] {
        return if command.cwd.is_none() {
            Ok(())
        } else {
            Err(refusal("invalid_read_command"))
        };
    }
    if command.cwd.as_deref() != Some(temporary_repository) {
        return Err(refusal("invalid_read_command"));
    }
    let readonly = ["--readonly", "--sandbox"];
    let allowed = [
        vec![
            "--readonly".to_owned(),
            "--sandbox".to_owned(),
            "--json".to_owned(),
            "vc".to_owned(),
            "status".to_owned(),
        ],
        vec![
            "--readonly".to_owned(),
            "--sandbox".to_owned(),
            "export".to_owned(),
        ],
        vec![
            "--readonly".to_owned(),
            "--sandbox".to_owned(),
            "--json".to_owned(),
            "kv".to_owned(),
            "list".to_owned(),
        ],
    ];
    if command.argv.get(..2).map(|prefix| prefix == readonly) != Some(true)
        || !allowed.contains(&command.argv)
    {
        return Err(refusal("invalid_read_command"));
    }
    Ok(())
}

/// Refuses every outer Git plan except the two local worktree-location queries.
pub fn validate_read_locator_command(
    command: &CommandSpec,
    checkout: &Path,
) -> Result<(), StoreError> {
    if command.program != Path::new("git") || command.cwd.as_deref() != Some(checkout) {
        return Err(refusal("invalid_read_command"));
    }
    let allowed = [
        vec!["rev-parse".to_owned(), "--show-toplevel".to_owned()],
        vec![
            "rev-parse".to_owned(),
            "--path-format=absolute".to_owned(),
            "--git-common-dir".to_owned(),
        ],
    ];
    allowed
        .contains(&command.argv)
        .then_some(())
        .ok_or_else(|| refusal("invalid_read_command"))
}

fn bootstrap_source_command(argv: &[String]) -> bool {
    match argv {
        [command, verify, end_of_options, reference]
            if command == "rev-parse"
                && verify == "--verify"
                && end_of_options == "--end-of-options" =>
        {
            reference
                .strip_suffix("^{commit}")
                .is_some_and(|reference| {
                    !reference.trim().is_empty() && !reference.contains(['\n', '\r'])
                })
        }
        [
            command,
            recursive,
            names_only,
            nul,
            commit,
            separator,
            intents,
            specs,
            tasks,
        ] if command == "ls-tree"
            && recursive == "-r"
            && names_only == "--name-only"
            && nul == "-z"
            && is_lower_hex_sha(commit)
            && separator == "--"
            && intents == "docs/intents"
            && specs == "docs/specs"
            && tasks == "tasks" =>
        {
            true
        }
        [command, object] if command == "show" => {
            object.split_once(':').is_some_and(|(commit, path)| {
                is_lower_hex_sha(commit) && !path.is_empty() && !path.contains(['\n', '\r'])
            })
        }
        [command, one, format, commit, separator, literal_path]
            if command == "log"
                && one == "-1"
                && format == "--format=%H"
                && is_lower_hex_sha(commit)
                && separator == "--" =>
        {
            literal_path
                .strip_prefix(":(literal)")
                .is_some_and(|path| !path.is_empty() && !path.contains(['\n', '\r']))
        }
        _ => false,
    }
}

fn bootstrap_repository_git_command(argv: &[String]) -> bool {
    match argv {
        [init, quiet] => init == "init" && quiet == "--quiet",
        [config, key, value] if config == "config" => matches!(
            (key.as_str(), value.as_str()),
            ("user.email", "plasmosome@local.invalid")
                | ("user.name", "Plasmosome local shadow")
                | ("dolt.auto-push", "false")
        ),
        _ => false,
    }
}

fn bootstrap_beads_command(argv: &[String]) -> bool {
    match argv {
        [
            sandbox,
            init,
            stealth,
            skip_agents,
            skip_hooks,
            non_interactive,
        ] if sandbox == "--sandbox"
            && init == "init"
            && stealth == "--stealth"
            && skip_agents == "--skip-agents"
            && skip_hooks == "--skip-hooks"
            && non_interactive == "--non-interactive" =>
        {
            true
        }
        [sandbox, import, path, json]
            if sandbox == "--sandbox"
                && import == "import"
                && json == "--json"
                && Path::new(path).is_absolute()
                && !path.contains(['\n', '\r']) =>
        {
            true
        }
        [sandbox, export] if sandbox == "--sandbox" && export == "export" => true,
        [sandbox, kv, set, key, value] if sandbox == "--sandbox" && kv == "kv" && set == "set" => {
            (key == "plasmosome.authority-mode" && value == "markdown-shadow")
                || (key == "plasmosome.source-commit" && is_lower_hex_sha(value))
        }
        [sandbox, kv, get, key]
            if sandbox == "--sandbox"
                && kv == "kv"
                && get == "get"
                && matches!(
                    key.as_str(),
                    "plasmosome.authority-mode" | "plasmosome.source-commit"
                ) =>
        {
            true
        }
        [sandbox, dolt, commit, message, value]
            if sandbox == "--sandbox"
                && dolt == "dolt"
                && commit == "commit"
                && message == "-m" =>
        {
            value
                .strip_prefix("bootstrap markdown-shadow ")
                .is_some_and(is_lower_hex_sha)
        }
        [readonly, sandbox, json, version_control, status]
            if readonly == "--readonly"
                && sandbox == "--sandbox"
                && json == "--json"
                && version_control == "vc"
                && status == "status" =>
        {
            true
        }
        _ => false,
    }
}

/// Refuses every process plan outside the exact local source and bootstrap forms.
pub fn validate_bootstrap_command(
    command: &CommandSpec,
    source_root: &Path,
    repository: &Path,
    binary: &Path,
) -> Result<(), StoreError> {
    let valid = if command.program == Path::new("git") {
        match command.cwd.as_deref() {
            Some(cwd) if cwd == source_root => bootstrap_source_command(&command.argv),
            Some(cwd) if cwd == repository => bootstrap_repository_git_command(&command.argv),
            _ => false,
        }
    } else if command.program == binary && command.cwd.as_deref() == Some(repository) {
        bootstrap_beads_command(&command.argv)
    } else {
        false
    };
    valid
        .then_some(())
        .ok_or_else(|| refusal("invalid_bootstrap_command"))
}

fn bootstrap_locator_command(command: &CommandSpec) -> bool {
    if command.program != Path::new("git") || command.cwd.is_none() {
        return false;
    }
    match command.argv.as_slice() {
        [command, top_level] => command == "rev-parse" && top_level == "--show-toplevel",
        [command, path_format, common_dir] => {
            command == "rev-parse"
                && path_format == "--path-format=absolute"
                && common_dir == "--git-common-dir"
        }
        _ => false,
    }
}

struct BootstrapCommandRunner {
    source_root: PathBuf,
    initial_binary: PathBuf,
    inner: SystemCommandRunner,
}

impl BootstrapCommandRunner {
    fn new(source_root: PathBuf, initial_binary: PathBuf) -> Self {
        Self {
            source_root,
            initial_binary,
            inner: SystemCommandRunner,
        }
    }
}

impl CommandRunner for BootstrapCommandRunner {
    fn run(&mut self, command: CommandSpec) -> Result<crate::command::CommandOutput, String> {
        let valid = if command.argv == ["--version"] && command.cwd.is_none() {
            command.program == self.initial_binary
                || command.program.file_name().and_then(|name| name.to_str()) == Some("bd")
        } else if bootstrap_locator_command(&command) {
            true
        } else if let Some(repository) = command.cwd.as_deref() {
            validate_bootstrap_command(&command, &self.source_root, repository, &command.program)
                .is_ok()
                || validate_read_command(&command, repository, &command.program).is_ok()
        } else {
            false
        };
        if !valid {
            return Err("invalid_bootstrap_command".into());
        }
        self.inner.run(command)
    }
}

fn run_read_command<R: CommandRunner>(
    runner: &mut R,
    command: CommandSpec,
    temporary_repository: &Path,
    copied_binary: &Path,
) -> Result<String, StoreError> {
    validate_read_command(&command, temporary_repository, copied_binary)?;
    let output = runner.run(command).map_err(|_| refusal("invalid_store"))?;
    if output.status != 0 {
        return Err(refusal("invalid_store"));
    }
    Ok(output.stdout)
}

fn read_disposable_snapshot_with_binary<R: CommandRunner>(
    runner: &mut R,
    generation: &CurrentGeneration,
    pin: &PinManifest,
    target: &str,
    selected_binary: &Path,
) -> Result<FencedSnapshot, StoreError> {
    let expected_binary_sha = pin
        .targets
        .iter()
        .find(|candidate| candidate.target == target)
        .ok_or_else(|| refusal("unsupported_beads_platform"))?
        .binary_sha256
        .as_str();
    if generation.manifest.host_target != target
        || generation.manifest.beads_binary_sha256 != expected_binary_sha
    {
        return Err(refusal("invalid_store"));
    }
    let runtime = generation.root.join("runtime");
    let shared_environment = environment_for_runtime(&runtime, false)?;
    InstalledBeads::verify(pin, target, selected_binary, shared_environment, runner)
        .map_err(pin_refusal)?;
    let temporary_root = tempfile::Builder::new()
        .prefix("plasmosome-read-")
        .tempdir()
        .map_err(|_| refusal("invalid_store"))?;
    let result = (|| {
        let repository = temporary_root.path().join("repository");
        copy_private_tree(&generation.root.join("repository"), &repository)?;
        let copied_binary = temporary_root.path().join("bd");
        copy_regular_file(selected_binary, &copied_binary)?;
        let copied_runtime = temporary_root.path().join("runtime");
        let environment = environment_for_runtime(&copied_runtime, true)?;
        InstalledBeads::verify(pin, target, &copied_binary, environment.clone(), runner)
            .map_err(pin_refusal)?;
        let status_command = || CommandSpec {
            program: copied_binary.clone(),
            argv: vec![
                "--readonly".into(),
                "--sandbox".into(),
                "--json".into(),
                "vc".into(),
                "status".into(),
            ],
            cwd: Some(repository.clone()),
            environment: environment.clone(),
            redacted_argv_positions: Vec::new(),
        };
        let before = run_read_command(runner, status_command(), &repository, &copied_binary)?;
        let export = run_read_command(
            runner,
            CommandSpec {
                program: copied_binary.clone(),
                argv: vec!["--readonly".into(), "--sandbox".into(), "export".into()],
                cwd: Some(repository.clone()),
                environment: environment.clone(),
                redacted_argv_positions: Vec::new(),
            },
            &repository,
            &copied_binary,
        )?;
        let key_values = run_read_command(
            runner,
            CommandSpec {
                program: copied_binary.clone(),
                argv: vec![
                    "--readonly".into(),
                    "--sandbox".into(),
                    "--json".into(),
                    "kv".into(),
                    "list".into(),
                ],
                cwd: Some(repository.clone()),
                environment: environment.clone(),
                redacted_argv_positions: Vec::new(),
            },
            &repository,
            &copied_binary,
        )?;
        let after = run_read_command(runner, status_command(), &repository, &copied_binary)?;
        validate_fenced_snapshot(&generation.manifest, &before, &export, &key_values, &after)
    })();
    match (result, temporary_root.close()) {
        (Ok(snapshot), Ok(())) => Ok(snapshot),
        (Err(error), _) => Err(error),
        (Ok(_), Err(_)) => Err(refusal("temporary_cleanup_failed")),
    }
}

/// Reads one active generation through a private disposable copy and validates both fences.
pub fn read_disposable_snapshot<R: CommandRunner>(
    runner: &mut R,
    generation: &CurrentGeneration,
    pin: &PinManifest,
    target: &str,
) -> Result<FencedSnapshot, StoreError> {
    read_disposable_snapshot_with_binary(
        runner,
        generation,
        pin,
        target,
        &generation.root.join("bd"),
    )
}

/// Loads the one complete immutable generation named by `current` without modifying it.
pub fn current_generation(location: &StoreLocation) -> Result<CurrentGeneration, StoreError> {
    directory_without_symlink(&location.state_root)?;
    directory_without_symlink(&location.generations_dir)?;
    let pointer_path = location.state_root.join(CURRENT_POINTER);
    let mut pointer = String::new();
    regular_file(&pointer_path)?
        .read_to_string(&mut pointer)
        .map_err(|_| refusal("invalid_store"))?;
    if !pointer.ends_with('\n') || pointer.matches('\n').count() != 1 {
        return Err(refusal("invalid_store"));
    }
    let name = pointer.trim_end_matches('\n');
    if !safe_generation_name(name) {
        return Err(refusal("invalid_store"));
    }
    let root = location.generations_dir.join(name);
    if root
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(refusal("invalid_store"));
    }
    let root_metadata = fs::symlink_metadata(&root).map_err(|_| refusal("invalid_store"))?;
    if !root_metadata.file_type().is_dir() || root_metadata.file_type().is_symlink() {
        return Err(refusal("invalid_store"));
    }
    let mut state = String::new();
    regular_file(&root.join("state.json"))?
        .read_to_string(&mut state)
        .map_err(|_| refusal("invalid_store"))?;
    let manifest = strict_manifest(&state)?;
    Ok(CurrentGeneration {
        name: name.to_owned(),
        root,
        manifest,
    })
}

/// Builds the minimal cleared environment used for the two local Git locator calls.
pub fn locator_environment() -> Result<BTreeMap<String, String>, StoreError> {
    let path = std::env::var_os("PATH").ok_or_else(|| refusal("invalid_store_location"))?;
    Ok(BTreeMap::from([(
        "PATH".to_owned(),
        path.to_string_lossy().into_owned(),
    )]))
}

/// Loads the release pin compiled into the installed wrapper, not a mutable source checkout file.
pub fn compiled_pin_manifest() -> Result<PinManifest, StoreError> {
    PinManifest::parse(include_str!("../../../tools/work-state-beads-1.1.2.toml"))
        .map_err(pin_refusal)
}

/// Returns the build host target supported by the compiled wrapper.
pub fn host_target() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else {
        "unsupported"
    }
}

fn safe_state_root_exists(location: &StoreLocation) -> Result<bool, StoreError> {
    match fs::symlink_metadata(&location.state_root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Ok(_) => {
            directory_without_symlink(&location.state_root)?;
            Ok(true)
        }
        Err(_) => Err(refusal("invalid_store")),
    }
}

/// Loads the active generation while mapping an absent `current` pointer to `not_initialized`.
pub fn active_generation(location: &StoreLocation) -> Result<CurrentGeneration, StoreError> {
    if !safe_state_root_exists(location)? {
        return Err(refusal("not_initialized"));
    }
    match fs::symlink_metadata(location.state_root.join(CURRENT_POINTER)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(refusal("not_initialized"))
        }
        Ok(_) => current_generation(location),
        Err(_) => Err(refusal("invalid_store")),
    }
}

/// Verifies that this process is the checksum-bound executable selected from the active generation.
pub fn validate_installed_wrapper(
    generation: &CurrentGeneration,
    executable: &Path,
) -> Result<(), StoreError> {
    let expected = generation.root.join("plasmosome-work-state");
    let executable_metadata =
        fs::symlink_metadata(executable).map_err(|_| refusal("invalid_store"))?;
    if !executable_metadata.file_type().is_file() || executable_metadata.file_type().is_symlink() {
        return Err(refusal("invalid_store"));
    }
    let expected_metadata =
        fs::symlink_metadata(&expected).map_err(|_| refusal("invalid_store"))?;
    if !expected_metadata.file_type().is_file() || expected_metadata.file_type().is_symlink() {
        return Err(refusal("invalid_store"));
    }
    if fs::canonicalize(executable).map_err(|_| refusal("invalid_store"))?
        != fs::canonicalize(&expected).map_err(|_| refusal("invalid_store"))?
    {
        return Err(refusal("invalid_store"));
    }
    wrapper_is_valid(generation)
}

fn optional_current_generation(
    location: &StoreLocation,
) -> Result<Option<CurrentGeneration>, StoreError> {
    if !safe_state_root_exists(location)? {
        return Ok(None);
    }
    match fs::symlink_metadata(location.state_root.join(CURRENT_POINTER)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(_) => current_generation(location).map(Some),
        Err(_) => Err(refusal("invalid_store")),
    }
}

fn create_generation_staging(location: &StoreLocation) -> Result<(PathBuf, String), StoreError> {
    match fs::symlink_metadata(&location.generations_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&location.generations_dir).map_err(|_| refusal("invalid_store"))?;
        }
        Ok(_) => {}
        Err(_) => return Err(refusal("invalid_store")),
    }
    directory_without_symlink(&location.generations_dir)?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&location.generations_dir)
        .map_err(|_| refusal("invalid_store"))?
        .keep();
    let suffix = staging
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(".staging-"))
        .filter(|suffix| !suffix.is_empty())
        .ok_or_else(|| refusal("invalid_store"))?;
    let generation_name = format!("generation-{suffix}");
    if !safe_generation_name(&generation_name) {
        return Err(refusal("invalid_store"));
    }
    Ok((staging, generation_name))
}

fn install_staged_runtime<R: CommandRunner>(
    runner: &mut R,
    staging: &Path,
    request: &BootstrapRequest,
    pin: &PinManifest,
) -> Result<(BTreeMap<String, String>, String, String), StoreError> {
    let runtime = staging.join("runtime");
    fs::create_dir(&runtime).map_err(|_| refusal("invalid_store"))?;
    let environment = environment_for_runtime(&runtime, true)?;
    let wrapper = staging.join("plasmosome-work-state");
    copy_regular_file(&request.wrapper, &wrapper)?;
    owner_private_executable(&wrapper)?;
    File::open(&wrapper)
        .and_then(|file| file.sync_all())
        .map_err(|_| refusal("invalid_store"))?;
    let binary = staging.join("bd");
    copy_regular_file(&request.binary, &binary)?;
    owner_private_executable(&binary)?;
    File::open(&binary)
        .and_then(|file| file.sync_all())
        .map_err(|_| refusal("invalid_store"))?;
    InstalledBeads::verify(
        pin,
        &request.host_target,
        &binary,
        environment.clone(),
        runner,
    )
    .map_err(pin_refusal)?;
    Ok((environment, sha256(&wrapper)?, sha256(&binary)?))
}

fn initialize_private_repository<R: CommandRunner>(
    runner: &mut R,
    staging: &Path,
    binary: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<(), StoreError> {
    let repository = staging.join("repository");
    fs::create_dir(&repository).map_err(|_| refusal("invalid_store"))?;
    for argv in [
        vec!["init".into(), "--quiet".into()],
        vec![
            "config".into(),
            "user.email".into(),
            "plasmosome@local.invalid".into(),
        ],
        vec![
            "config".into(),
            "user.name".into(),
            "Plasmosome local shadow".into(),
        ],
        vec!["config".into(), "dolt.auto-push".into(), "false".into()],
    ] {
        run_bootstrap_command(
            runner,
            bootstrap_command("git", argv, &repository, environment),
        )?;
    }
    run_bootstrap_command(
        runner,
        bootstrap_command(
            binary,
            vec![
                "--sandbox".into(),
                "init".into(),
                "--stealth".into(),
                "--skip-agents".into(),
                "--skip-hooks".into(),
                "--non-interactive".into(),
            ],
            &repository,
            environment,
        ),
    )?;
    Ok(())
}

fn commit_bootstrap<R: CommandRunner>(
    runner: &mut R,
    staging: &Path,
    source_commit: &str,
    environment: &BTreeMap<String, String>,
) -> Result<String, StoreError> {
    let repository = staging.join("repository");
    let binary = staging.join("bd");
    run_bootstrap_command(
        runner,
        bootstrap_command(
            &binary,
            vec![
                "--sandbox".into(),
                "dolt".into(),
                "commit".into(),
                "-m".into(),
                format!("bootstrap markdown-shadow {source_commit}"),
            ],
            &repository,
            environment,
        ),
    )?;
    let status = run_bootstrap_command(
        runner,
        bootstrap_command(
            binary,
            vec![
                "--readonly".into(),
                "--sandbox".into(),
                "--json".into(),
                "vc".into(),
                "status".into(),
            ],
            &repository,
            environment,
        ),
    )?;
    version_control_commit(&status)
}

fn manifest_for_snapshot(
    source: &SourceDocuments,
    documents: &[OperationalDocument],
    local_generation: String,
    host_target: String,
    wrapper_sha256: String,
    beads_binary_sha256: String,
) -> Result<StateManifest, StoreError> {
    let logical =
        canonical_logical_export(&source.documents).map_err(|_| refusal("invalid_store"))?;
    let operational =
        canonical_operational_projection(documents).map_err(|_| refusal("invalid_store"))?;
    Ok(StateManifest {
        schema_version: 1,
        authority_mode: "markdown-shadow".into(),
        source_commit: source.source_commit.clone(),
        logical_export_sha256: logical_export_digest(&logical),
        operational_projection_sha256: operational_projection_digest(&operational),
        local_generation,
        host_target,
        wrapper_sha256,
        beads_binary_sha256,
        remote_relation: RemoteRelation::Unknown,
        remote_generation: None,
        remote_observed_at: None,
        observed_local_generation: None,
        last_successful_sync_at: None,
        pending_operation_ids: Vec::new(),
    })
}

fn write_staged_manifest(staging: &Path, manifest: &StateManifest) -> Result<(), StoreError> {
    let contents = serde_json::to_vec(manifest).map_err(|_| refusal("invalid_store"))?;
    write_new_sync(&staging.join("state.json"), &contents)
}

fn verify_source_snapshot(
    source: &SourceDocuments,
    snapshot: &FencedSnapshot,
) -> Result<(), StoreError> {
    let documents = snapshot
        .documents
        .iter()
        .map(|document| document.document.clone())
        .collect::<Vec<_>>();
    compare_shadow_parity(&source.documents, &documents).map_err(|_| refusal("invalid_store"))
}

fn stage_generation_from_markdown<R: CommandRunner>(
    runner: &mut R,
    location: &StoreLocation,
    request: &BootstrapRequest,
    pin: &PinManifest,
    source: &SourceDocuments,
) -> Result<BootstrapResult, StoreError> {
    let (staging, generation_name) = create_generation_staging(location)?;
    (|| {
        let (environment, wrapper_sha256, beads_binary_sha256) =
            install_staged_runtime(runner, &staging, request, pin)?;
        let binary = staging.join("bd");
        initialize_private_repository(runner, &staging, &binary, &environment)?;
        let operational = initial_operational_metadata(&source.documents)
            .map_err(|_| refusal("invalid_store"))?;
        let imported = import_operational_shadow_documents(
            runner,
            &ShadowStore::new(
                "bootstrap",
                staging.join("runtime/tmp"),
                staging.join("repository"),
                environment.clone(),
                binary,
            ),
            &source.source_commit,
            &source.documents,
            &operational,
        )
        .map_err(|_| refusal("invalid_store"))?;
        let local_generation =
            commit_bootstrap(runner, &staging, &source.source_commit, &environment)?;
        let manifest = manifest_for_snapshot(
            source,
            &imported.documents,
            local_generation,
            request.host_target.clone(),
            wrapper_sha256,
            beads_binary_sha256,
        )?;
        write_staged_manifest(&staging, &manifest)?;
        let staged = CurrentGeneration {
            name: generation_name.clone(),
            root: staging.clone(),
            manifest: manifest.clone(),
        };
        let snapshot = read_disposable_snapshot(runner, &staged, pin, &request.host_target)?;
        verify_source_snapshot(source, &snapshot)?;
        activate_staged_generation(location, &staging, &generation_name, None)?;
        Ok(BootstrapResult {
            outcome: BootstrapOutcome::Installed,
            source_commit: source.source_commit.clone(),
            local_generation: manifest.local_generation,
            document_counts: bootstrap_counts(source),
            logical_export_sha256: manifest.logical_export_sha256,
        })
    })()
}

fn wrapper_is_valid(generation: &CurrentGeneration) -> Result<(), StoreError> {
    let wrapper = generation.root.join("plasmosome-work-state");
    if sha256(&wrapper)? != generation.manifest.wrapper_sha256 {
        return Err(refusal("invalid_store"));
    }
    Ok(())
}

fn stage_runtime_reinstall<R: CommandRunner>(
    runner: &mut R,
    location: &StoreLocation,
    request: &BootstrapRequest,
    pin: &PinManifest,
    source: &SourceDocuments,
    existing: &CurrentGeneration,
    snapshot: &FencedSnapshot,
) -> Result<BootstrapResult, StoreError> {
    let (staging, generation_name) = create_generation_staging(location)?;
    (|| {
        let (environment, wrapper_sha256, beads_binary_sha256) =
            install_staged_runtime(runner, &staging, request, pin)?;
        copy_private_tree(
            &existing.root.join("repository"),
            &staging.join("repository"),
        )?;
        let mut manifest = existing.manifest.clone();
        manifest.host_target = request.host_target.clone();
        manifest.wrapper_sha256 = wrapper_sha256;
        manifest.beads_binary_sha256 = beads_binary_sha256;
        write_staged_manifest(&staging, &manifest)?;
        let staged = CurrentGeneration {
            name: generation_name.clone(),
            root: staging.clone(),
            manifest: manifest.clone(),
        };
        let copied_snapshot = read_disposable_snapshot(runner, &staged, pin, &request.host_target)?;
        if copied_snapshot != *snapshot {
            return Err(refusal("invalid_store"));
        }
        verify_source_snapshot(source, &copied_snapshot)?;
        let _ = environment;
        activate_staged_generation(location, &staging, &generation_name, None)?;
        Ok(BootstrapResult {
            outcome: BootstrapOutcome::Reinstalled,
            source_commit: manifest.source_commit,
            local_generation: manifest.local_generation,
            document_counts: bootstrap_counts(source),
            logical_export_sha256: manifest.logical_export_sha256,
        })
    })()
}

/// Performs the only explicit installation path for one clone-local Markdown shadow generation.
pub fn bootstrap(request: &BootstrapRequest) -> Result<BootstrapResult, StoreError> {
    let pin = PinManifest::load(
        request
            .source_root
            .join("tools/work-state-beads-1.1.2.toml"),
    )
    .map_err(pin_refusal)?;
    let verification_root = tempfile::tempdir().map_err(|_| refusal("invalid_store"))?;
    let verification_environment = environment_for_runtime(verification_root.path(), true)?;
    let mut runner =
        BootstrapCommandRunner::new(request.source_root.clone(), request.binary.clone());
    VerifiedBeads::verify_with_environment(
        &pin,
        &request.host_target,
        &request.archive,
        &request.binary,
        verification_environment.clone(),
        &mut runner,
    )
    .map_err(pin_refusal)?;
    let location = locate_store(
        &mut runner,
        &request.checkout,
        verification_environment.clone(),
    )?;
    let _lock = BootstrapLock::acquire(&location)?;
    let source = load_documents(
        &mut runner,
        &request.source_root,
        &verification_environment,
        &request.source_ref,
    )
    .map_err(source_refusal)?;
    let Some(existing) = optional_current_generation(&location)? else {
        return stage_generation_from_markdown(&mut runner, &location, request, &pin, &source);
    };
    if existing.manifest.source_commit != source.source_commit {
        return Err(refusal("source_commit_mismatch"));
    }
    let runtime_valid = wrapper_is_valid(&existing).is_ok()
        && read_disposable_snapshot(&mut runner, &existing, &pin, &request.host_target).is_ok();
    let snapshot = if runtime_valid {
        read_disposable_snapshot(&mut runner, &existing, &pin, &request.host_target)?
    } else {
        read_disposable_snapshot_with_binary(
            &mut runner,
            &existing,
            &pin,
            &request.host_target,
            &request.binary,
        )?
    };
    verify_source_snapshot(&source, &snapshot)?;
    if runtime_valid {
        return Ok(BootstrapResult {
            outcome: BootstrapOutcome::Unchanged,
            source_commit: existing.manifest.source_commit,
            local_generation: existing.manifest.local_generation,
            document_counts: bootstrap_counts(&source),
            logical_export_sha256: existing.manifest.logical_export_sha256,
        });
    }
    stage_runtime_reinstall(
        &mut runner,
        &location,
        request,
        &pin,
        &source,
        &existing,
        &snapshot,
    )
}
