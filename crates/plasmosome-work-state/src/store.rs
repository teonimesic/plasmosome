use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::command::{CommandRunner, CommandSpec, SystemCommandRunner};
use crate::document::{
    SourceDocuments, discovered_document_paths, is_lower_hex_sha, load_documents,
};
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
            Ok(_) => {
                directory_without_symlink(&location.state_root)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&location.state_root).map_err(|_| refusal("invalid_store"))?;
                directory_without_symlink(&location.state_root)?;
            }
            Err(_) => return Err(refusal("invalid_store")),
        }
        sync_directory(&location.common_dir)?;
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

fn sync_regular_tree(path: &Path) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| refusal("invalid_store"))?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        for entry in fs::read_dir(path).map_err(|_| refusal("invalid_store"))? {
            let entry = entry.map_err(|_| refusal("invalid_store"))?;
            sync_regular_tree(&entry.path())?;
        }
        return sync_directory(path);
    }
    if metadata.file_type().is_file() && !metadata.file_type().is_symlink() {
        return File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|_| refusal("invalid_store"));
    }
    Err(refusal("invalid_store"))
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
    sync_regular_tree(staging)?;
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
    CommandSpec {
        program: PathBuf::from("git"),
        argv,
        cwd: Some(checkout.to_path_buf()),
        environment: environment.clone(),
        redacted_argv_positions: Vec::new(),
    }
}

#[derive(Clone, Copy)]
enum LocatorScope {
    OrdinaryRead,
    Bootstrap,
}

fn run_locator<R: CommandRunner>(
    runner: &mut R,
    checkout: &Path,
    environment: &BTreeMap<String, String>,
    argv: Vec<String>,
    scope: LocatorScope,
) -> Result<PathBuf, StoreError> {
    let command = locator_command(checkout, environment, argv);
    if matches!(scope, LocatorScope::OrdinaryRead) {
        validate_read_locator_command(&command, checkout)
            .map_err(|_| refusal("invalid_store_location"))?;
    }
    let output = runner
        .run(command)
        .map_err(|_| refusal("invalid_store_location"))?;
    if output.status != 0 {
        return Err(refusal("invalid_store_location"));
    }
    one_absolute_path(&output.stdout).ok_or_else(|| refusal("invalid_store_location"))
}

/// Resolves the one state root shared by linked worktrees without creating it.
fn locate_store_for_scope<R: CommandRunner>(
    runner: &mut R,
    checkout: &Path,
    environment: BTreeMap<String, String>,
    scope: LocatorScope,
) -> Result<StoreLocation, StoreError> {
    let supplied_checkout = canonical_existing_directory(checkout)?;
    let top_level = run_locator(
        runner,
        &supplied_checkout,
        &environment,
        vec!["rev-parse".into(), "--show-toplevel".into()],
        scope,
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
        scope,
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

/// Resolves the shared store using only the sealed ordinary-read locator environment.
pub fn locate_store<R: CommandRunner>(
    runner: &mut R,
    checkout: &Path,
    environment: BTreeMap<String, String>,
) -> Result<StoreLocation, StoreError> {
    locate_store_for_scope(runner, checkout, environment, LocatorScope::OrdinaryRead)
}

fn locate_bootstrap_store(
    runner: &mut BootstrapCommandRunner,
    checkout: &Path,
    environment: BTreeMap<String, String>,
) -> Result<StoreLocation, StoreError> {
    locate_store_for_scope(runner, checkout, environment, LocatorScope::Bootstrap)
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

fn owner_private_executable_is_valid(path: &Path) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| refusal("invalid_store"))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(refusal("invalid_store"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o7777 != 0o700 {
            return Err(refusal("invalid_store"));
        }
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

struct ReadVersionRunner<'a, R> {
    inner: &'a mut R,
    installed_binary: &'a Path,
    installed_environment: BTreeMap<String, String>,
    copied_binary: &'a Path,
    copied_environment: BTreeMap<String, String>,
}

impl<'a, R> ReadVersionRunner<'a, R> {
    fn new(
        inner: &'a mut R,
        installed_binary: &'a Path,
        installed_environment: BTreeMap<String, String>,
        copied_binary: &'a Path,
        copied_environment: BTreeMap<String, String>,
    ) -> Self {
        Self {
            inner,
            installed_binary,
            installed_environment,
            copied_binary,
            copied_environment,
        }
    }

    fn valid(&self, command: &CommandSpec) -> bool {
        command.argv == ["--version"]
            && command.cwd.is_none()
            && command.redacted_argv_positions.is_empty()
            && ((command.program == self.installed_binary
                && command.environment == self.installed_environment)
                || (command.program == self.copied_binary
                    && command.environment == self.copied_environment))
    }
}

impl<R: CommandRunner> CommandRunner for ReadVersionRunner<'_, R> {
    fn run(&mut self, command: CommandSpec) -> Result<crate::command::CommandOutput, String> {
        if !self.valid(&command) {
            return Err("invalid_read_command".into());
        }
        self.inner.run(command)
    }
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
    let expected_environment =
        locator_environment().map_err(|_| refusal("invalid_read_command"))?;
    if command.program != Path::new("git")
        || command.cwd.as_deref() != Some(checkout)
        || command.environment != expected_environment
        || !command.redacted_argv_positions.is_empty()
    {
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

#[derive(Clone)]
enum SourceCommandCapture {
    ResolvedCommit,
    DiscoveredPaths,
    SelectedContents(String),
    ContentCommit(String),
    EstablishedContents(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SourcePathPhase {
    AwaitSelectedContents,
    AwaitContentCommit,
    AwaitEstablishedContents(String),
    Complete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SourcePathDiscovery {
    NotRun,
    Rejected,
    Paths(BTreeMap<String, SourcePathPhase>),
}

impl SourcePathDiscovery {
    fn paths(&self) -> Option<&BTreeMap<String, SourcePathPhase>> {
        match self {
            Self::Paths(paths) => Some(paths),
            Self::NotRun | Self::Rejected => None,
        }
    }

    fn paths_mut(&mut self) -> Option<&mut BTreeMap<String, SourcePathPhase>> {
        match self {
            Self::Paths(paths) => Some(paths),
            Self::NotRun | Self::Rejected => None,
        }
    }
}

/// The production-only command fence for bootstrap. It binds every dynamic root before it can
/// reach the system runner; generic recording runners retain their narrow unit-test seam.
struct BootstrapCommandRunner {
    source_root: PathBuf,
    checkout: PathBuf,
    requested_ref: String,
    initial_binary: PathBuf,
    verification_environment: BTreeMap<String, String>,
    resolved_source_commit: Option<String>,
    source_paths: SourcePathDiscovery,
    location: Option<StoreLocation>,
    staging_roots: BTreeMap<PathBuf, BTreeMap<String, String>>,
    installed_roots: BTreeMap<PathBuf, BTreeMap<String, String>>,
    disposable_roots: BTreeMap<PathBuf, BTreeMap<String, String>>,
    inner: SystemCommandRunner,
}

impl BootstrapCommandRunner {
    fn new(source_root: PathBuf, initial_binary: PathBuf) -> Self {
        Self {
            checkout: source_root.clone(),
            source_root,
            requested_ref: "origin/main".into(),
            initial_binary,
            verification_environment: BTreeMap::new(),
            resolved_source_commit: None,
            source_paths: SourcePathDiscovery::NotRun,
            location: None,
            staging_roots: BTreeMap::new(),
            installed_roots: BTreeMap::new(),
            disposable_roots: BTreeMap::new(),
            inner: SystemCommandRunner,
        }
    }

    fn bind_source_inputs(
        &mut self,
        requested_ref: String,
        verification_environment: BTreeMap<String, String>,
    ) -> Result<(), StoreError> {
        if requested_ref.trim().is_empty() || requested_ref.contains(['\n', '\r']) {
            return Err(refusal("invalid_source_ref"));
        }
        directory_without_symlink(&self.source_root)?;
        if fs::canonicalize(&self.source_root).map_err(|_| refusal("invalid_store"))?
            != self.source_root
        {
            return Err(refusal("invalid_store"));
        }
        self.requested_ref = requested_ref;
        self.verification_environment = verification_environment;
        Ok(())
    }

    fn bind_checkout(&mut self, checkout: PathBuf) -> Result<(), StoreError> {
        directory_without_symlink(&checkout)?;
        if fs::canonicalize(&checkout).map_err(|_| refusal("invalid_store"))? != checkout {
            return Err(refusal("invalid_store"));
        }
        self.checkout = checkout;
        Ok(())
    }

    fn bind_location(&mut self, location: StoreLocation) -> Result<(), StoreError> {
        if location.worktree_root != self.checkout
            || location.state_root != location.common_dir.join(STORE_DIRECTORY)
            || location.generations_dir != location.state_root.join(GENERATIONS_DIRECTORY)
        {
            return Err(refusal("invalid_store"));
        }
        directory_without_symlink(&location.common_dir)?;
        if fs::canonicalize(&location.common_dir).map_err(|_| refusal("invalid_store"))?
            != location.common_dir
        {
            return Err(refusal("invalid_store"));
        }
        self.location = Some(location);
        Ok(())
    }

    fn register_staging(
        &mut self,
        root: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<(), StoreError> {
        let location = self
            .location
            .as_ref()
            .ok_or_else(|| refusal("invalid_store"))?;
        if root.parent() != Some(location.generations_dir.as_path())
            || !root
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".staging-") && name.len() > ".staging-".len())
        {
            return Err(refusal("invalid_store"));
        }
        directory_without_symlink(root)?;
        if fs::canonicalize(root).map_err(|_| refusal("invalid_store"))? != root
            || environment_for_runtime(&root.join("runtime"), false)? != *environment
            || self.staging_roots.contains_key(root)
        {
            return Err(refusal("invalid_store"));
        }
        self.staging_roots
            .insert(root.to_path_buf(), environment.clone());
        Ok(())
    }

    fn register_installed_generation(
        &mut self,
        generation: &CurrentGeneration,
        environment: &BTreeMap<String, String>,
    ) -> Result<(), StoreError> {
        let location = self
            .location
            .as_ref()
            .ok_or_else(|| refusal("invalid_store"))?;
        if generation.root.parent() != Some(location.generations_dir.as_path())
            || !safe_generation_name(&generation.name)
            || generation.root.file_name().and_then(|name| name.to_str())
                != Some(generation.name.as_str())
            || environment_for_runtime(&generation.root.join("runtime"), false)? != *environment
        {
            return Err(refusal("invalid_store"));
        }
        directory_without_symlink(&generation.root)?;
        if fs::canonicalize(&generation.root).map_err(|_| refusal("invalid_store"))?
            != generation.root
        {
            return Err(refusal("invalid_store"));
        }
        self.installed_roots
            .insert(generation.root.clone(), environment.clone());
        Ok(())
    }

    fn register_disposable(
        &mut self,
        root: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<(), StoreError> {
        directory_without_symlink(root)?;
        fs::canonicalize(root).map_err(|_| refusal("invalid_store"))?;
        if environment_for_runtime(&root.join("runtime"), false)? != *environment
            || self.disposable_roots.contains_key(root)
        {
            return Err(refusal("invalid_store"));
        }
        self.disposable_roots
            .insert(root.to_path_buf(), environment.clone());
        Ok(())
    }

    fn unregister_disposable(&mut self, root: &Path) {
        self.disposable_roots.remove(root);
    }

    fn source_environment(&self) -> BTreeMap<String, String> {
        let mut environment = self.verification_environment.clone();
        environment.insert("GIT_NO_LAZY_FETCH".into(), "1".into());
        environment
    }

    fn locator_environment(&self) -> BTreeMap<String, String> {
        let mut environment = self.verification_environment.clone();
        for (key, value) in [
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_NO_LAZY_FETCH", "1"),
            ("GIT_OPTIONAL_LOCKS", "0"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ] {
            environment.insert(key.into(), value.into());
        }
        environment
    }

    fn source_command_capture(&self, command: &CommandSpec) -> Option<SourceCommandCapture> {
        if command.program != Path::new("git")
            || command.cwd.as_deref() != Some(self.source_root.as_path())
            || command.environment != self.source_environment()
        {
            return None;
        }
        let resolved = self.resolved_source_commit.as_deref();
        match command.argv.as_slice() {
            [program, verify, end_of_options, reference]
                if self.resolved_source_commit.is_none()
                    && program == "rev-parse"
                    && verify == "--verify"
                    && end_of_options == "--end-of-options"
                    && reference == &format!("{}^{{commit}}", self.requested_ref) =>
            {
                Some(SourceCommandCapture::ResolvedCommit)
            }
            [
                program,
                recursive,
                names,
                nul,
                commit,
                separator,
                intents,
                specs,
                tasks,
            ] if program == "ls-tree"
                && recursive == "-r"
                && names == "--name-only"
                && nul == "-z"
                && matches!(self.source_paths, SourcePathDiscovery::NotRun)
                && Some(commit.as_str()) == resolved
                && separator == "--"
                && intents == "docs/intents"
                && specs == "docs/specs"
                && tasks == "tasks" =>
            {
                Some(SourceCommandCapture::DiscoveredPaths)
            }
            [program, object] if program == "show" => {
                let (commit, path) = object.split_once(':')?;
                let SourcePathDiscovery::Paths(paths) = &self.source_paths else {
                    return None;
                };
                match paths.get(path)? {
                    SourcePathPhase::AwaitSelectedContents
                        if Some(commit) == resolved
                            && !path.is_empty()
                            && !path.contains(['\n', '\r']) =>
                    {
                        Some(SourceCommandCapture::SelectedContents(path.to_owned()))
                    }
                    SourcePathPhase::AwaitEstablishedContents(expected)
                        if expected == commit
                            && !path.is_empty()
                            && !path.contains(['\n', '\r']) =>
                    {
                        Some(SourceCommandCapture::EstablishedContents(path.to_owned()))
                    }
                    _ => None,
                }
            }
            [program, one, format, commit, separator, literal_path]
                if program == "log"
                    && one == "-1"
                    && format == "--format=%H"
                    && Some(commit.as_str()) == resolved
                    && separator == "--"
                    && literal_path.starts_with(":(literal)")
                    && literal_path.len() > ":(literal)".len()
                    && !literal_path.contains(['\n', '\r'])
                    && self
                        .source_paths
                        .paths()
                        .and_then(|paths| paths.get(&literal_path[":(literal)".len()..]))
                        .is_some_and(|phase| {
                            matches!(phase, SourcePathPhase::AwaitContentCommit)
                        }) =>
            {
                Some(SourceCommandCapture::ContentCommit(
                    literal_path[":(literal)".len()..].to_owned(),
                ))
            }
            _ => None,
        }
    }

    fn valid_locator(&self, command: &CommandSpec) -> bool {
        command.program == Path::new("git")
            && command.cwd.as_deref() == Some(self.checkout.as_path())
            && command.environment == self.locator_environment()
            && bootstrap_locator_command(command)
    }

    fn valid_initial_version(&self, command: &CommandSpec) -> bool {
        command.program == self.initial_binary
            && command.argv == ["--version"]
            && command.cwd.is_none()
            && command.environment == self.verification_environment
    }

    fn valid_staging_command(&self, command: &CommandSpec) -> bool {
        self.staging_roots.iter().any(|(root, environment)| {
            let repository = root.join("repository");
            let binary = root.join("bd");
            if command.environment != *environment {
                return false;
            }
            if command.program == binary && command.argv == ["--version"] && command.cwd.is_none() {
                return true;
            }
            let valid_repository_command = if command.program == Path::new("git") {
                command.cwd.as_deref() == Some(repository.as_path())
                    && bootstrap_repository_git_command(&command.argv)
            } else {
                command.program == binary
                    && command.cwd.as_deref() == Some(repository.as_path())
                    && bootstrap_beads_command(&command.argv)
            };
            if !valid_repository_command {
                return false;
            }
            if matches!(command.argv.as_slice(), [sandbox, import, path, json] if sandbox == "--sandbox" && import == "import" && json == "--json") {
                let import_path = Path::new(&command.argv[2]);
                return import_path.parent() == Some(root.join("runtime/tmp").as_path())
                    && regular_file(import_path).is_ok();
            }
            if let [sandbox, dolt, commit, message, value] = command.argv.as_slice()
                && sandbox == "--sandbox"
                && dolt == "dolt"
                && commit == "commit"
                && message == "-m"
            {
                return value
                    .strip_prefix("bootstrap markdown-shadow ")
                    .is_some_and(|commit| Some(commit) == self.resolved_source_commit.as_deref());
            }
            if let [sandbox, kv, set, key, value] = command.argv.as_slice()
                && sandbox == "--sandbox"
                && kv == "kv"
                && set == "set"
                && key == "plasmosome.source-commit"
            {
                return Some(value.as_str()) == self.resolved_source_commit.as_deref();
            }
            true
        })
    }

    fn valid_installed_version(&self, command: &CommandSpec) -> bool {
        self.installed_roots.iter().any(|(root, environment)| {
            command.program == root.join("bd")
                && command.argv == ["--version"]
                && command.cwd.is_none()
                && command.environment == *environment
        })
    }

    fn valid_disposable_command(&self, command: &CommandSpec) -> bool {
        self.disposable_roots.iter().any(|(root, environment)| {
            command.environment == *environment
                && validate_read_command(command, &root.join("repository"), &root.join("bd"))
                    .is_ok()
        })
    }

    fn record_source_output(
        &mut self,
        capture: SourceCommandCapture,
        output: &crate::command::CommandOutput,
    ) -> Result<(), String> {
        if output.status != 0 {
            return Ok(());
        }
        let value = output
            .stdout
            .strip_suffix('\n')
            .filter(|value| !value.contains(['\n', '\r']))
            .filter(|value| is_lower_hex_sha(value))
            .map(str::to_owned);
        match capture {
            SourceCommandCapture::ResolvedCommit => {
                if let Some(commit) = value {
                    self.resolved_source_commit = Some(commit);
                }
            }
            SourceCommandCapture::DiscoveredPaths => {
                self.source_paths = match discovered_document_paths(&output.stdout) {
                    Ok(paths) => SourcePathDiscovery::Paths(
                        paths
                            .into_iter()
                            .map(|path| (path, SourcePathPhase::AwaitSelectedContents))
                            .collect(),
                    ),
                    Err(_) => SourcePathDiscovery::Rejected,
                };
            }
            SourceCommandCapture::SelectedContents(path) => {
                let Some(phase) = self
                    .source_paths
                    .paths_mut()
                    .and_then(|paths| paths.get_mut(&path))
                else {
                    return Err("invalid_bootstrap_command".into());
                };
                if !matches!(phase, SourcePathPhase::AwaitSelectedContents) {
                    return Err("invalid_bootstrap_command".into());
                }
                *phase = SourcePathPhase::AwaitContentCommit;
            }
            SourceCommandCapture::ContentCommit(path) => {
                let Some(commit) = value else {
                    return Ok(());
                };
                let Some(phase) = self
                    .source_paths
                    .paths_mut()
                    .and_then(|paths| paths.get_mut(&path))
                else {
                    return Err("invalid_bootstrap_command".into());
                };
                if !matches!(phase, SourcePathPhase::AwaitContentCommit) {
                    return Err("invalid_bootstrap_command".into());
                }
                *phase = SourcePathPhase::AwaitEstablishedContents(commit);
            }
            SourceCommandCapture::EstablishedContents(path) => {
                let Some(phase) = self
                    .source_paths
                    .paths_mut()
                    .and_then(|paths| paths.get_mut(&path))
                else {
                    return Err("invalid_bootstrap_command".into());
                };
                if !matches!(phase, SourcePathPhase::AwaitEstablishedContents(_)) {
                    return Err("invalid_bootstrap_command".into());
                }
                *phase = SourcePathPhase::Complete;
            }
        }
        Ok(())
    }
}

impl CommandRunner for BootstrapCommandRunner {
    fn run(&mut self, command: CommandSpec) -> Result<crate::command::CommandOutput, String> {
        let source_capture = self.source_command_capture(&command);
        let valid = source_capture.is_some()
            || self.valid_initial_version(&command)
            || self.valid_locator(&command)
            || self.valid_staging_command(&command)
            || self.valid_installed_version(&command)
            || self.valid_disposable_command(&command);
        if !valid {
            return Err("invalid_bootstrap_command".into());
        }
        let output = self.inner.run(command)?;
        if let Some(capture) = source_capture {
            self.record_source_output(capture, &output)?;
        }
        Ok(output)
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

struct DisposableSnapshotRequest<'a> {
    pin: &'a PinManifest,
    target: &'a str,
    selected_binary: &'a Path,
    selected_environment: BTreeMap<String, String>,
    require_matching_installed_runtime: bool,
    temporary_root: &'a Path,
    copied_environment: BTreeMap<String, String>,
}

fn read_disposable_snapshot_in_root<R: CommandRunner>(
    runner: &mut R,
    generation: &CurrentGeneration,
    request: DisposableSnapshotRequest<'_>,
) -> Result<FencedSnapshot, StoreError> {
    let expected_binary_sha = request
        .pin
        .targets
        .iter()
        .find(|candidate| candidate.target == request.target)
        .ok_or_else(|| refusal("unsupported_beads_platform"))?
        .binary_sha256
        .as_str();
    if request.require_matching_installed_runtime
        && (generation.manifest.host_target != request.target
            || generation.manifest.beads_binary_sha256 != expected_binary_sha)
    {
        return Err(refusal("invalid_store"));
    }
    if request.require_matching_installed_runtime
        && !matches!(
            fs::symlink_metadata(request.selected_binary),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        )
    {
        owner_private_executable_is_valid(request.selected_binary)?;
    }
    let repository = request.temporary_root.join("repository");
    let copied_binary = request.temporary_root.join("bd");
    {
        let mut version_runner = ReadVersionRunner::new(
            runner,
            request.selected_binary,
            request.selected_environment.clone(),
            &copied_binary,
            request.copied_environment.clone(),
        );
        InstalledBeads::verify(
            request.pin,
            request.target,
            request.selected_binary,
            request.selected_environment.clone(),
            &mut version_runner,
        )
        .map_err(pin_refusal)?;
        copy_private_tree(&generation.root.join("repository"), &repository)?;
        copy_regular_file(request.selected_binary, &copied_binary)?;
        InstalledBeads::verify(
            request.pin,
            request.target,
            &copied_binary,
            request.copied_environment.clone(),
            &mut version_runner,
        )
        .map_err(pin_refusal)?;
    }
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
        environment: request.copied_environment.clone(),
        redacted_argv_positions: Vec::new(),
    };
    let before = run_read_command(runner, status_command(), &repository, &copied_binary)?;
    let export = run_read_command(
        runner,
        CommandSpec {
            program: copied_binary.clone(),
            argv: vec!["--readonly".into(), "--sandbox".into(), "export".into()],
            cwd: Some(repository.clone()),
            environment: request.copied_environment.clone(),
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
            environment: request.copied_environment.clone(),
            redacted_argv_positions: Vec::new(),
        },
        &repository,
        &copied_binary,
    )?;
    let after = run_read_command(runner, status_command(), &repository, &copied_binary)?;
    validate_fenced_snapshot(&generation.manifest, &before, &export, &key_values, &after)
}

fn finish_disposable_snapshot(
    result: Result<FencedSnapshot, StoreError>,
    cleanup: std::io::Result<()>,
) -> Result<FencedSnapshot, StoreError> {
    if cleanup.is_err() {
        return Err(refusal("temporary_cleanup_failed"));
    }
    result
}

fn read_disposable_snapshot_with_binary<R: CommandRunner>(
    runner: &mut R,
    generation: &CurrentGeneration,
    pin: &PinManifest,
    target: &str,
    selected_binary: &Path,
    require_matching_installed_runtime: bool,
) -> Result<FencedSnapshot, StoreError> {
    let shared_environment = environment_for_runtime(&generation.root.join("runtime"), false)?;
    let temporary_root = tempfile::Builder::new()
        .prefix("plasmosome-read-")
        .tempdir()
        .map_err(|_| refusal("invalid_store"))?;
    let result = (|| {
        let copied_environment =
            environment_for_runtime(&temporary_root.path().join("runtime"), true)?;
        read_disposable_snapshot_in_root(
            runner,
            generation,
            DisposableSnapshotRequest {
                pin,
                target,
                selected_binary,
                selected_environment: shared_environment,
                require_matching_installed_runtime,
                temporary_root: temporary_root.path(),
                copied_environment,
            },
        )
    })();
    finish_disposable_snapshot(result, temporary_root.close())
}

fn bootstrap_read_disposable_snapshot_with_binary(
    runner: &mut BootstrapCommandRunner,
    generation: &CurrentGeneration,
    pin: &PinManifest,
    target: &str,
    selected_binary: &Path,
    selected_environment: BTreeMap<String, String>,
    require_matching_installed_runtime: bool,
) -> Result<FencedSnapshot, StoreError> {
    let temporary_root = tempfile::Builder::new()
        .prefix("plasmosome-read-")
        .tempdir()
        .map_err(|_| refusal("invalid_store"))?;
    let result = (|| {
        let copied_environment =
            environment_for_runtime(&temporary_root.path().join("runtime"), true)?;
        runner.register_disposable(temporary_root.path(), &copied_environment)?;
        read_disposable_snapshot_in_root(
            runner,
            generation,
            DisposableSnapshotRequest {
                pin,
                target,
                selected_binary,
                selected_environment,
                require_matching_installed_runtime,
                temporary_root: temporary_root.path(),
                copied_environment,
            },
        )
    })();
    runner.unregister_disposable(temporary_root.path());
    finish_disposable_snapshot(result, temporary_root.close())
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
        true,
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
    Ok(BTreeMap::from([
        ("PATH".to_owned(), path.to_string_lossy().into_owned()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_NO_LAZY_FETCH".into(), "1".into()),
        ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
    ]))
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

/// Binds an installed wrapper to its own immutable generation without reading `current`.
pub fn generation_for_installed_wrapper(
    location: &StoreLocation,
    executable: &Path,
) -> Result<CurrentGeneration, StoreError> {
    if location.state_root != location.common_dir.join(STORE_DIRECTORY)
        || location.generations_dir != location.state_root.join(GENERATIONS_DIRECTORY)
        || !executable.is_absolute()
    {
        return Err(refusal("invalid_store"));
    }
    if !safe_state_root_exists(location)? {
        return Err(refusal("not_initialized"));
    }
    for directory in [
        &location.common_dir,
        &location.state_root,
        &location.generations_dir,
    ] {
        directory_without_symlink(directory)?;
        if fs::canonicalize(directory).map_err(|_| refusal("invalid_store"))? != *directory {
            return Err(refusal("invalid_store"));
        }
    }
    let executable_metadata =
        fs::symlink_metadata(executable).map_err(|_| refusal("invalid_store"))?;
    if !executable_metadata.file_type().is_file() || executable_metadata.file_type().is_symlink() {
        return Err(refusal("invalid_store"));
    }
    let canonical_executable =
        fs::canonicalize(executable).map_err(|_| refusal("invalid_store"))?;
    if canonical_executable != executable
        || canonical_executable
            .file_name()
            .and_then(|name| name.to_str())
            != Some("plasmosome-work-state")
    {
        return Err(refusal("invalid_store"));
    }
    let root = canonical_executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| refusal("invalid_store"))?;
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| safe_generation_name(name))
        .ok_or_else(|| refusal("invalid_store"))?;
    if root.parent() != Some(location.generations_dir.as_path()) {
        return Err(refusal("invalid_store"));
    }
    directory_without_symlink(&root)?;
    if fs::canonicalize(&root).map_err(|_| refusal("invalid_store"))? != root {
        return Err(refusal("invalid_store"));
    }
    let mut state = String::new();
    regular_file(&root.join("state.json"))?
        .read_to_string(&mut state)
        .map_err(|_| refusal("invalid_store"))?;
    let generation = CurrentGeneration {
        name: name.to_owned(),
        root,
        manifest: strict_manifest(&state)?,
    };
    wrapper_is_valid(&generation)?;
    Ok(generation)
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

fn install_staged_runtime(
    runner: &mut BootstrapCommandRunner,
    staging: &Path,
    request: &BootstrapRequest,
    pin: &PinManifest,
) -> Result<(BTreeMap<String, String>, String, String), StoreError> {
    let runtime = staging.join("runtime");
    fs::create_dir(&runtime).map_err(|_| refusal("invalid_store"))?;
    let environment = environment_for_runtime(&runtime, true)?;
    runner.register_staging(staging, &environment)?;
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

fn stage_generation_from_markdown(
    runner: &mut BootstrapCommandRunner,
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
        let snapshot = bootstrap_read_disposable_snapshot_with_binary(
            runner,
            &staged,
            pin,
            &request.host_target,
            &staged.root.join("bd"),
            environment.clone(),
            true,
        )?;
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
    if sha256(&wrapper)? != generation.manifest.wrapper_sha256
        || owner_private_executable_is_valid(&wrapper).is_err()
    {
        return Err(refusal("invalid_store"));
    }
    Ok(())
}

fn wrapper_matches_requested(
    generation: &CurrentGeneration,
    requested_wrapper: &Path,
) -> Result<(), StoreError> {
    wrapper_is_valid(generation)?;
    if sha256(requested_wrapper)? != generation.manifest.wrapper_sha256 {
        return Err(refusal("invalid_store"));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum InstalledRuntimePreflight {
    Usable {
        environment: BTreeMap<String, String>,
    },
    RepairRequired,
}

fn installed_runtime_preflight(
    runner: &mut BootstrapCommandRunner,
    generation: &CurrentGeneration,
    pin: &PinManifest,
    target: &str,
    requested_wrapper: &Path,
) -> Result<InstalledRuntimePreflight, StoreError> {
    if wrapper_matches_requested(generation, requested_wrapper).is_err() {
        return Ok(InstalledRuntimePreflight::RepairRequired);
    }
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
        return Ok(InstalledRuntimePreflight::RepairRequired);
    }
    if owner_private_executable_is_valid(&generation.root.join("bd")).is_err() {
        return Ok(InstalledRuntimePreflight::RepairRequired);
    }
    let environment = match environment_for_runtime(&generation.root.join("runtime"), false) {
        Ok(environment) => environment,
        Err(_) => return Ok(InstalledRuntimePreflight::RepairRequired),
    };
    runner.register_installed_generation(generation, &environment)?;
    if InstalledBeads::verify(
        pin,
        target,
        &generation.root.join("bd"),
        environment.clone(),
        runner,
    )
    .is_err()
    {
        return Ok(InstalledRuntimePreflight::RepairRequired);
    }
    Ok(InstalledRuntimePreflight::Usable { environment })
}

fn stage_runtime_reinstall(
    runner: &mut BootstrapCommandRunner,
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
        let copied_snapshot = bootstrap_read_disposable_snapshot_with_binary(
            runner,
            &staged,
            pin,
            &request.host_target,
            &staged.root.join("bd"),
            environment.clone(),
            true,
        )?;
        if copied_snapshot != *snapshot {
            return Err(refusal("invalid_store"));
        }
        verify_source_snapshot(source, &copied_snapshot)?;
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

struct PreparedInstall {
    runner: BootstrapCommandRunner,
    location: StoreLocation,
    pin: PinManifest,
    source: SourceDocuments,
}

struct PreparedReinstall {
    runner: BootstrapCommandRunner,
    location: StoreLocation,
    pin: PinManifest,
    source: SourceDocuments,
    existing: CurrentGeneration,
    snapshot: FencedSnapshot,
}

enum PreparedBootstrap {
    Install(Box<PreparedInstall>),
    Unchanged(BootstrapResult),
    Reinstall(Box<PreparedReinstall>),
}

fn finish_verification_cleanup<T>(
    preparation: Result<T, StoreError>,
    cleanup: Result<(), std::io::Error>,
) -> Result<T, StoreError> {
    if cleanup.is_err() {
        Err(refusal("temporary_cleanup_failed"))
    } else {
        preparation
    }
}

/// Performs the only explicit installation path for one clone-local Markdown shadow generation.
pub fn bootstrap(request: &BootstrapRequest) -> Result<BootstrapResult, StoreError> {
    let source_root = canonical_existing_directory(&request.source_root)?;
    let pin = PinManifest::load(source_root.join("tools/work-state-beads-1.1.2.toml"))
        .map_err(pin_refusal)?;
    let verification_root = tempfile::tempdir().map_err(|_| refusal("invalid_store"))?;
    let mut lock = None;
    let preparation = (|| {
        let verification_environment = environment_for_runtime(verification_root.path(), true)?;
        let mut runner = BootstrapCommandRunner::new(source_root.clone(), request.binary.clone());
        runner.bind_source_inputs(request.source_ref.clone(), verification_environment.clone())?;
        VerifiedBeads::verify_with_environment(
            &pin,
            &request.host_target,
            &request.archive,
            &request.binary,
            verification_environment.clone(),
            &mut runner,
        )
        .map_err(pin_refusal)?;
        let checkout = canonical_existing_directory(&request.checkout)?;
        runner.bind_checkout(checkout.clone())?;
        let location =
            locate_bootstrap_store(&mut runner, &checkout, verification_environment.clone())?;
        runner.bind_location(location.clone())?;
        lock = Some(BootstrapLock::acquire(&location)?);
        let source = load_documents(
            &mut runner,
            &source_root,
            &verification_environment,
            &request.source_ref,
        )
        .map_err(source_refusal)?;
        let Some(existing) = optional_current_generation(&location)? else {
            return Ok(PreparedBootstrap::Install(Box::new(PreparedInstall {
                runner,
                location,
                pin,
                source,
            })));
        };
        if existing.manifest.source_commit != source.source_commit {
            return Err(refusal("source_commit_mismatch"));
        }
        let (snapshot, runtime_valid) = match installed_runtime_preflight(
            &mut runner,
            &existing,
            &pin,
            &request.host_target,
            &request.wrapper,
        )? {
            InstalledRuntimePreflight::Usable { environment } => (
                bootstrap_read_disposable_snapshot_with_binary(
                    &mut runner,
                    &existing,
                    &pin,
                    &request.host_target,
                    &existing.root.join("bd"),
                    environment,
                    true,
                )?,
                true,
            ),
            InstalledRuntimePreflight::RepairRequired => (
                bootstrap_read_disposable_snapshot_with_binary(
                    &mut runner,
                    &existing,
                    &pin,
                    &request.host_target,
                    &request.binary,
                    verification_environment.clone(),
                    false,
                )?,
                false,
            ),
        };
        verify_source_snapshot(&source, &snapshot)?;
        if runtime_valid {
            return Ok(PreparedBootstrap::Unchanged(BootstrapResult {
                outcome: BootstrapOutcome::Unchanged,
                source_commit: existing.manifest.source_commit,
                local_generation: existing.manifest.local_generation,
                document_counts: bootstrap_counts(&source),
                logical_export_sha256: existing.manifest.logical_export_sha256,
            }));
        }
        Ok(PreparedBootstrap::Reinstall(Box::new(PreparedReinstall {
            runner,
            location,
            pin,
            source,
            existing,
            snapshot,
        })))
    })();
    let prepared = finish_verification_cleanup(preparation, verification_root.close())?;
    let _lock = lock;
    match prepared {
        PreparedBootstrap::Install(prepared) => {
            let PreparedInstall {
                mut runner,
                location,
                pin,
                source,
            } = *prepared;
            stage_generation_from_markdown(&mut runner, &location, request, &pin, &source)
        }
        PreparedBootstrap::Unchanged(result) => Ok(result),
        PreparedBootstrap::Reinstall(prepared) => {
            let PreparedReinstall {
                mut runner,
                location,
                pin,
                source,
                existing,
                snapshot,
            } = *prepared;
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandOutput, CommandRunner, CommandSpec, RecordingCommandRunner};
    use crate::document::parse_document;
    use crate::shadow::{
        canonical_logical_export, canonical_operational_projection, initial_operational_metadata,
        operational_projection_digest, to_operational_beads_jsonl,
    };

    #[cfg(unix)]
    #[test]
    fn bootstrap_runner_binds_locator_source_binary_and_environment() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let expected_binary = root.path().join("expected-bd");
        let unregistered_repository = root.path().join("unregistered-repository");
        let unregistered_binary = root.path().join("unregistered/bd");
        let marker = root.path().join("unexpected-command-ran");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&unregistered_repository).unwrap();
        fs::create_dir_all(unregistered_binary.parent().unwrap()).unwrap();
        fs::write(&expected_binary, "expected binary").unwrap();
        fs::write(
            &unregistered_binary,
            format!("#!/bin/sh\nprintf unexpected > '{}'\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(&unregistered_binary, fs::Permissions::from_mode(0o755)).unwrap();
        let mut runner = BootstrapCommandRunner::new(source, expected_binary);

        let result = runner.run(CommandSpec {
            program: unregistered_binary,
            argv: vec!["--sandbox".into(), "export".into()],
            cwd: Some(unregistered_repository),
            environment: BTreeMap::new(),
            redacted_argv_positions: Vec::new(),
        });

        assert!(
            result.is_err(),
            "an unregistered Beads-shaped plan must not dispatch"
        );
        assert!(
            !marker.exists(),
            "the runner must reject the plan before SystemCommandRunner starts it"
        );
    }

    #[cfg(unix)]
    #[test]
    fn bootstrap_runner_accepts_only_registered_staging_and_disposable_scopes() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let expected_binary = root.path().join("expected-bd");
        let fake_bin = root.path().join("fake-bin");
        let fake_git = fake_bin.join("git");
        let locator_checkout = root.path().join("unregistered-checkout");
        let disposable_root = root.path().join("unregistered-disposable");
        let disposable_repository = disposable_root.join("repository");
        let disposable_binary = disposable_root.join("bd");
        let locator_marker = root.path().join("unexpected-locator-ran");
        let disposable_marker = root.path().join("unexpected-disposable-ran");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&fake_bin).unwrap();
        fs::create_dir_all(&locator_checkout).unwrap();
        fs::create_dir_all(&disposable_repository).unwrap();
        fs::write(&expected_binary, "expected binary").unwrap();
        fs::write(
            &fake_git,
            format!(
                "#!/bin/sh\nprintf locator > '{}'\n",
                locator_marker.display()
            ),
        )
        .unwrap();
        fs::write(
            &disposable_binary,
            format!(
                "#!/bin/sh\nprintf disposable > '{}'\n",
                disposable_marker.display()
            ),
        )
        .unwrap();
        for path in [&fake_git, &disposable_binary] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let disposable_environment =
            environment_for_runtime(&disposable_root.join("runtime"), true).unwrap();
        let mut runner = BootstrapCommandRunner::new(source, expected_binary);

        let locator = runner.run(CommandSpec {
            program: PathBuf::from("git"),
            argv: vec!["rev-parse".into(), "--show-toplevel".into()],
            cwd: Some(locator_checkout),
            environment: BTreeMap::from([("PATH".into(), fake_bin.display().to_string())]),
            redacted_argv_positions: Vec::new(),
        });
        let disposable = runner.run(CommandSpec {
            program: disposable_binary,
            argv: vec![
                "--readonly".into(),
                "--sandbox".into(),
                "--json".into(),
                "vc".into(),
                "status".into(),
            ],
            cwd: Some(disposable_repository),
            environment: disposable_environment,
            redacted_argv_positions: Vec::new(),
        });

        assert!(
            locator.is_err(),
            "an arbitrary locator context must not dispatch"
        );
        assert!(
            disposable.is_err(),
            "an unregistered disposable root must not dispatch"
        );
        assert!(!locator_marker.exists());
        assert!(!disposable_marker.exists());
    }

    #[cfg(unix)]
    #[test]
    fn bootstrap_runner_binds_each_content_commit_to_its_discovered_literal_path() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let fake_bin = root.path().join("fake-bin");
        let fake_git = fake_bin.join("git");
        let calls = root.path().join("git-calls");
        let initial_binary = root.path().join("initial-bd");
        let resolved = "a".repeat(40);
        let first_commit = "b".repeat(40);
        let second_commit = "c".repeat(40);
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&fake_bin).unwrap();
        fs::write(&initial_binary, "initial binary").unwrap();
        fs::write(
            &fake_git,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$*\" in\n  \"rev-parse --verify --end-of-options origin/main^{{commit}}\") printf '{}\\n' ;;\n  \"ls-tree -r --name-only -z {} -- docs/intents docs/specs tasks\") printf 'docs/intents/001-one.md\\0docs/intents/002-two.md\\0' ;;\n  \"show {}:docs/intents/001-one.md\") printf selected-one ;;\n  \"log -1 --format=%H {} -- :(literal)docs/intents/001-one.md\") printf '{}\\n' ;;\n  \"show {}:docs/intents/001-one.md\") printf selected-one ;;\n  \"show {}:docs/intents/002-two.md\") printf selected-two ;;\n  \"log -1 --format=%H {} -- :(literal)docs/intents/002-two.md\") printf '{}\\n' ;;\n  \"show {}:docs/intents/002-two.md\") printf selected-two ;;\n  *) exit 77 ;;\nesac\n",
                calls.display(),
                resolved,
                resolved,
                resolved,
                resolved,
                first_commit,
                first_commit,
                resolved,
                resolved,
                second_commit,
                second_commit,
            ),
        )
        .unwrap();
        fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o755)).unwrap();
        let source = source.canonicalize().unwrap();
        let mut environment =
            environment_for_runtime(&root.path().join("verification-runtime"), true).unwrap();
        environment.insert("PATH".into(), fake_bin.display().to_string());
        let mut runner = BootstrapCommandRunner::new(source.clone(), initial_binary);
        runner
            .bind_source_inputs("origin/main".into(), environment.clone())
            .unwrap();
        let source_environment = runner.source_environment();
        let command = |argv: Vec<String>| CommandSpec {
            program: PathBuf::from("git"),
            argv,
            cwd: Some(source.clone()),
            environment: source_environment.clone(),
            redacted_argv_positions: Vec::new(),
        };
        let source_show =
            |commit: &str, path: &str| command(vec!["show".into(), format!("{commit}:{path}")]);
        let literal_log = |path: &str| {
            command(vec![
                "log".into(),
                "-1".into(),
                "--format=%H".into(),
                resolved.clone(),
                "--".into(),
                format!(":(literal){path}"),
            ])
        };

        runner
            .run(command(vec![
                "rev-parse".into(),
                "--verify".into(),
                "--end-of-options".into(),
                "origin/main^{commit}".into(),
            ]))
            .unwrap();
        runner
            .run(command(vec![
                "ls-tree".into(),
                "-r".into(),
                "--name-only".into(),
                "-z".into(),
                resolved.clone(),
                "--".into(),
                "docs/intents".into(),
                "docs/specs".into(),
                "tasks".into(),
            ]))
            .unwrap();

        for invalid in [
            source_show(&resolved, "tasks/999-undiscovered.md"),
            literal_log("docs/intents/001-one.md"),
            source_show(&first_commit, "docs/intents/001-one.md"),
        ] {
            let calls_before = fs::read_to_string(&calls).unwrap();
            assert!(
                runner.run(invalid).is_err(),
                "an undiscovered or wrong-phase source command must not dispatch"
            );
            assert_eq!(fs::read_to_string(&calls).unwrap(), calls_before);
        }

        runner
            .run(source_show(&resolved, "docs/intents/001-one.md"))
            .unwrap();
        runner.run(literal_log("docs/intents/001-one.md")).unwrap();
        for invalid in [
            source_show(&first_commit, "docs/intents/002-two.md"),
            source_show(&resolved, "docs/intents/001-one.md"),
            literal_log("docs/intents/001-one.md"),
        ] {
            let calls_before = fs::read_to_string(&calls).unwrap();
            assert!(
                runner.run(invalid).is_err(),
                "a content commit cannot authorize another path or a replay"
            );
            assert_eq!(fs::read_to_string(&calls).unwrap(), calls_before);
        }

        runner
            .run(source_show(&first_commit, "docs/intents/001-one.md"))
            .unwrap();
        let calls_before = fs::read_to_string(&calls).unwrap();
        assert!(
            runner
                .run(source_show(&first_commit, "docs/intents/001-one.md"))
                .is_err(),
            "an established content read cannot replay"
        );
        assert_eq!(fs::read_to_string(&calls).unwrap(), calls_before);

        runner
            .run(source_show(&resolved, "docs/intents/002-two.md"))
            .unwrap();
        runner.run(literal_log("docs/intents/002-two.md")).unwrap();
        runner
            .run(source_show(&second_commit, "docs/intents/002-two.md"))
            .unwrap();
        assert_eq!(
            fs::read_to_string(&calls).unwrap().lines().count(),
            8,
            "only the selected, literal-log, and matching establishing commands dispatch"
        );
    }

    #[cfg(unix)]
    #[test]
    fn malformed_discovery_preserves_loader_error_and_authorizes_no_paths() {
        use std::os::unix::fs::PermissionsExt;

        fn run_case(tree: &str, expected_code: &str, expected_key: &str) {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source");
            let fake_bin = root.path().join("fake-bin");
            let fake_git = fake_bin.join("git");
            let calls = root.path().join("git-calls");
            let initial_binary = root.path().join("initial-bd");
            let resolved = "a".repeat(40);
            fs::create_dir_all(&source).unwrap();
            fs::create_dir_all(&fake_bin).unwrap();
            fs::write(&initial_binary, "initial binary").unwrap();
            fs::write(
                &fake_git,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$*\" in\n  \"rev-parse --verify --end-of-options origin/main^{{commit}}\") printf '{}\\n' ;;\n  \"ls-tree -r --name-only -z {} -- docs/intents docs/specs tasks\") printf '{}' ;;\n  *) exit 77 ;;\nesac\n",
                    calls.display(),
                    resolved,
                    resolved,
                    tree,
                ),
            )
            .unwrap();
            fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o755)).unwrap();
            let source = source.canonicalize().unwrap();
            let mut environment =
                environment_for_runtime(&root.path().join("verification-runtime"), true).unwrap();
            environment.insert("PATH".into(), fake_bin.display().to_string());
            let mut runner = BootstrapCommandRunner::new(source.clone(), initial_binary);
            runner
                .bind_source_inputs("origin/main".into(), environment.clone())
                .unwrap();

            let error = load_documents(&mut runner, &source, &environment, "origin/main")
                .expect_err("the strict loader must retain its own malformed-tree refusal");
            assert_eq!(error.code(), expected_code);
            assert_eq!(error.offending_key.as_deref(), Some(expected_key));
            assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 2);

            let source_environment = runner.source_environment();
            let repeated_tree = CommandSpec {
                program: PathBuf::from("git"),
                argv: vec![
                    "ls-tree".into(),
                    "-r".into(),
                    "--name-only".into(),
                    "-z".into(),
                    resolved.clone(),
                    "--".into(),
                    "docs/intents".into(),
                    "docs/specs".into(),
                    "tasks".into(),
                ],
                cwd: Some(source.clone()),
                environment: source_environment.clone(),
                redacted_argv_positions: Vec::new(),
            };
            let show = CommandSpec {
                program: PathBuf::from("git"),
                argv: vec!["show".into(), format!("{resolved}:tasks/001-first.md")],
                cwd: Some(source),
                environment: source_environment,
                redacted_argv_positions: Vec::new(),
            };
            let calls_before = fs::read_to_string(&calls).unwrap();
            assert_eq!(
                runner.run(repeated_tree).unwrap_err(),
                "invalid_bootstrap_command"
            );
            assert_eq!(runner.run(show).unwrap_err(), "invalid_bootstrap_command");
            assert_eq!(fs::read_to_string(&calls).unwrap(), calls_before);
        }

        run_case("tasks/001.md\\0", "invalid_document", "task:001");
        run_case(
            "tasks/001-first.md\\0tasks/001-second.md\\0",
            "duplicate_document_id",
            "task:001",
        );
    }

    #[test]
    fn registered_staging_scope_does_not_admit_source_git_forms() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let common = root.path().join("common");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&common).unwrap();
        let source = source.canonicalize().unwrap();
        let common = common.canonicalize().unwrap();
        let expected_binary = root.path().join("expected-bd");
        fs::write(&expected_binary, "expected binary").unwrap();
        let verification_environment =
            environment_for_runtime(&root.path().join("verification-runtime"), true).unwrap();
        let state_root = common.join(STORE_DIRECTORY);
        let generations_dir = state_root.join(GENERATIONS_DIRECTORY);
        fs::create_dir_all(&generations_dir).unwrap();
        let mut runner = BootstrapCommandRunner::new(source.clone(), expected_binary);
        runner
            .bind_source_inputs("origin/main".into(), verification_environment)
            .unwrap();
        runner.bind_checkout(source.clone()).unwrap();
        runner
            .bind_location(StoreLocation {
                worktree_root: source.clone(),
                common_dir: common,
                state_root,
                generations_dir: generations_dir.clone(),
            })
            .unwrap();
        let staging = generations_dir.join(".staging-safe");
        fs::create_dir(&staging).unwrap();
        let staging_environment = environment_for_runtime(&staging.join("runtime"), true).unwrap();
        runner
            .register_staging(&staging, &staging_environment)
            .unwrap();
        runner.resolved_source_commit = Some("a".repeat(40));
        let attempted_source_read = CommandSpec {
            program: PathBuf::from("git"),
            argv: vec![
                "show".into(),
                format!("{}:tasks/046-task.md", "a".repeat(40)),
            ],
            cwd: Some(source),
            environment: staging_environment.clone(),
            redacted_argv_positions: Vec::new(),
        };
        let mismatched_authority_commit = CommandSpec {
            program: staging.join("bd"),
            argv: vec![
                "--sandbox".into(),
                "kv".into(),
                "set".into(),
                "plasmosome.source-commit".into(),
                "b".repeat(40),
            ],
            cwd: Some(staging.join("repository")),
            environment: staging_environment.clone(),
            redacted_argv_positions: Vec::new(),
        };
        let established_authority_commit = CommandSpec {
            argv: {
                let mut argv = mismatched_authority_commit.argv.clone();
                argv[4] = "a".repeat(40);
                argv
            },
            ..mismatched_authority_commit.clone()
        };

        assert!(
            !runner.valid_staging_command(&attempted_source_read),
            "a staging environment may authorize only its exact repository forms"
        );
        assert!(
            !runner.valid_staging_command(&mismatched_authority_commit),
            "the imported authority marker must be bound to the resolved source commit"
        );
        assert!(
            runner.valid_staging_command(&established_authority_commit),
            "the exact resolved source commit remains an allowed authority marker"
        );
    }

    #[test]
    fn bootstrap_runner_registers_a_native_tempdir_disposable_root() {
        let source = tempfile::tempdir().unwrap();
        let selected_binary = source.path().join("selected-bd");
        fs::write(&selected_binary, "selected binary").unwrap();
        let disposable = tempfile::tempdir().unwrap();
        let environment = environment_for_runtime(&disposable.path().join("runtime"), true)
            .expect("a newly created disposable runtime is valid");
        let mut runner =
            BootstrapCommandRunner::new(source.path().canonicalize().unwrap(), selected_binary);

        runner
            .register_disposable(disposable.path(), &environment)
            .expect("a native TempDir spelling must be accepted for bootstrap reads");
    }

    #[test]
    fn ordinary_version_checks_are_bound_before_dispatch() {
        let root = tempfile::tempdir().unwrap();
        let installed_binary = root.path().join("generation/bd");
        let copied_binary = root.path().join("disposable/bd");
        fs::create_dir_all(installed_binary.parent().unwrap()).unwrap();
        fs::create_dir_all(copied_binary.parent().unwrap()).unwrap();
        let installed_environment =
            environment_for_runtime(&root.path().join("generation/runtime"), true).unwrap();
        let copied_environment =
            environment_for_runtime(&root.path().join("disposable/runtime"), true).unwrap();
        let installed = CommandSpec {
            program: installed_binary.clone(),
            argv: vec!["--version".into()],
            cwd: None,
            environment: installed_environment.clone(),
            redacted_argv_positions: Vec::new(),
        };
        let copied = CommandSpec {
            program: copied_binary.clone(),
            argv: vec!["--version".into()],
            cwd: None,
            environment: copied_environment.clone(),
            redacted_argv_positions: Vec::new(),
        };
        let mut inner = RecordingCommandRunner::scripted(vec![
            Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
            Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
        ]);
        {
            let mut runner = ReadVersionRunner::new(
                &mut inner,
                &installed_binary,
                installed_environment.clone(),
                &copied_binary,
                copied_environment.clone(),
            );
            runner.run(installed.clone()).unwrap();
            runner.run(copied.clone()).unwrap();
        }
        assert_eq!(inner.commands(), &[installed.clone(), copied.clone()]);
        assert!(inner.finish().is_ok());

        for invalid in [
            CommandSpec {
                program: root.path().join("unbound-bd"),
                ..installed.clone()
            },
            CommandSpec {
                argv: vec!["--readonly".into(), "--version".into()],
                ..installed.clone()
            },
            CommandSpec {
                cwd: Some(root.path().to_path_buf()),
                ..installed.clone()
            },
            CommandSpec {
                environment: BTreeMap::new(),
                ..installed.clone()
            },
            CommandSpec {
                redacted_argv_positions: vec![0],
                ..installed
            },
        ] {
            let mut sentinel = RecordingCommandRunner::default();
            let mut runner = ReadVersionRunner::new(
                &mut sentinel,
                &installed_binary,
                installed_environment.clone(),
                &copied_binary,
                copied_environment.clone(),
            );
            assert_eq!(runner.run(invalid).unwrap_err(), "invalid_read_command");
            assert!(
                sentinel.commands().is_empty(),
                "an invalid version plan must be refused before dispatch"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn runtime_repair_does_not_depend_on_the_installed_runtime_tree() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        fn shell_quote(value: &str) -> String {
            format!("'{}'", value.replace('\'', "'\"'\"'"))
        }

        let root = tempfile::tempdir().unwrap();
        let source_root = root.path().join("source");
        fs::create_dir(&source_root).unwrap();
        let source_root = source_root.canonicalize().unwrap();
        let source_commit = "a".repeat(40);
        let local_generation = "d".repeat(40);
        let documents = vec![
            parse_document(
                "docs/intents/001-intent.md",
                "---\nid: 001\ntitle: Intent\nstatus: approved\n---\n",
                &source_commit,
            )
            .unwrap(),
        ];
        let operational = initial_operational_metadata(&documents).unwrap();
        let export = to_operational_beads_jsonl(&documents, &operational).unwrap();
        let operational_documents = decode_operational_beads_jsonl(&export).unwrap();
        let logical_export_sha256 =
            logical_export_digest(&canonical_logical_export(&documents).unwrap());
        let operational_projection_sha256 = operational_projection_digest(
            &canonical_operational_projection(&operational_documents).unwrap(),
        );
        let status = serde_json::json!({
            "schema_version": 1,
            "branch": "main",
            "commit": local_generation,
        })
        .to_string();
        let keys = serde_json::json!({
            "schema_version": 1,
            "plasmosome.authority-mode": "markdown-shadow",
            "plasmosome.source-commit": source_commit,
        })
        .to_string();
        let binary = root.path().join("requested-bd");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\ncase \"$1:$2:$3:$4:$5\" in\n--version::::) printf 'bd version 1.1.2 (test)\\n' ;;\n--readonly:--sandbox:export::) printf '%s' {} ;;\n--readonly:--sandbox:--json:kv:list) printf '%s' {} ;;\n--readonly:--sandbox:--json:vc:status) printf '%s' {} ;;\n*) exit 47 ;;\nesac\n",
                shell_quote(&export),
                shell_quote(&keys),
                shell_quote(&status),
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let pin = PinManifest::parse(&format!(
            "version = \"1.1.2\"\nrelease = \"https://example.invalid/release\"\nsource_commit = \"{}\"\nlicense = \"MIT\"\nchecksums_url = \"https://example.invalid/checksums\"\nchecksums_sha256 = \"{}\"\n\n[[targets]]\ntarget = \"aarch64-apple-darwin\"\narchive = \"beads_1.1.2_darwin_arm64.tar.gz\"\narchive_sha256 = \"{}\"\nbinary_sha256 = \"{}\"\n",
            "b".repeat(40),
            "c".repeat(64),
            "e".repeat(64),
            sha256(&binary).unwrap(),
        ))
        .unwrap();

        for runtime_shape in ["absent", "malformed", "symlinked"] {
            let generation = CurrentGeneration {
                name: format!("generation-{runtime_shape}"),
                root: root.path().join(format!("generation-{runtime_shape}")),
                manifest: StateManifest {
                    schema_version: 1,
                    authority_mode: "markdown-shadow".into(),
                    source_commit: source_commit.clone(),
                    logical_export_sha256: logical_export_sha256.clone(),
                    operational_projection_sha256: operational_projection_sha256.clone(),
                    local_generation: local_generation.clone(),
                    host_target: "retired-target".into(),
                    wrapper_sha256: "f".repeat(64),
                    beads_binary_sha256: "0".repeat(64),
                    remote_relation: RemoteRelation::Unknown,
                    remote_generation: None,
                    remote_observed_at: None,
                    observed_local_generation: None,
                    last_successful_sync_at: None,
                    pending_operation_ids: Vec::new(),
                },
            };
            fs::create_dir_all(generation.root.join("repository")).unwrap();
            match runtime_shape {
                "absent" => {}
                "malformed" => fs::write(generation.root.join("runtime"), "not a runtime").unwrap(),
                "symlinked" => {
                    let target = root.path().join("runtime-target");
                    fs::create_dir_all(&target).unwrap();
                    symlink(target, generation.root.join("runtime")).unwrap();
                }
                _ => unreachable!(),
            }
            let verification_environment = environment_for_runtime(
                &root
                    .path()
                    .join(format!("verification-runtime-{runtime_shape}")),
                true,
            )
            .unwrap();
            assert_eq!(
                read_disposable_snapshot_with_binary(
                    &mut RecordingCommandRunner::default(),
                    &generation,
                    &pin,
                    "aarch64-apple-darwin",
                    &binary,
                    false,
                )
                .expect_err("ordinary reads must keep rejecting an unhealthy installed runtime")
                .code(),
                "invalid_store"
            );
            let mut runner = BootstrapCommandRunner::new(source_root.clone(), binary.clone());
            runner
                .bind_source_inputs("origin/main".into(), verification_environment.clone())
                .unwrap();
            let snapshot = bootstrap_read_disposable_snapshot_with_binary(
                &mut runner,
                &generation,
                &pin,
                "aarch64-apple-darwin",
                &binary,
                verification_environment,
                false,
            )
            .expect("runtime repair must read only the old regular repository with the fresh verified binary");
            assert_eq!(snapshot.documents.len(), 1, "failed {runtime_shape} case");
        }
    }

    #[test]
    fn requested_wrapper_hash_must_match_the_active_generation() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("installed-wrapper");
        let requested = root.path().join("requested-wrapper");
        fs::write(&installed, "installed wrapper").unwrap();
        fs::write(&requested, "requested wrapper").unwrap();
        let generation = CurrentGeneration {
            name: "generation-safe".into(),
            root: root.path().to_path_buf(),
            manifest: StateManifest {
                schema_version: 1,
                authority_mode: "markdown-shadow".into(),
                source_commit: "a".repeat(40),
                logical_export_sha256: "b".repeat(64),
                operational_projection_sha256: "c".repeat(64),
                local_generation: "local-generation".into(),
                host_target: "aarch64-apple-darwin".into(),
                wrapper_sha256: sha256(&installed).unwrap(),
                beads_binary_sha256: "d".repeat(64),
                remote_relation: RemoteRelation::Unknown,
                remote_generation: None,
                remote_observed_at: None,
                observed_local_generation: None,
                last_successful_sync_at: None,
                pending_operation_ids: Vec::new(),
            },
        };
        fs::rename(&installed, generation.root.join("plasmosome-work-state")).unwrap();
        owner_private_executable(&generation.root.join("plasmosome-work-state")).unwrap();

        assert!(wrapper_is_valid(&generation).is_ok());
        assert_eq!(
            wrapper_matches_requested(&generation, &requested)
                .expect_err("a different currently running wrapper must require reinstallation")
                .code(),
            "invalid_store"
        );
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_requires_owner_private_executable_mode() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let wrapper = root.path().join("plasmosome-work-state");
        fs::write(&wrapper, "verified wrapper").unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o600)).unwrap();
        let generation = CurrentGeneration {
            name: "generation-safe".into(),
            root: root.path().to_path_buf(),
            manifest: StateManifest {
                schema_version: 1,
                authority_mode: "markdown-shadow".into(),
                source_commit: "a".repeat(40),
                logical_export_sha256: "b".repeat(64),
                operational_projection_sha256: "c".repeat(64),
                local_generation: "local-generation".into(),
                host_target: "aarch64-apple-darwin".into(),
                wrapper_sha256: sha256(&wrapper).unwrap(),
                beads_binary_sha256: "d".repeat(64),
                remote_relation: RemoteRelation::Unknown,
                remote_generation: None,
                remote_observed_at: None,
                observed_local_generation: None,
                last_successful_sync_at: None,
                pending_operation_ids: Vec::new(),
            },
        };

        assert_eq!(
            wrapper_is_valid(&generation)
                .expect_err("a launcher-inexecutable installed wrapper must trigger repair")
                .code(),
            "invalid_store"
        );
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(wrapper_is_valid(&generation).is_ok());
    }

    #[test]
    fn bootstrap_reinstalls_runtime_without_reimporting_state() {
        let root = tempfile::tempdir().unwrap();
        let source_commit = "a".repeat(40);
        let local_generation = "d".repeat(40);
        let documents = vec![
            parse_document(
                "docs/intents/001-intent.md",
                "---\nid: 001\ntitle: Intent\nstatus: approved\n---\n",
                &source_commit,
            )
            .unwrap(),
        ];
        let operational = initial_operational_metadata(&documents).unwrap();
        let export = to_operational_beads_jsonl(&documents, &operational).unwrap();
        let operational_documents = decode_operational_beads_jsonl(&export).unwrap();
        let binary = root.path().join("requested-bd");
        fs::write(&binary, "requested verified binary").unwrap();
        let pin = PinManifest::parse(&format!(
            "version = \"1.1.2\"\nrelease = \"https://example.invalid/release\"\nsource_commit = \"{}\"\nlicense = \"MIT\"\nchecksums_url = \"https://example.invalid/checksums\"\nchecksums_sha256 = \"{}\"\n\n[[targets]]\ntarget = \"aarch64-apple-darwin\"\narchive = \"beads_1.1.2_darwin_arm64.tar.gz\"\narchive_sha256 = \"{}\"\nbinary_sha256 = \"{}\"\n",
            "b".repeat(40),
            "c".repeat(64),
            "e".repeat(64),
            sha256(&binary).unwrap(),
        ))
        .unwrap();
        let generation = CurrentGeneration {
            name: "generation-old".into(),
            root: root.path().join("generation-old"),
            manifest: StateManifest {
                schema_version: 1,
                authority_mode: "markdown-shadow".into(),
                source_commit: source_commit.clone(),
                logical_export_sha256: logical_export_digest(
                    &canonical_logical_export(&documents).unwrap(),
                ),
                operational_projection_sha256: operational_projection_digest(
                    &canonical_operational_projection(&operational_documents).unwrap(),
                ),
                local_generation: local_generation.clone(),
                host_target: "retired-target".into(),
                wrapper_sha256: "f".repeat(64),
                beads_binary_sha256: "0".repeat(64),
                remote_relation: RemoteRelation::Unknown,
                remote_generation: None,
                remote_observed_at: None,
                observed_local_generation: None,
                last_successful_sync_at: None,
                pending_operation_ids: Vec::new(),
            },
        };
        fs::create_dir_all(generation.root.join("repository")).unwrap();
        environment_for_runtime(&generation.root.join("runtime"), true).unwrap();
        let status = serde_json::json!({
            "schema_version": 1,
            "branch": "main",
            "commit": local_generation,
        })
        .to_string();
        let keys = serde_json::json!({
            "schema_version": 1,
            "plasmosome.authority-mode": "markdown-shadow",
            "plasmosome.source-commit": source_commit,
        })
        .to_string();

        assert_eq!(
            read_disposable_snapshot_with_binary(
                &mut RecordingCommandRunner::default(),
                &generation,
                &pin,
                "aarch64-apple-darwin",
                &binary,
                true,
            )
            .expect_err("ordinary reads must still reject mismatched installed runtime metadata")
            .code(),
            "invalid_store"
        );
        let mut runner = RecordingCommandRunner::scripted(vec![
            Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
            Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
            Ok(CommandOutput::success(status.clone())),
            Ok(CommandOutput::success(export)),
            Ok(CommandOutput::success(keys)),
            Ok(CommandOutput::success(status)),
        ]);
        let snapshot = read_disposable_snapshot_with_binary(
            &mut runner,
            &generation,
            &pin,
            "aarch64-apple-darwin",
            &binary,
            false,
        )
        .expect(
            "bootstrap recovery must validate the old state through the requested verified runtime",
        );
        assert_eq!(snapshot.documents.len(), 1);
        assert!(runner.finish().is_ok());
    }

    #[test]
    fn temporary_cleanup_failure_takes_precedence_over_read_failure() {
        let result = finish_disposable_snapshot(
            Err(refusal("invalid_store")),
            Err(std::io::Error::other("unable to remove disposable root")),
        );

        assert_eq!(
            result
                .expect_err("cleanup failure must override the failed disposable read")
                .code(),
            "temporary_cleanup_failed"
        );
    }

    #[test]
    fn bootstrap_cleanup_failure_is_not_a_repairable_runtime_failure() {
        let result = finish_disposable_snapshot(
            Err(refusal("invalid_store")),
            Err(std::io::Error::other("unable to remove disposable root")),
        );

        assert_eq!(
            result
                .expect_err("bootstrap must stop rather than attempting runtime repair")
                .code(),
            "temporary_cleanup_failed"
        );
    }

    #[test]
    fn bootstrap_verification_cleanup_failure_takes_precedence() {
        let result = finish_verification_cleanup(
            Err::<(), _>(refusal("source_ref_unavailable")),
            Err(std::io::Error::other("unable to remove verification root")),
        );

        assert_eq!(
            result
                .expect_err("verification cleanup must override preparation refusal")
                .code(),
            "temporary_cleanup_failed"
        );
    }

    #[test]
    fn bootstrap_verification_cleanup_failure_precedes_activation() {
        let root = tempfile::tempdir().unwrap();
        let activation = root.path().join("activation");
        let result = (|| -> Result<(), StoreError> {
            finish_verification_cleanup(
                Ok(()),
                Err(std::io::Error::other("unable to remove verification root")),
            )?;
            fs::write(&activation, "activated").map_err(|_| refusal("invalid_store"))?;
            Ok(())
        })();

        assert_eq!(
            result
                .expect_err("verification cleanup must block activation")
                .code(),
            "temporary_cleanup_failed"
        );
        assert!(
            !activation.exists(),
            "the activation continuation must not run after verification cleanup failure"
        );
    }

    #[cfg(unix)]
    fn installed_runtime_preflight_fixture() -> (
        tempfile::TempDir,
        BootstrapCommandRunner,
        CurrentGeneration,
        PinManifest,
        PathBuf,
    ) {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let checkout = root.path().join("checkout");
        let common = root.path().join("common");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(&common).unwrap();
        let source = source.canonicalize().unwrap();
        let checkout = checkout.canonicalize().unwrap();
        let common = common.canonicalize().unwrap();
        let location = StoreLocation {
            worktree_root: checkout.clone(),
            common_dir: common.clone(),
            state_root: common.join(STORE_DIRECTORY),
            generations_dir: common.join(STORE_DIRECTORY).join(GENERATIONS_DIRECTORY),
        };
        let generation_root = location.generations_dir.join("generation-safe");
        fs::create_dir_all(&generation_root).unwrap();
        let wrapper = generation_root.join("plasmosome-work-state");
        let requested_wrapper = root.path().join("requested-wrapper");
        fs::write(&wrapper, "installed wrapper").unwrap();
        fs::write(&requested_wrapper, "installed wrapper").unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&requested_wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        let binary = generation_root.join("bd");
        fs::write(
            &binary,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'bd version 1.1.2 (test)\\n'; exit 0; fi\nexit 99\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        environment_for_runtime(&generation_root.join("runtime"), true).unwrap();
        let target = "aarch64-apple-darwin";
        let pin = PinManifest::parse(&format!(
            "version = \"1.1.2\"\nrelease = \"https://example.invalid/release\"\nsource_commit = \"{}\"\nlicense = \"MIT\"\nchecksums_url = \"https://example.invalid/checksums\"\nchecksums_sha256 = \"{}\"\n\n[[targets]]\ntarget = \"{target}\"\narchive = \"beads_1.1.2_test.tar.gz\"\narchive_sha256 = \"{}\"\nbinary_sha256 = \"{}\"\n",
            "a".repeat(40),
            "b".repeat(64),
            "c".repeat(64),
            sha256(&binary).unwrap(),
        ))
        .unwrap();
        let generation = CurrentGeneration {
            name: "generation-safe".into(),
            root: generation_root,
            manifest: StateManifest {
                schema_version: 1,
                authority_mode: "markdown-shadow".into(),
                source_commit: "a".repeat(40),
                logical_export_sha256: "b".repeat(64),
                operational_projection_sha256: "c".repeat(64),
                local_generation: "d".repeat(40),
                host_target: target.into(),
                wrapper_sha256: sha256(&wrapper).unwrap(),
                beads_binary_sha256: sha256(&binary).unwrap(),
                remote_relation: RemoteRelation::Unknown,
                remote_generation: None,
                remote_observed_at: None,
                observed_local_generation: None,
                last_successful_sync_at: None,
                pending_operation_ids: Vec::new(),
            },
        };
        let selected_binary = root.path().join("selected-bd");
        fs::write(&selected_binary, "selected binary").unwrap();
        let verification_environment =
            environment_for_runtime(&root.path().join("verification-runtime"), true).unwrap();
        let mut runner = BootstrapCommandRunner::new(source, selected_binary);
        runner
            .bind_source_inputs("origin/main".into(), verification_environment)
            .unwrap();
        runner.bind_checkout(checkout).unwrap();
        runner.bind_location(location).unwrap();
        (root, runner, generation, pin, requested_wrapper)
    }

    #[cfg(unix)]
    #[test]
    fn installed_runtime_damage_is_the_only_repair_disposition() {
        use std::os::unix::fs::PermissionsExt;

        let (_root, mut runner, generation, pin, requested_wrapper) =
            installed_runtime_preflight_fixture();
        assert_eq!(
            sha256(&generation.root.join("plasmosome-work-state")).unwrap(),
            generation.manifest.wrapper_sha256
        );
        assert_eq!(
            sha256(&requested_wrapper).unwrap(),
            generation.manifest.wrapper_sha256
        );
        assert!(wrapper_matches_requested(&generation, &requested_wrapper).is_ok());
        let environment = environment_for_runtime(&generation.root.join("runtime"), false).unwrap();
        runner
            .register_installed_generation(&generation, &environment)
            .unwrap();
        assert!(
            InstalledBeads::verify(
                &pin,
                "aarch64-apple-darwin",
                &generation.root.join("bd"),
                environment,
                &mut runner,
            )
            .is_ok()
        );
        let preflight = installed_runtime_preflight(
            &mut runner,
            &generation,
            &pin,
            "aarch64-apple-darwin",
            &requested_wrapper,
        );
        assert!(
            matches!(preflight, Ok(InstalledRuntimePreflight::Usable { .. })),
            "{preflight:?}"
        );

        let (_root, mut runner, generation, pin, requested_wrapper) =
            installed_runtime_preflight_fixture();
        fs::rename(
            generation.root.join("bd"),
            generation.root.join("bd-removed"),
        )
        .unwrap();
        assert!(matches!(
            installed_runtime_preflight(
                &mut runner,
                &generation,
                &pin,
                "aarch64-apple-darwin",
                &requested_wrapper,
            ),
            Ok(InstalledRuntimePreflight::RepairRequired)
        ));

        let (_root, mut runner, generation, pin, requested_wrapper) =
            installed_runtime_preflight_fixture();
        fs::set_permissions(
            generation.root.join("plasmosome-work-state"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert!(matches!(
            installed_runtime_preflight(
                &mut runner,
                &generation,
                &pin,
                "aarch64-apple-darwin",
                &requested_wrapper,
            ),
            Ok(InstalledRuntimePreflight::RepairRequired)
        ));

        let (_root, mut runner, generation, pin, requested_wrapper) =
            installed_runtime_preflight_fixture();
        fs::set_permissions(
            generation.root.join("bd"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert!(matches!(
            installed_runtime_preflight(
                &mut runner,
                &generation,
                &pin,
                "aarch64-apple-darwin",
                &requested_wrapper,
            ),
            Ok(InstalledRuntimePreflight::RepairRequired)
        ));

        let (_root, mut runner, generation, pin, requested_wrapper) =
            installed_runtime_preflight_fixture();
        fs::set_permissions(
            generation.root.join("bd"),
            fs::Permissions::from_mode(0o777),
        )
        .unwrap();
        assert!(matches!(
            installed_runtime_preflight(
                &mut runner,
                &generation,
                &pin,
                "aarch64-apple-darwin",
                &requested_wrapper,
            ),
            Ok(InstalledRuntimePreflight::RepairRequired)
        ));

        let (_root, mut runner, generation, pin, requested_wrapper) =
            installed_runtime_preflight_fixture();
        fs::rename(
            generation.root.join("runtime"),
            generation.root.join("runtime-removed"),
        )
        .unwrap();
        assert!(matches!(
            installed_runtime_preflight(
                &mut runner,
                &generation,
                &pin,
                "aarch64-apple-darwin",
                &requested_wrapper,
            ),
            Ok(InstalledRuntimePreflight::RepairRequired)
        ));

        let (_root, mut runner, mut generation, pin, requested_wrapper) =
            installed_runtime_preflight_fixture();
        generation.manifest.host_target = "x86_64-unknown-linux-gnu".into();
        assert!(matches!(
            installed_runtime_preflight(
                &mut runner,
                &generation,
                &pin,
                "aarch64-apple-darwin",
                &requested_wrapper,
            ),
            Ok(InstalledRuntimePreflight::RepairRequired)
        ));

        let (_root, mut runner, mut generation, pin, requested_wrapper) =
            installed_runtime_preflight_fixture();
        generation.manifest.beads_binary_sha256 = "0".repeat(64);
        assert!(matches!(
            installed_runtime_preflight(
                &mut runner,
                &generation,
                &pin,
                "aarch64-apple-darwin",
                &requested_wrapper,
            ),
            Ok(InstalledRuntimePreflight::RepairRequired)
        ));

        let (root, mut runner, mut generation, pin, requested_wrapper) =
            installed_runtime_preflight_fixture();
        let outside = root.path().join("outside-generation");
        copy_private_tree(&generation.root, &outside).unwrap();
        generation.root = outside;
        assert_eq!(
            installed_runtime_preflight(
                &mut runner,
                &generation,
                &pin,
                "aarch64-apple-darwin",
                &requested_wrapper,
            )
            .expect_err("an unbound installed root is a fatal policy error")
            .code(),
            "invalid_store"
        );
    }
}
