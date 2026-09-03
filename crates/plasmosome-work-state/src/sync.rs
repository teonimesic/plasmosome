use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::command::{CommandOutput, CommandRunner, CommandSpec};
use crate::document::is_lower_hex_sha;
use crate::freshness::{
    Freshness, FreshnessEnvelope, ObservationState, PendingMutations, classify,
};
use crate::project::{ProjectConfig, compiled_project_config};
use crate::store::{
    CurrentGeneration, FailedSyncObservation, FencedSnapshot, GenerationActivationLock, StoreError,
    StoreLocation, prepare_sync_staging, read_disposable_snapshot,
};

/// A stable refusal raised while validating the online-sync command sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncError {
    code: &'static str,
    state_changed: bool,
}

impl SyncError {
    /// Returns the stable machine-readable refusal code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns whether this refusal atomically activated a justified metadata-only generation.
    pub fn state_changed(&self) -> bool {
        self.state_changed
    }

    fn with_state_changed(mut self, state_changed: bool) -> Self {
        self.state_changed = state_changed;
        self
    }
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for SyncError {}

fn refusal(code: &'static str) -> SyncError {
    SyncError {
        code,
        state_changed: false,
    }
}

/// The complete successful response emitted by the explicit installed-wrapper sync command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SyncResult {
    /// The fixed public command name.
    pub command: String,
    /// The fixed checksum-bound project identifier.
    pub project_id: String,
    /// The fixed successful outcome.
    pub outcome: String,
    /// The installed Beads authority mode.
    pub authority_mode: String,
    /// The Markdown source commit retained from the active generation.
    pub source_commit: String,
    /// Whether a complete compatible generation was atomically activated.
    pub state_changed: bool,
    /// The complete persisted freshness envelope.
    pub freshness: FreshnessEnvelope,
}

impl SyncResult {
    /// Creates the only success outcome Task 047 may expose.
    pub fn synchronized(
        source_commit: String,
        freshness: FreshnessEnvelope,
        state_changed: bool,
    ) -> Self {
        Self {
            command: "sync".into(),
            project_id: "plasmosome".into(),
            outcome: "synchronized".into(),
            authority_mode: "markdown-shadow".into(),
            source_commit,
            state_changed,
            freshness,
        }
    }
}

fn displayed_option(value: Option<&str>) -> &str {
    value.unwrap_or("none")
}

fn freshness_name(freshness: &Freshness) -> &'static str {
    match freshness {
        Freshness::SynchronizedAsOf => "synchronized_as_of",
        Freshness::Stale => "stale",
        Freshness::Unknown => "unknown",
        Freshness::Unpublished => "unpublished",
        Freshness::StaleWithUnpublished => "stale_with_unpublished",
        Freshness::UnknownWithUnpublished => "unknown_with_unpublished",
    }
}

/// Renders a successful sync without representing its state as current or up to date.
pub fn render_sync_human(result: &SyncResult) -> String {
    let freshness = &result.freshness;
    let freshness_line = match freshness.freshness {
        Freshness::SynchronizedAsOf => format!(
            "freshness: synchronized as of {}",
            displayed_option(freshness.last_successful_sync_at.as_deref())
        ),
        _ => format!("freshness: {}", freshness_name(&freshness.freshness)),
    };
    [
        format!(
            "sync: synchronized as of {}",
            displayed_option(freshness.last_successful_sync_at.as_deref())
        ),
        format!("project: {}", result.project_id),
        format!("authority mode: {}", result.authority_mode),
        format!("source commit: {}", result.source_commit),
        format!("state changed: {}", result.state_changed),
        format!(
            "last successful sync at: {}",
            displayed_option(freshness.last_successful_sync_at.as_deref())
        ),
        format!("local generation: {}", freshness.local_generation),
        format!(
            "remote generation: {}",
            displayed_option(freshness.remote_generation.as_deref())
        ),
        format!(
            "remote observed at: {}",
            displayed_option(freshness.remote_observed_at.as_deref())
        ),
        format!(
            "pending mutations: {} [{}]",
            freshness.pending_mutations.count,
            freshness.pending_mutations.operation_ids.join(", ")
        ),
        freshness_line,
    ]
    .join("\n")
        + "\n"
}

/// The immutable paths and sealed environment bound to one staged synchronization attempt.
#[derive(Clone, Debug)]
pub struct SyncCommandBinding {
    project: ProjectConfig,
    staging_root: PathBuf,
    repository: PathBuf,
    binary: PathBuf,
    environment: BTreeMap<String, String>,
}

impl SyncCommandBinding {
    /// Binds the one allowed staging root, repository, binary, and cleared environment.
    pub fn new(
        project: ProjectConfig,
        staging_root: PathBuf,
        repository: PathBuf,
        binary: PathBuf,
        environment: BTreeMap<String, String>,
    ) -> Result<Self, SyncError> {
        if !staging_root.is_absolute()
            || repository != staging_root.join("repository")
            || binary != staging_root.join("bd")
            || environment.is_empty()
            || environment.get("BEADS_DIR")
                != Some(&repository.join(".beads").display().to_string())
            || environment.get("BD_BACKUP_ENABLED") != Some(&"false".to_owned())
        {
            return Err(refusal("invalid_sync_command"));
        }
        Ok(Self {
            project,
            staging_root,
            repository,
            binary,
            environment,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    AwaitFirstObservation,
    AwaitCloneDecision,
    AwaitInit,
    AwaitRemoteList,
    AwaitSecondObservation,
    AwaitStableDecision,
    AwaitCandidateVersion,
    AwaitMetadataVersion,
    AwaitReadonlyBeforeStatus,
    AwaitReadonlyExport,
    AwaitReadonlyKeyValues,
    AwaitReadonlyAfterStatus,
    Complete,
    Terminal,
}

/// The terminal interpretation of one strict Git observation command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteObservation {
    /// One exact lowercase data-ref commit was observed.
    Found(String),
    /// Git reported the requested ref did not exist.
    NoMatch,
    /// Git failed before a trustworthy remote observation was available.
    Transport,
}

/// An ordered fence that admits only Task 047's remote observation and fresh-clone commands.
pub struct SyncCommandRunner<'a, R> {
    inner: &'a mut R,
    binding: SyncCommandBinding,
    phase: Phase,
    first: Option<RemoteObservation>,
    second: Option<RemoteObservation>,
}

impl<'a, R> SyncCommandRunner<'a, R> {
    /// Creates one command runner bound to one newly staged synchronization attempt.
    pub fn new(inner: &'a mut R, binding: SyncCommandBinding) -> Self {
        Self {
            inner,
            binding,
            phase: Phase::AwaitFirstObservation,
            first: None,
            second: None,
        }
    }

    fn new_for_failed_observation(inner: &'a mut R, binding: SyncCommandBinding) -> Self {
        Self {
            inner,
            binding,
            phase: Phase::AwaitMetadataVersion,
            first: None,
            second: None,
        }
    }

    /// Returns the exact first remote generation if a valid R0 observation completed.
    pub fn first_observation(&self) -> Option<String> {
        match &self.first {
            Some(RemoteObservation::Found(value)) => Some(value.clone()),
            Some(RemoteObservation::NoMatch | RemoteObservation::Transport) | None => None,
        }
    }

    /// Returns the exact second remote generation if a valid R1 observation completed.
    pub fn second_observation(&self) -> Option<String> {
        match &self.second {
            Some(RemoteObservation::Found(value)) => Some(value.clone()),
            Some(RemoteObservation::NoMatch | RemoteObservation::Transport) | None => None,
        }
    }

    /// Returns the recorded first observation classification without inspecting command stderr.
    pub fn first_outcome(&self) -> Option<&RemoteObservation> {
        self.first.as_ref()
    }

    /// Returns the recorded second observation classification without inspecting command stderr.
    pub fn second_outcome(&self) -> Option<&RemoteObservation> {
        self.second.as_ref()
    }

    /// Requires the two completed observations to name the same exact remote generation.
    pub fn require_stable_observation(&mut self) -> Result<String, SyncError> {
        if self.phase != Phase::AwaitStableDecision {
            return Err(refusal("invalid_sync_command"));
        }
        let result = match (self.first.as_ref(), self.second.as_ref()) {
            (Some(RemoteObservation::Found(first)), Some(RemoteObservation::Found(second)))
                if first == second =>
            {
                Ok(first.clone())
            }
            (Some(RemoteObservation::Found(_)), Some(RemoteObservation::Transport)) => {
                Err(refusal("remote_transport"))
            }
            (Some(RemoteObservation::Found(_)), Some(RemoteObservation::NoMatch))
            | (Some(RemoteObservation::Found(_)), Some(RemoteObservation::Found(_))) => {
                Err(refusal("remote_changed"))
            }
            _ => Err(refusal("invalid_sync_command")),
        };
        self.phase = if result.is_ok() {
            Phase::AwaitCandidateVersion
        } else {
            Phase::Terminal
        };
        result
    }

    /// Allows the exact fresh-clone commands only after a valid R0 generation is observed.
    pub fn authorize_fresh_clone(
        &mut self,
        pending_operation_ids: &[String],
    ) -> Result<(), SyncError> {
        if self.phase != Phase::AwaitCloneDecision
            || !matches!(self.first, Some(RemoteObservation::Found(_)))
        {
            return Err(refusal("invalid_sync_command"));
        }
        if !pending_operation_ids.is_empty() {
            return Err(refusal("pending_mutations"));
        }
        self.phase = Phase::AwaitInit;
        Ok(())
    }

    /// Admits only the sealed readonly fence needed to persist a justified failed observation.
    pub fn authorize_failed_observation_fence(&mut self) -> Result<(), SyncError> {
        if !matches!(self.phase, Phase::AwaitCloneDecision | Phase::Terminal)
            || self.first.is_none()
        {
            return Err(refusal("invalid_sync_command"));
        }
        self.phase = Phase::AwaitMetadataVersion;
        Ok(())
    }

    fn observation_command(&self, command: &CommandSpec) -> bool {
        command.program == Path::new("git")
            && command.argv
                == [
                    "ls-remote",
                    "--exit-code",
                    self.binding.project.git_observation_url(),
                    self.binding.project.data_ref(),
                ]
            && command.cwd.as_deref() == Some(self.binding.staging_root.as_path())
            && command.environment == self.binding.environment
            && command.redacted_argv_positions == [2]
    }

    fn init_command(&self, command: &CommandSpec) -> bool {
        command.program == self.binding.binary
            && command.argv
                == [
                    "--sandbox",
                    "init",
                    "--remote",
                    self.binding.project.dolt_remote_url(),
                    "--stealth",
                    "--skip-agents",
                    "--skip-hooks",
                    "--non-interactive",
                ]
            && command.cwd.as_deref() == Some(self.binding.repository.as_path())
            && command.environment == self.binding.environment
            && command.redacted_argv_positions == [3]
    }

    fn remote_list_command(&self, command: &CommandSpec) -> bool {
        command.program == self.binding.binary
            && command.argv == ["--sandbox", "--json", "dolt", "remote", "list"]
            && command.cwd.as_deref() == Some(self.binding.repository.as_path())
            && command.environment == self.binding.environment
            && command.redacted_argv_positions.is_empty()
    }

    fn readonly_status_command(&self, command: &CommandSpec) -> bool {
        command.program == self.binding.binary
            && command.argv == ["--readonly", "--sandbox", "--json", "vc", "status"]
            && command.cwd.as_deref() == Some(self.binding.repository.as_path())
            && command.environment == self.binding.environment
            && command.redacted_argv_positions.is_empty()
    }

    fn version_command(&self, command: &CommandSpec) -> bool {
        command.program == self.binding.binary
            && command.argv == ["--version"]
            && command.cwd.as_deref() == Some(self.binding.staging_root.as_path())
            && command.environment == self.binding.environment
            && command.redacted_argv_positions.is_empty()
    }

    fn readonly_export_command(&self, command: &CommandSpec) -> bool {
        command.program == self.binding.binary
            && command.argv == ["--readonly", "--sandbox", "export"]
            && command.cwd.as_deref() == Some(self.binding.repository.as_path())
            && command.environment == self.binding.environment
            && command.redacted_argv_positions.is_empty()
    }

    fn readonly_key_values_command(&self, command: &CommandSpec) -> bool {
        command.program == self.binding.binary
            && command.argv == ["--readonly", "--sandbox", "--json", "kv", "list"]
            && command.cwd.as_deref() == Some(self.binding.repository.as_path())
            && command.environment == self.binding.environment
            && command.redacted_argv_positions.is_empty()
    }

    fn valid(&self, command: &CommandSpec) -> bool {
        match self.phase {
            Phase::AwaitFirstObservation | Phase::AwaitSecondObservation => {
                self.observation_command(command)
            }
            Phase::AwaitInit => self.init_command(command),
            Phase::AwaitRemoteList => self.remote_list_command(command),
            Phase::AwaitCandidateVersion | Phase::AwaitMetadataVersion => {
                self.version_command(command)
            }
            Phase::AwaitReadonlyBeforeStatus | Phase::AwaitReadonlyAfterStatus => {
                self.readonly_status_command(command)
            }
            Phase::AwaitReadonlyExport => self.readonly_export_command(command),
            Phase::AwaitReadonlyKeyValues => self.readonly_key_values_command(command),
            Phase::AwaitCloneDecision
            | Phase::AwaitStableDecision
            | Phase::Complete
            | Phase::Terminal => false,
        }
    }

    fn record_observation(output: &CommandOutput) -> Result<RemoteObservation, SyncError> {
        match output.status {
            0 => {
                let (commit, reference) = output
                    .stdout
                    .strip_suffix('\n')
                    .filter(|value| !value.contains('\n'))
                    .and_then(|value| value.split_once('\t'))
                    .ok_or_else(|| refusal("invalid_remote_observation"))?;
                if !is_lower_hex_sha(commit) || reference != "refs/dolt/data" {
                    return Err(refusal("invalid_remote_observation"));
                }
                Ok(RemoteObservation::Found(commit.to_owned()))
            }
            2 if output.stdout.is_empty() => Ok(RemoteObservation::NoMatch),
            _ => Ok(RemoteObservation::Transport),
        }
    }

    fn record_remote_list(&self, output: &CommandOutput) -> Result<(), SyncError> {
        if output.status != 0 {
            return Ok(());
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Remote {
            name: String,
            url: String,
            sql_url: String,
            status: String,
        }
        let remotes: Vec<Remote> = serde_json::from_str(&output.stdout)
            .map_err(|_| refusal("remote_configuration_mismatch"))?;
        if !matches!(
            remotes.as_slice(),
            [Remote {
                name,
                url,
                sql_url,
                status,
            }] if name == self.binding.project.remote_name()
                && url == self.binding.project.dolt_remote_url()
                && sql_url == self.binding.project.dolt_remote_url()
                && status == "ok"
        ) {
            return Err(refusal("remote_configuration_mismatch"));
        }
        Ok(())
    }

    fn record(&mut self, command: &CommandSpec, output: &CommandOutput) -> Result<(), SyncError> {
        match self.phase {
            Phase::AwaitFirstObservation => {
                let observation = match Self::record_observation(output) {
                    Ok(observation) => observation,
                    Err(error) => {
                        self.phase = Phase::Terminal;
                        return Err(error);
                    }
                };
                self.phase = if matches!(observation, RemoteObservation::Found(_)) {
                    Phase::AwaitCloneDecision
                } else {
                    Phase::Terminal
                };
                self.first = Some(observation);
            }
            Phase::AwaitInit => {
                self.phase = if output.status == 0 {
                    Phase::AwaitRemoteList
                } else {
                    Phase::Terminal
                };
            }
            Phase::AwaitRemoteList => {
                if let Err(error) = self.record_remote_list(output) {
                    self.phase = Phase::Terminal;
                    return Err(error);
                }
                self.phase = if output.status == 0 {
                    Phase::AwaitSecondObservation
                } else {
                    Phase::Terminal
                };
            }
            Phase::AwaitSecondObservation => {
                let observation = match Self::record_observation(output) {
                    Ok(observation) => observation,
                    Err(error) => {
                        self.phase = Phase::Terminal;
                        return Err(error);
                    }
                };
                self.phase = Phase::AwaitStableDecision;
                self.second = Some(observation);
            }
            Phase::AwaitCandidateVersion | Phase::AwaitMetadataVersion => {
                self.phase = if output.status == 0 {
                    Phase::AwaitReadonlyBeforeStatus
                } else {
                    Phase::Terminal
                };
            }
            Phase::AwaitReadonlyBeforeStatus => {
                self.phase = if output.status == 0 {
                    Phase::AwaitReadonlyExport
                } else {
                    Phase::Terminal
                };
            }
            Phase::AwaitReadonlyExport => {
                self.phase = if output.status == 0 {
                    Phase::AwaitReadonlyKeyValues
                } else {
                    Phase::Terminal
                };
            }
            Phase::AwaitReadonlyKeyValues => {
                self.phase = if output.status == 0 {
                    Phase::AwaitReadonlyAfterStatus
                } else {
                    Phase::Terminal
                };
            }
            Phase::AwaitReadonlyAfterStatus => {
                self.phase = if output.status == 0 {
                    Phase::Complete
                } else {
                    Phase::Terminal
                };
            }
            Phase::AwaitCloneDecision
            | Phase::AwaitStableDecision
            | Phase::Complete
            | Phase::Terminal => {
                return Err(refusal("invalid_sync_command"));
            }
        }
        let _ = command;
        Ok(())
    }
}

impl<R: CommandRunner> CommandRunner for SyncCommandRunner<'_, R> {
    fn run(&mut self, command: CommandSpec) -> Result<CommandOutput, String> {
        if !self.valid(&command) {
            return Err("invalid_sync_command".into());
        }
        let output = self.inner.run(command.clone())?;
        self.record(&command, &output)
            .map_err(|error| error.code().to_owned())?;
        Ok(output)
    }
}

/// The one internal clock dependency used to make persisted synchronization observations
/// deterministic in tests. Callers never supply a timestamp.
trait SyncClock {
    fn now_utc(&self) -> Result<String, SyncError>;
}

struct SystemSyncClock;

impl SyncClock for SystemSyncClock {
    fn now_utc(&self) -> Result<String, SyncError> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| refusal("invalid_store"))?;
        let seconds = i64::try_from(elapsed.as_secs()).map_err(|_| refusal("invalid_store"))?;
        Ok(canonical_utc_from_unix_seconds(seconds))
    }
}

/// Formats a non-negative Unix timestamp as the canonical UTC representation persisted in
/// manifests without adding a general-purpose time dependency.
fn canonical_utc_from_unix_seconds(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    // Civil-date conversion from a proleptic Gregorian day count. The offset maps Unix day zero
    // to 1970-01-01 and keeps the arithmetic valid for every representable positive timestamp.
    let civil = days + 719_468;
    let era = if civil >= 0 { civil } else { civil - 146_096 } / 146_097;
    let day_of_era = civil - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    let hour = remainder / 3_600;
    let minute = (remainder % 3_600) / 60;
    let second = remainder % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn store_refusal(error: StoreError) -> SyncError {
    refusal(error.code())
}

fn command_refusal(error: String) -> SyncError {
    match error.as_str() {
        "invalid_remote_observation" => refusal("invalid_remote_observation"),
        "remote_configuration_mismatch" => refusal("remote_configuration_mismatch"),
        "remote_changed" => refusal("remote_changed"),
        "pending_mutations" => refusal("pending_mutations"),
        "unsupported_beads_version" => refusal("unsupported_beads_version"),
        _ => refusal("remote_transport"),
    }
}

fn observation_command(
    project: &ProjectConfig,
    staging_root: &Path,
    environment: &BTreeMap<String, String>,
) -> CommandSpec {
    CommandSpec {
        program: PathBuf::from("git"),
        argv: vec![
            "ls-remote".into(),
            "--exit-code".into(),
            project.git_observation_url().into(),
            project.data_ref().into(),
        ],
        cwd: Some(staging_root.to_path_buf()),
        environment: environment.clone(),
        redacted_argv_positions: vec![2],
    }
}

fn init_command(
    project: &ProjectConfig,
    repository: &Path,
    binary: &Path,
    environment: &BTreeMap<String, String>,
) -> CommandSpec {
    CommandSpec {
        program: binary.to_path_buf(),
        argv: vec![
            "--sandbox".into(),
            "init".into(),
            "--remote".into(),
            project.dolt_remote_url().into(),
            "--stealth".into(),
            "--skip-agents".into(),
            "--skip-hooks".into(),
            "--non-interactive".into(),
        ],
        cwd: Some(repository.to_path_buf()),
        environment: environment.clone(),
        redacted_argv_positions: vec![3],
    }
}

fn remote_list_command(
    repository: &Path,
    binary: &Path,
    environment: &BTreeMap<String, String>,
) -> CommandSpec {
    CommandSpec {
        program: binary.to_path_buf(),
        argv: vec![
            "--sandbox".into(),
            "--json".into(),
            "dolt".into(),
            "remote".into(),
            "list".into(),
        ],
        cwd: Some(repository.to_path_buf()),
        environment: environment.clone(),
        redacted_argv_positions: Vec::new(),
    }
}

fn binding_for_staging(
    project: ProjectConfig,
    root: &Path,
    repository: PathBuf,
    binary: PathBuf,
    environment: BTreeMap<String, String>,
) -> Result<SyncCommandBinding, SyncError> {
    SyncCommandBinding::new(project, root.to_path_buf(), repository, binary, environment)
}

struct SynchronizationContext<'a> {
    project: &'a ProjectConfig,
    location: &'a StoreLocation,
    selected: &'a CurrentGeneration,
    selected_snapshot: &'a FencedSnapshot,
    pin: &'a crate::pin::PinManifest,
    target: &'a str,
    lock: &'a GenerationActivationLock,
}

fn activate_failed_observation<R: CommandRunner>(
    runner: &mut R,
    context: &SynchronizationContext<'_>,
    observation: FailedSyncObservation,
) -> Result<bool, SyncError> {
    let staging = prepare_sync_staging(
        context.location,
        context.selected,
        context.selected_snapshot,
        context.pin,
        context.target,
        context.lock,
    )
    .map_err(store_refusal)?;
    let binding = binding_for_staging(
        context.project.clone(),
        staging.root(),
        staging.repository(),
        staging.binary(),
        staging.environment().clone(),
    )?;
    let mut fenced = SyncCommandRunner::new_for_failed_observation(runner, binding);
    let activated = staging
        .activate_unknown_if_changed(&mut fenced, observation)
        .map_err(store_refusal)?;
    Ok(activated.is_some())
}

fn failure_after_observation<R: CommandRunner, C: SyncClock>(
    runner: &mut R,
    context: &SynchronizationContext<'_>,
    clock: &C,
    remote_generation: &str,
    failure: SyncError,
) -> Result<SyncResult, SyncError> {
    let observed_at = clock.now_utc()?;
    let state_changed = activate_failed_observation(
        runner,
        context,
        FailedSyncObservation::AfterR0 {
            remote_generation: remote_generation.to_owned(),
            observed_at,
        },
    )?;
    Err(failure.with_state_changed(state_changed))
}

/// Exercises the production preflight continuation with the exact disposable-cleanup refusal.
///
/// The online-sync contract uses this narrow adapter to prove that a completed disposable read
/// cleanup failure prevents every remote command before the normal orchestration begins.
pub(crate) fn synchronize_after_disposable_cleanup_failure_for_contract<R: CommandRunner>(
    runner: &mut R,
    location: &StoreLocation,
    selected: &CurrentGeneration,
    pin: &crate::pin::PinManifest,
    target: &str,
) -> Result<SyncResult, SyncError> {
    continue_sync_after_preflight(
        Err(refusal("temporary_cleanup_failed")),
        runner,
        location,
        selected,
        pin,
        target,
        &SystemSyncClock,
    )
}

/// Performs one explicit zero-remote-write synchronization using the system clock.
pub fn synchronize<R: CommandRunner>(
    runner: &mut R,
    location: &StoreLocation,
    selected: &CurrentGeneration,
    pin: &crate::pin::PinManifest,
    target: &str,
) -> Result<SyncResult, SyncError> {
    let preflight = read_disposable_snapshot(runner, selected, pin, target).map_err(store_refusal);
    continue_sync_after_preflight(
        preflight,
        runner,
        location,
        selected,
        pin,
        target,
        &SystemSyncClock,
    )
}

#[cfg(test)]
fn synchronize_with_clock<R: CommandRunner, C: SyncClock>(
    runner: &mut R,
    location: &StoreLocation,
    selected: &CurrentGeneration,
    selected_snapshot: &FencedSnapshot,
    pin: &crate::pin::PinManifest,
    target: &str,
    clock: &C,
) -> Result<SyncResult, SyncError> {
    continue_sync_after_preflight(
        Ok(selected_snapshot.clone()),
        runner,
        location,
        selected,
        pin,
        target,
        clock,
    )
}

fn continue_sync_after_preflight<R: CommandRunner, C: SyncClock>(
    preflight: Result<FencedSnapshot, SyncError>,
    runner: &mut R,
    location: &StoreLocation,
    selected: &CurrentGeneration,
    pin: &crate::pin::PinManifest,
    target: &str,
    clock: &C,
) -> Result<SyncResult, SyncError> {
    let selected_snapshot = preflight?;
    let project = compiled_project_config().map_err(|_| refusal("invalid_project_config"))?;
    let lock = GenerationActivationLock::acquire_for_sync(location).map_err(store_refusal)?;
    let context = SynchronizationContext {
        project: &project,
        location,
        selected,
        selected_snapshot: &selected_snapshot,
        pin,
        target,
        lock: &lock,
    };
    let staging = prepare_sync_staging(
        context.location,
        context.selected,
        context.selected_snapshot,
        context.pin,
        context.target,
        context.lock,
    )
    .map_err(store_refusal)?;
    let binding = binding_for_staging(
        context.project.clone(),
        staging.root(),
        staging.repository(),
        staging.binary(),
        staging.environment().clone(),
    )?;
    let mut fenced = SyncCommandRunner::new(runner, binding);
    match fenced.run(observation_command(
        context.project,
        staging.root(),
        staging.environment(),
    )) {
        Ok(_) => {}
        Err(error) => {
            drop(fenced);
            drop(staging);
            let state_changed =
                activate_failed_observation(runner, &context, FailedSyncObservation::BeforeR0)?;
            return Err(command_refusal(error).with_state_changed(state_changed));
        }
    }
    let r0 = match fenced.first_outcome().cloned() {
        Some(RemoteObservation::Found(remote)) => remote,
        Some(RemoteObservation::NoMatch) => {
            drop(fenced);
            drop(staging);
            let state_changed =
                activate_failed_observation(runner, &context, FailedSyncObservation::BeforeR0)?;
            return Err(refusal("remote_uninitialized").with_state_changed(state_changed));
        }
        Some(RemoteObservation::Transport) => {
            drop(fenced);
            drop(staging);
            let state_changed =
                activate_failed_observation(runner, &context, FailedSyncObservation::BeforeR0)?;
            return Err(refusal("remote_transport").with_state_changed(state_changed));
        }
        None => {
            drop(fenced);
            drop(staging);
            let state_changed =
                activate_failed_observation(runner, &context, FailedSyncObservation::BeforeR0)?;
            return Err(refusal("invalid_remote_observation").with_state_changed(state_changed));
        }
    };
    if let Err(error) =
        fenced.authorize_fresh_clone(&context.selected.manifest.pending_operation_ids)
    {
        drop(fenced);
        drop(staging);
        return failure_after_observation(runner, &context, clock, &r0, error);
    }
    let candidate = match staging.create_fresh_repository() {
        Ok(candidate) => candidate,
        Err(error) => return Err(store_refusal(error)),
    };
    match fenced.run(init_command(
        context.project,
        &candidate.repository(),
        &candidate.binary(),
        candidate.environment(),
    )) {
        Ok(output) if output.status == 0 => {}
        Ok(_) => {
            drop(fenced);
            return failure_after_observation(
                runner,
                &context,
                clock,
                &r0,
                refusal("remote_transport"),
            );
        }
        Err(error) => {
            drop(fenced);
            return failure_after_observation(runner, &context, clock, &r0, command_refusal(error));
        }
    }
    match fenced.run(remote_list_command(
        &candidate.repository(),
        &candidate.binary(),
        candidate.environment(),
    )) {
        Ok(output) if output.status == 0 => {}
        Ok(_) => {
            drop(fenced);
            return failure_after_observation(
                runner,
                &context,
                clock,
                &r0,
                refusal("remote_transport"),
            );
        }
        Err(error) => {
            drop(fenced);
            return failure_after_observation(runner, &context, clock, &r0, command_refusal(error));
        }
    }
    let r1 = match fenced.run(observation_command(
        context.project,
        candidate.root(),
        candidate.environment(),
    )) {
        Ok(_) => fenced.second_observation().unwrap_or_else(|| r0.clone()),
        Err(error) => {
            drop(fenced);
            return failure_after_observation(runner, &context, clock, &r0, command_refusal(error));
        }
    };
    let stable_remote = match fenced.require_stable_observation() {
        Ok(remote) => remote,
        Err(error) => {
            drop(fenced);
            return failure_after_observation(runner, &context, clock, &r1, error);
        }
    };
    let synchronized_at = clock.now_utc()?;
    let finalized =
        match candidate.validate_and_finalize(&mut fenced, &stable_remote, &synchronized_at) {
            Ok(finalized) => finalized,
            Err(error) => {
                drop(fenced);
                return failure_after_observation(
                    runner,
                    &context,
                    clock,
                    &stable_remote,
                    store_refusal(error),
                );
            }
        };
    drop(fenced);
    let activated = finalized.activate().map_err(store_refusal)?;
    let freshness = classify(ObservationState {
        last_successful_sync_at: activated.manifest.last_successful_sync_at.clone(),
        local_generation: activated.manifest.local_generation.clone(),
        remote_generation: activated.manifest.remote_generation.clone(),
        remote_observed_at: activated.manifest.remote_observed_at.clone(),
        observed_local_generation: activated.manifest.observed_local_generation.clone(),
        remote_relation: activated.manifest.remote_relation.clone(),
        pending_mutations: PendingMutations {
            operation_ids: activated.manifest.pending_operation_ids.clone(),
        },
    })
    .map_err(|_| refusal("invalid_store"))?;
    Ok(SyncResult::synchronized(
        activated.manifest.source_commit,
        freshness,
        true,
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;

    use sha2::{Digest, Sha256};

    use super::*;
    use crate::command::{CommandOutput, CommandRunner, CommandSpec, RecordingCommandRunner};
    use crate::freshness::{
        Freshness, ObservationState, PendingMutations, RemoteRelation, classify,
    };
    use crate::pin::PinManifest;
    use crate::shadow::{
        canonical_logical_export, canonical_operational_projection, logical_export_digest,
        operational_projection_digest,
    };
    use crate::store::{
        CurrentGeneration, FencedSnapshot, StateManifest, StoreLocation, current_generation,
    };

    struct FixedClock(&'static str);

    impl SyncClock for FixedClock {
        fn now_utc(&self) -> Result<String, SyncError> {
            Ok(self.0.into())
        }
    }

    fn checksum(path: &Path) -> String {
        format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
    }

    fn sync_fixture() -> (
        tempfile::TempDir,
        StoreLocation,
        CurrentGeneration,
        FencedSnapshot,
        PinManifest,
    ) {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let checkout = root.path().join("checkout");
        let common = root.path().join("common");
        fs::create_dir_all(&checkout).unwrap();
        fs::create_dir_all(&common).unwrap();
        let checkout = checkout.canonicalize().unwrap();
        let common = common.canonicalize().unwrap();
        let location = StoreLocation {
            worktree_root: checkout,
            common_dir: common.clone(),
            state_root: common.join("plasmosome-work-state"),
            generations_dir: common.join("plasmosome-work-state/generations"),
        };
        let generation_root = location.generations_dir.join("generation-active");
        fs::create_dir_all(&generation_root).unwrap();
        let wrapper = generation_root.join("plasmosome-work-state");
        let binary = generation_root.join("bd");
        fs::write(&wrapper, "installed wrapper").unwrap();
        fs::write(&binary, "installed bd").unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = generation_root.join("runtime");
        for directory in ["home", "xdg_config", "xdg_cache", "xdg_data", "tmp"] {
            fs::create_dir_all(runtime.join(directory)).unwrap();
        }
        fs::write(runtime.join("git_config_global"), "").unwrap();
        fs::create_dir(generation_root.join("repository")).unwrap();
        let source_commit = "a".repeat(40);
        let local_generation = "b".repeat(40);
        let empty_documents = Vec::new();
        let empty_operational: Vec<crate::shadow::OperationalDocument> = Vec::new();
        let manifest = StateManifest {
            schema_version: 1,
            authority_mode: "markdown-shadow".into(),
            source_commit: source_commit.clone(),
            logical_export_sha256: logical_export_digest(
                &canonical_logical_export(&empty_documents).unwrap(),
            ),
            operational_projection_sha256: operational_projection_digest(
                &canonical_operational_projection(&empty_operational).unwrap(),
            ),
            local_generation: local_generation.clone(),
            host_target: "aarch64-apple-darwin".into(),
            wrapper_sha256: checksum(&wrapper),
            beads_binary_sha256: checksum(&binary),
            remote_relation: RemoteRelation::Unknown,
            remote_generation: None,
            remote_observed_at: None,
            observed_local_generation: None,
            last_successful_sync_at: None,
            pending_operation_ids: Vec::new(),
        };
        fs::write(
            generation_root.join("state.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(location.state_root.join("current"), "generation-active\n").unwrap();
        let selected = CurrentGeneration {
            name: "generation-active".into(),
            root: generation_root,
            manifest,
        };
        let snapshot = FencedSnapshot {
            documents: Vec::new(),
            freshness: classify(ObservationState {
                last_successful_sync_at: None,
                local_generation,
                remote_generation: None,
                remote_observed_at: None,
                observed_local_generation: None,
                remote_relation: RemoteRelation::Unknown,
                pending_mutations: PendingMutations {
                    operation_ids: Vec::new(),
                },
            })
            .unwrap(),
        };
        let pin = PinManifest::parse(&format!(
            "version = \"1.1.2\"\nrelease = \"https://example.invalid/release\"\nsource_commit = \"{}\"\nlicense = \"MIT\"\nchecksums_url = \"https://example.invalid/checksums\"\nchecksums_sha256 = \"{}\"\n\n[[targets]]\ntarget = \"aarch64-apple-darwin\"\narchive = \"beads_1.1.2_test.tar.gz\"\narchive_sha256 = \"{}\"\nbinary_sha256 = \"{}\"\n",
            "c".repeat(40),
            "d".repeat(64),
            "e".repeat(64),
            selected.manifest.beads_binary_sha256,
        ))
        .unwrap();
        (root, location, selected, snapshot, pin)
    }

    #[test]
    fn stable_compatible_sync_activates_one_complete_generation() {
        let (_root, location, selected, snapshot, pin) = sync_fixture();
        let remote = "e".repeat(40);
        let candidate_generation = "f".repeat(40);
        let status = |commit: &str| {
            serde_json::json!({
                "schema_version": 1,
                "branch": "main",
                "commit": commit,
            })
            .to_string()
        };
        let remote_list = r#"[{"name":"origin","url":"git+https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok"}]"#;
        let keys = serde_json::json!({
            "schema_version": 1,
            "plasmosome.authority-mode": "markdown-shadow",
            "plasmosome.source-commit": selected.manifest.source_commit,
        })
        .to_string();
        let mut runner = RecordingCommandRunner::scripted(vec![
            Ok(CommandOutput::success(format!(
                "{remote}\trefs/dolt/data\n"
            ))),
            Ok(CommandOutput::success("")),
            Ok(CommandOutput::success(remote_list)),
            Ok(CommandOutput::success(format!(
                "{remote}\trefs/dolt/data\n"
            ))),
            Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
            Ok(CommandOutput::success(status(&candidate_generation))),
            Ok(CommandOutput::success("")),
            Ok(CommandOutput::success(keys)),
            Ok(CommandOutput::success(status(&candidate_generation))),
        ]);

        let result = synchronize_with_clock(
            &mut runner,
            &location,
            &selected,
            &snapshot,
            &pin,
            "aarch64-apple-darwin",
            &FixedClock("2026-09-02T00:00:00Z"),
        )
        .expect("stable exact parity activates a complete new generation");
        assert!(result.state_changed);
        assert_eq!(
            result.freshness.remote_generation.as_deref(),
            Some(remote.as_str())
        );
        assert_eq!(result.freshness.local_generation, candidate_generation);
        assert_eq!(
            result.freshness.last_successful_sync_at.as_deref(),
            Some("2026-09-02T00:00:00Z")
        );
        assert_ne!(current_generation(&location).unwrap().name, selected.name);
        assert!(runner.finish().is_ok());
    }

    #[test]
    fn existing_remote_is_cloned_into_fresh_staging_not_pulled_into_local_history() {
        struct MaterializingCloneRunner {
            scripted: RecordingCommandRunner,
            commands: Vec<CommandSpec>,
            remote_candidate: PathBuf,
            active_repository: PathBuf,
            init_count: usize,
        }

        impl MaterializingCloneRunner {
            fn finish(self) {
                assert_eq!(
                    self.init_count, 1,
                    "only the admitted init materializes a candidate"
                );
                self.scripted.finish().unwrap();
            }
        }

        impl CommandRunner for MaterializingCloneRunner {
            fn run(&mut self, command: CommandSpec) -> Result<CommandOutput, String> {
                let is_init = command.argv.get(1).map(String::as_str) == Some("init");
                if is_init {
                    assert_eq!(
                        command.argv,
                        [
                            "--sandbox",
                            "init",
                            "--remote",
                            "git+https://github.com/teonimesic/plasmosome.git",
                            "--stealth",
                            "--skip-agents",
                            "--skip-hooks",
                            "--non-interactive",
                        ]
                    );
                    let repository = command
                        .cwd
                        .as_deref()
                        .expect("the admitted init has the fresh staging repository cwd");
                    assert!(
                        fs::read_dir(repository).unwrap().next().is_none(),
                        "the staged repository is empty before the remote clone materializes"
                    );
                    assert_eq!(
                        command.program,
                        repository.parent().unwrap().join("bd"),
                        "init uses only the copied staged binary"
                    );
                    assert_eq!(command.redacted_argv_positions, vec![3]);
                    assert!(
                        !repository.join("active-only").exists(),
                        "a fresh candidate cannot inherit active local history"
                    );
                    fs::copy(
                        self.remote_candidate.join("remote-only"),
                        repository.join("remote-only"),
                    )
                    .map_err(|_| "candidate materialization failed".to_owned())?;
                    assert!(
                        self.active_repository.join("active-only").exists(),
                        "the test keeps the active repository separate from the materialized candidate"
                    );
                    self.init_count += 1;
                    self.commands.push(command);
                    return Ok(CommandOutput::success(""));
                }
                let output = self.scripted.run(command.clone())?;
                self.commands.push(command);
                Ok(output)
            }
        }

        let (root, location, selected, _snapshot, pin) = sync_fixture();
        let active_repository = selected.root.join("repository");
        fs::write(active_repository.join("active-only"), "active history").unwrap();
        let remote_candidate = root.path().join("recorded-remote-candidate");
        fs::create_dir(&remote_candidate).unwrap();
        fs::write(remote_candidate.join("remote-only"), "remote candidate").unwrap();
        let remote = "e".repeat(40);
        let candidate_generation = "f".repeat(40);
        let status = |commit: &str| {
            serde_json::json!({
                "schema_version": 1,
                "branch": "main",
                "commit": commit,
            })
            .to_string()
        };
        let keys = serde_json::json!({
            "schema_version": 1,
            "plasmosome.authority-mode": "markdown-shadow",
            "plasmosome.source-commit": selected.manifest.source_commit,
        })
        .to_string();
        let selected_status = status(&selected.manifest.local_generation);
        let mut runner = MaterializingCloneRunner {
            scripted: RecordingCommandRunner::scripted(vec![
                Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
                Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
                Ok(CommandOutput::success(selected_status.clone())),
                Ok(CommandOutput::success("")),
                Ok(CommandOutput::success(keys.clone())),
                Ok(CommandOutput::success(selected_status)),
                Ok(CommandOutput::success(format!(
                    "{remote}\trefs/dolt/data\n"
                ))),
                Ok(CommandOutput::success(
                    r#"[{"name":"origin","url":"git+https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok"}]"#,
                )),
                Ok(CommandOutput::success(format!(
                    "{remote}\trefs/dolt/data\n"
                ))),
                Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
                Ok(CommandOutput::success(status(&candidate_generation))),
                Ok(CommandOutput::success("")),
                Ok(CommandOutput::success(keys)),
                Ok(CommandOutput::success(status(&candidate_generation))),
            ]),
            commands: Vec::new(),
            remote_candidate,
            active_repository,
            init_count: 0,
        };

        let result = synchronize(
            &mut runner,
            &location,
            &selected,
            &pin,
            "aarch64-apple-darwin",
        )
        .expect("the exact existing remote may activate only the materialized fresh candidate");

        assert!(result.state_changed);
        let activated = current_generation(&location).unwrap();
        assert_ne!(activated.name, selected.name);
        assert!(activated.root.join("repository/remote-only").exists());
        assert!(!activated.root.join("repository/active-only").exists());
        assert!(selected.root.join("repository/active-only").exists());
        assert_eq!(runner.commands.len(), 15);
        assert_eq!(runner.commands[6].program, PathBuf::from("git"));
        assert!(runner.commands.iter().all(|command| {
            !command.argv.iter().any(|argument| {
                matches!(
                    argument.as_str(),
                    "add" | "pull" | "push" | "fetch" | "force"
                )
            })
        }));
        runner.finish();
    }

    #[test]
    fn sync_failure_activation_is_idempotent() {
        fn snapshot_for(generation: &CurrentGeneration) -> FencedSnapshot {
            FencedSnapshot {
                documents: Vec::new(),
                freshness: classify(ObservationState {
                    last_successful_sync_at: generation.manifest.last_successful_sync_at.clone(),
                    local_generation: generation.manifest.local_generation.clone(),
                    remote_generation: generation.manifest.remote_generation.clone(),
                    remote_observed_at: generation.manifest.remote_observed_at.clone(),
                    observed_local_generation: generation
                        .manifest
                        .observed_local_generation
                        .clone(),
                    remote_relation: generation.manifest.remote_relation.clone(),
                    pending_mutations: PendingMutations {
                        operation_ids: generation.manifest.pending_operation_ids.clone(),
                    },
                })
                .unwrap(),
            }
        }

        fn metadata_fence_outputs(local: &str, source: &str) -> Vec<Result<CommandOutput, String>> {
            let status = serde_json::json!({
                "schema_version": 1,
                "branch": "main",
                "commit": local,
            })
            .to_string();
            let keys = serde_json::json!({
                "schema_version": 1,
                "plasmosome.authority-mode": "markdown-shadow",
                "plasmosome.source-commit": source,
            })
            .to_string();
            vec![
                Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
                Ok(CommandOutput::success(status.clone())),
                Ok(CommandOutput::success("")),
                Ok(CommandOutput::success(keys)),
                Ok(CommandOutput::success(status)),
            ]
        }

        let (_root, location, selected, snapshot, pin) = sync_fixture();
        let remote = "e".repeat(40);
        let observed_at = "2026-09-02T00:00:00Z";
        let mut first_script = vec![
            Ok(CommandOutput::success(format!(
                "{remote}\trefs/dolt/data\n"
            ))),
            Ok(CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "remote init failed".into(),
            }),
        ];
        first_script.extend(metadata_fence_outputs(
            &selected.manifest.local_generation,
            &selected.manifest.source_commit,
        ));
        let mut first_runner = RecordingCommandRunner::scripted(first_script);
        let first = synchronize_with_clock(
            &mut first_runner,
            &location,
            &selected,
            &snapshot,
            &pin,
            "aarch64-apple-darwin",
            &FixedClock(observed_at),
        )
        .unwrap_err();
        assert_eq!(first.code(), "remote_transport");
        assert!(
            first.state_changed(),
            "the first complete Unknown observation atomically activates metadata"
        );
        first_runner.finish().unwrap();

        let first_activated = current_generation(&location).unwrap();
        let current_after_first = fs::read(location.state_root.join("current")).unwrap();
        assert_ne!(first_activated.name, selected.name);
        assert_eq!(
            first_activated.manifest.remote_relation,
            RemoteRelation::Unknown
        );
        assert_eq!(
            first_activated.manifest.remote_generation.as_deref(),
            Some(remote.as_str())
        );
        assert_eq!(
            first_activated.manifest.remote_observed_at.as_deref(),
            Some(observed_at)
        );

        let mut second_runner = RecordingCommandRunner::scripted(vec![
            Ok(CommandOutput::success(format!(
                "{remote}\trefs/dolt/data\n"
            ))),
            Ok(CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "remote init failed again".into(),
            }),
        ]);
        let second = synchronize_with_clock(
            &mut second_runner,
            &location,
            &first_activated,
            &snapshot_for(&first_activated),
            &pin,
            "aarch64-apple-darwin",
            &FixedClock(observed_at),
        )
        .unwrap_err();
        assert_eq!(second.code(), "remote_transport");
        assert!(
            !second.state_changed(),
            "an identical post-R0 failure cannot replace current again"
        );
        assert_eq!(
            second_runner.commands().len(),
            2,
            "the retry performs no metadata repository copy or readonly fence"
        );
        second_runner.finish().unwrap();
        assert_eq!(
            fs::read(location.state_root.join("current")).unwrap(),
            current_after_first
        );
        assert_eq!(current_generation(&location).unwrap(), first_activated);
    }

    #[test]
    fn sync_cleanup_failure_precedes_remote_refusal() {
        let (_root, location, selected, _snapshot, pin) = sync_fixture();
        let current_before = fs::read(location.state_root.join("current")).unwrap();
        let mut runner = RecordingCommandRunner::with_output(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "remote transport would fail if dispatched".into(),
        });

        let error = continue_sync_after_preflight(
            Err(refusal("temporary_cleanup_failed")),
            &mut runner,
            &location,
            &selected,
            &pin,
            "aarch64-apple-darwin",
            &FixedClock("2026-09-02T00:00:00Z"),
        )
        .unwrap_err();

        assert_eq!(error.code(), "temporary_cleanup_failed");
        assert!(!error.state_changed());
        assert!(
            runner.commands().is_empty(),
            "a failed disposable cleanup must stop before Git observation or staging"
        );
        assert_eq!(
            fs::read(location.state_root.join("current")).unwrap(),
            current_before
        );
    }

    #[test]
    fn contract_cleanup_inventory_stops_before_remote_dispatch() {
        let (_root, location, selected, _snapshot, pin) = sync_fixture();
        let current_before = fs::read(location.state_root.join("current")).unwrap();
        let mut runner = RecordingCommandRunner::with_output(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "the recorded remote refusal must stay undispatched".into(),
        });

        let error = synchronize_after_disposable_cleanup_failure_for_contract(
            &mut runner,
            &location,
            &selected,
            &pin,
            "aarch64-apple-darwin",
        )
        .unwrap_err();

        assert_eq!(error.code(), "temporary_cleanup_failed");
        assert!(!error.state_changed());
        assert!(runner.commands().is_empty());
        assert_eq!(
            fs::read(location.state_root.join("current")).unwrap(),
            current_before
        );
    }

    #[test]
    fn remote_no_match_stops_before_any_beads_remote_command() {
        let (_root, location, selected, snapshot, pin) = sync_fixture();
        let current_before = fs::read(location.state_root.join("current")).unwrap();
        let mut runner = RecordingCommandRunner::with_output(CommandOutput {
            status: 2,
            stdout: String::new(),
            stderr: "missing data ref".into(),
        });

        let error = synchronize_with_clock(
            &mut runner,
            &location,
            &selected,
            &snapshot,
            &pin,
            "aarch64-apple-darwin",
            &FixedClock("2026-09-02T00:00:00Z"),
        )
        .unwrap_err();

        assert_eq!(error.code(), "remote_uninitialized");
        assert_eq!(
            runner.commands().len(),
            1,
            "R0 no-match dispatches no Beads command"
        );
        assert_eq!(runner.commands()[0].program, PathBuf::from("git"));
        assert_eq!(
            fs::read(location.state_root.join("current")).unwrap(),
            current_before,
            "a no-match cannot replace the active generation"
        );
        runner.finish().unwrap();
    }

    #[test]
    fn moving_remote_never_activates_the_cloned_candidate() {
        fn metadata_fence_outputs(local: &str, source: &str) -> Vec<Result<CommandOutput, String>> {
            let status = serde_json::json!({
                "schema_version": 1,
                "branch": "main",
                "commit": local,
            })
            .to_string();
            let keys = serde_json::json!({
                "schema_version": 1,
                "plasmosome.authority-mode": "markdown-shadow",
                "plasmosome.source-commit": source,
            })
            .to_string();
            vec![
                Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
                Ok(CommandOutput::success(status.clone())),
                Ok(CommandOutput::success("")),
                Ok(CommandOutput::success(keys)),
                Ok(CommandOutput::success(status)),
            ]
        }

        let (_root, location, selected, snapshot, pin) = sync_fixture();
        let current_before = fs::read(location.state_root.join("current")).unwrap();
        fs::write(
            selected.root.join("repository/active-only"),
            "the selected repository is not the remote candidate",
        )
        .unwrap();
        let r0 = "e".repeat(40);
        let r1 = "f".repeat(40);
        let remote_list = r#"[{"name":"origin","url":"git+https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok"}]"#;
        let mut script = vec![
            Ok(CommandOutput::success(format!("{r0}\trefs/dolt/data\n"))),
            Ok(CommandOutput::success("")),
            Ok(CommandOutput::success(remote_list)),
            Ok(CommandOutput::success(format!("{r1}\trefs/dolt/data\n"))),
        ];
        script.extend(metadata_fence_outputs(
            &selected.manifest.local_generation,
            &selected.manifest.source_commit,
        ));
        let mut runner = RecordingCommandRunner::scripted(script);

        let error = synchronize_with_clock(
            &mut runner,
            &location,
            &selected,
            &snapshot,
            &pin,
            "aarch64-apple-darwin",
            &FixedClock("2026-09-02T00:00:00Z"),
        )
        .unwrap_err();

        assert_eq!(error.code(), "remote_changed");
        assert!(
            error.state_changed(),
            "the justified R1 observation is persisted separately from the rejected candidate"
        );
        let activated = current_generation(&location).unwrap();
        assert_ne!(activated.name, selected.name);
        assert!(activated.root.join("repository/active-only").is_file());
        assert!(
            !activated.root.join("repository/remote-only").exists(),
            "a moved remote cannot expose the fresh clone candidate"
        );
        assert_eq!(activated.manifest.remote_relation, RemoteRelation::Unknown);
        assert_eq!(
            activated.manifest.remote_generation.as_deref(),
            Some(r1.as_str())
        );
        assert_ne!(
            fs::read(location.state_root.join("current")).unwrap(),
            current_before,
            "only a metadata-only Unknown observation may replace the pointer"
        );
        assert!(runner.commands().iter().all(|command| {
            !command.argv.iter().any(|argument| {
                matches!(
                    argument.as_str(),
                    "add" | "pull" | "push" | "fetch" | "force"
                )
            })
        }));
        runner.finish().unwrap();
    }

    #[test]
    fn remote_failure_phases_never_expose_the_fresh_candidate() {
        fn metadata_fence_outputs(local: &str, source: &str) -> Vec<Result<CommandOutput, String>> {
            let status = serde_json::json!({
                "schema_version": 1,
                "branch": "main",
                "commit": local,
            })
            .to_string();
            let keys = serde_json::json!({
                "schema_version": 1,
                "plasmosome.authority-mode": "markdown-shadow",
                "plasmosome.source-commit": source,
            })
            .to_string();
            vec![
                Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
                Ok(CommandOutput::success(status.clone())),
                Ok(CommandOutput::success("")),
                Ok(CommandOutput::success(keys)),
                Ok(CommandOutput::success(status)),
            ]
        }

        let r0 = "e".repeat(40);
        let r1 = "f".repeat(40);
        let found = |generation: &str| {
            Ok(CommandOutput::success(format!(
                "{generation}\trefs/dolt/data\n"
            )))
        };
        let remote_list = || {
            Ok(CommandOutput::success(
                r#"[{"name":"origin","url":"git+https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok"}]"#,
            ))
        };
        let transport = |message: &str| {
            Ok(CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: message.into(),
            })
        };
        let no_match = || {
            Ok(CommandOutput {
                status: 2,
                stdout: String::new(),
                stderr: "the data ref disappeared".into(),
            })
        };
        let malformed = || Ok(CommandOutput::success("not-a-lowercase-data-ref\n"));
        let remote_mismatch = || Ok(CommandOutput::success("[]"));
        let cases = vec![
            (
                "transport before R0",
                vec![transport("R0 transport")],
                "remote_transport",
                false,
                None,
            ),
            (
                "malformed R0",
                vec![malformed()],
                "invalid_remote_observation",
                false,
                None,
            ),
            (
                "init transport",
                vec![found(&r0), transport("init transport")],
                "remote_transport",
                true,
                Some(r0.clone()),
            ),
            (
                "remote-list transport",
                vec![
                    found(&r0),
                    Ok(CommandOutput::success("")),
                    transport("remote-list transport"),
                ],
                "remote_transport",
                true,
                Some(r0.clone()),
            ),
            (
                "remote-list mismatch",
                vec![
                    found(&r0),
                    Ok(CommandOutput::success("")),
                    remote_mismatch(),
                ],
                "remote_configuration_mismatch",
                true,
                Some(r0.clone()),
            ),
            (
                "R1 transport",
                vec![
                    found(&r0),
                    Ok(CommandOutput::success("")),
                    remote_list(),
                    transport("R1 transport"),
                ],
                "remote_transport",
                true,
                Some(r0.clone()),
            ),
            (
                "R1 no-match",
                vec![
                    found(&r0),
                    Ok(CommandOutput::success("")),
                    remote_list(),
                    no_match(),
                ],
                "remote_changed",
                true,
                Some(r0.clone()),
            ),
            (
                "malformed R1",
                vec![
                    found(&r0),
                    Ok(CommandOutput::success("")),
                    remote_list(),
                    malformed(),
                ],
                "invalid_remote_observation",
                true,
                Some(r0.clone()),
            ),
            (
                "moved R1",
                vec![
                    found(&r0),
                    Ok(CommandOutput::success("")),
                    remote_list(),
                    found(&r1),
                ],
                "remote_changed",
                true,
                Some(r1.clone()),
            ),
        ];

        for (label, mut script, expected_code, expected_state_changed, expected_remote) in cases {
            let (_root, location, selected, snapshot, pin) = sync_fixture();
            let current_before = fs::read(location.state_root.join("current")).unwrap();
            fs::write(
                selected.root.join("repository/active-only"),
                "the remote candidate may never replace this repository on refusal",
            )
            .unwrap();
            if expected_state_changed {
                script.extend(metadata_fence_outputs(
                    &selected.manifest.local_generation,
                    &selected.manifest.source_commit,
                ));
            }
            let mut runner = RecordingCommandRunner::scripted(script);

            let error = synchronize_with_clock(
                &mut runner,
                &location,
                &selected,
                &snapshot,
                &pin,
                "aarch64-apple-darwin",
                &FixedClock("2026-09-02T00:00:00Z"),
            )
            .unwrap_err();

            assert_eq!(error.code(), expected_code, "{label}");
            assert_eq!(error.state_changed(), expected_state_changed, "{label}");
            let current = fs::read(location.state_root.join("current")).unwrap();
            if expected_state_changed {
                let activated = current_generation(&location).unwrap();
                assert_ne!(activated.name, selected.name, "{label}");
                assert!(
                    activated.root.join("repository/active-only").is_file(),
                    "{label}"
                );
                assert!(
                    !activated.root.join("repository/remote-only").exists(),
                    "{label}: a rejected clone cannot become active"
                );
                assert_eq!(
                    activated.manifest.remote_generation.as_deref(),
                    expected_remote.as_deref(),
                    "{label}"
                );
                assert_eq!(
                    activated.manifest.remote_relation,
                    RemoteRelation::Unknown,
                    "{label}"
                );
                assert_ne!(current, current_before, "{label}");
            } else {
                assert_eq!(current, current_before, "{label}");
            }
            assert!(runner.commands().iter().all(|command| {
                !command.argv.iter().any(|argument| {
                    matches!(
                        argument.as_str(),
                        "add" | "pull" | "push" | "fetch" | "force"
                    )
                })
            }));
            runner.finish().unwrap();
        }
    }

    #[test]
    fn pending_mutations_are_observed_but_never_cloned_over() {
        let (_root, location, mut selected, mut snapshot, pin) = sync_fixture();
        let pending = "pending-1".to_owned();
        selected.manifest.pending_operation_ids = vec![pending.clone()];
        fs::write(
            selected.root.join("state.json"),
            serde_json::to_vec(&selected.manifest).unwrap(),
        )
        .unwrap();
        snapshot.freshness = classify(ObservationState {
            last_successful_sync_at: None,
            local_generation: selected.manifest.local_generation.clone(),
            remote_generation: None,
            remote_observed_at: None,
            observed_local_generation: None,
            remote_relation: RemoteRelation::Unknown,
            pending_mutations: PendingMutations {
                operation_ids: vec![pending.clone()],
            },
        })
        .unwrap();
        let remote = "e".repeat(40);
        let local = selected.manifest.local_generation.clone();
        let status = serde_json::json!({
            "schema_version": 1,
            "branch": "main",
            "commit": local,
        })
        .to_string();
        let keys = serde_json::json!({
            "schema_version": 1,
            "plasmosome.authority-mode": "markdown-shadow",
            "plasmosome.source-commit": selected.manifest.source_commit,
        })
        .to_string();
        let mut runner = RecordingCommandRunner::scripted(vec![
            Ok(CommandOutput::success(format!(
                "{remote}\trefs/dolt/data\n"
            ))),
            Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
            Ok(CommandOutput::success(status.clone())),
            Ok(CommandOutput::success("")),
            Ok(CommandOutput::success(keys)),
            Ok(CommandOutput::success(status)),
        ]);

        let error = synchronize_with_clock(
            &mut runner,
            &location,
            &selected,
            &snapshot,
            &pin,
            "aarch64-apple-darwin",
            &FixedClock("2026-09-02T00:00:00Z"),
        )
        .unwrap_err();

        assert_eq!(error.code(), "pending_mutations");
        assert!(
            runner
                .commands()
                .iter()
                .all(|command| !command.argv.iter().any(|argument| argument == "init")),
            "pending observations must stop before the fresh remote clone"
        );
        let activated = current_generation(&location).unwrap();
        assert_ne!(activated.name, selected.name);
        assert_eq!(activated.manifest.pending_operation_ids, vec![pending]);
        assert_eq!(activated.manifest.remote_relation, RemoteRelation::Unknown);
        assert_eq!(
            activated.manifest.remote_generation.as_deref(),
            Some(remote.as_str())
        );
        assert_eq!(
            activated.manifest.remote_observed_at.as_deref(),
            Some("2026-09-02T00:00:00Z")
        );
        runner.finish().unwrap();
    }

    #[test]
    fn pending_same_equivalent_observation_stays_unpublished_through_later_reobservation() {
        fn readonly_outputs(local: &str, source: &str) -> Vec<Result<CommandOutput, String>> {
            let status = serde_json::json!({
                "schema_version": 1,
                "branch": "main",
                "commit": local,
            })
            .to_string();
            let keys = serde_json::json!({
                "schema_version": 1,
                "plasmosome.authority-mode": "markdown-shadow",
                "plasmosome.source-commit": source,
            })
            .to_string();
            vec![
                Ok(CommandOutput::success("bd version 1.1.2 (test)\n")),
                Ok(CommandOutput::success(status.clone())),
                Ok(CommandOutput::success("")),
                Ok(CommandOutput::success(keys)),
                Ok(CommandOutput::success(status)),
            ]
        }

        let (_root, location, mut selected, mut snapshot, pin) = sync_fixture();
        let remote = "e".repeat(40);
        let first_observed_at = "2026-09-02T00:00:00Z";
        let second_observed_at = "2026-09-02T00:01:00Z";
        let pending = vec!["pending-first".to_owned(), "pending-second".to_owned()];
        selected.manifest.remote_relation = RemoteRelation::Equivalent;
        selected.manifest.remote_generation = Some(remote.clone());
        selected.manifest.remote_observed_at = Some(first_observed_at.into());
        selected.manifest.observed_local_generation =
            Some(selected.manifest.local_generation.clone());
        selected.manifest.last_successful_sync_at = Some(first_observed_at.into());
        selected.manifest.pending_operation_ids = pending.clone();
        fs::write(
            selected.root.join("state.json"),
            serde_json::to_vec(&selected.manifest).unwrap(),
        )
        .unwrap();
        snapshot.freshness = classify(ObservationState {
            last_successful_sync_at: selected.manifest.last_successful_sync_at.clone(),
            local_generation: selected.manifest.local_generation.clone(),
            remote_generation: selected.manifest.remote_generation.clone(),
            remote_observed_at: selected.manifest.remote_observed_at.clone(),
            observed_local_generation: selected.manifest.observed_local_generation.clone(),
            remote_relation: selected.manifest.remote_relation.clone(),
            pending_mutations: PendingMutations {
                operation_ids: pending.clone(),
            },
        })
        .unwrap();

        let mut first_script = vec![Ok(CommandOutput::success(format!(
            "{remote}\trefs/dolt/data\n"
        )))];
        first_script.extend(readonly_outputs(
            &selected.manifest.local_generation,
            &selected.manifest.source_commit,
        ));
        let mut first_runner = RecordingCommandRunner::scripted(first_script);
        let first = synchronize_with_clock(
            &mut first_runner,
            &location,
            &selected,
            &snapshot,
            &pin,
            "aarch64-apple-darwin",
            &FixedClock(second_observed_at),
        )
        .unwrap_err();
        assert_eq!(first.code(), "pending_mutations");
        first_runner.finish().unwrap();

        let activated = current_generation(&location).unwrap();
        assert_ne!(activated.name, selected.name);
        assert_eq!(
            activated.manifest.remote_relation,
            RemoteRelation::Equivalent
        );
        assert_eq!(
            activated.manifest.remote_generation.as_deref(),
            Some(remote.as_str())
        );
        assert_eq!(
            activated.manifest.remote_observed_at.as_deref(),
            Some(second_observed_at)
        );
        assert_eq!(
            activated.manifest.last_successful_sync_at.as_deref(),
            Some(first_observed_at)
        );
        assert_eq!(activated.manifest.pending_operation_ids, pending);
        let activated_snapshot = FencedSnapshot {
            documents: Vec::new(),
            freshness: classify(ObservationState {
                last_successful_sync_at: activated.manifest.last_successful_sync_at.clone(),
                local_generation: activated.manifest.local_generation.clone(),
                remote_generation: activated.manifest.remote_generation.clone(),
                remote_observed_at: activated.manifest.remote_observed_at.clone(),
                observed_local_generation: activated.manifest.observed_local_generation.clone(),
                remote_relation: activated.manifest.remote_relation.clone(),
                pending_mutations: PendingMutations {
                    operation_ids: activated.manifest.pending_operation_ids.clone(),
                },
            })
            .unwrap(),
        };
        assert_eq!(
            activated_snapshot.freshness.freshness,
            Freshness::Unpublished
        );

        let third_observed_at = "2026-09-02T00:02:00Z";
        let mut second_script = vec![Ok(CommandOutput::success(format!(
            "{remote}\trefs/dolt/data\n"
        )))];
        second_script.extend(readonly_outputs(
            &activated.manifest.local_generation,
            &activated.manifest.source_commit,
        ));
        let mut second_runner = RecordingCommandRunner::scripted(second_script);
        let second = synchronize_with_clock(
            &mut second_runner,
            &location,
            &activated,
            &activated_snapshot,
            &pin,
            "aarch64-apple-darwin",
            &FixedClock(third_observed_at),
        )
        .unwrap_err();
        assert_eq!(second.code(), "pending_mutations");
        second_runner.finish().unwrap();

        let reobserved = current_generation(&location).unwrap();
        assert_eq!(
            reobserved.manifest.remote_relation,
            RemoteRelation::Equivalent
        );
        assert_eq!(
            reobserved.manifest.last_successful_sync_at.as_deref(),
            Some(first_observed_at)
        );
        assert_eq!(
            reobserved.manifest.remote_observed_at.as_deref(),
            Some(third_observed_at)
        );
        assert_eq!(reobserved.manifest.pending_operation_ids, pending);
        assert_eq!(
            classify(ObservationState {
                last_successful_sync_at: reobserved.manifest.last_successful_sync_at,
                local_generation: reobserved.manifest.local_generation,
                remote_generation: reobserved.manifest.remote_generation,
                remote_observed_at: reobserved.manifest.remote_observed_at,
                observed_local_generation: reobserved.manifest.observed_local_generation,
                remote_relation: reobserved.manifest.remote_relation,
                pending_mutations: PendingMutations {
                    operation_ids: reobserved.manifest.pending_operation_ids,
                },
            })
            .unwrap()
            .freshness,
            Freshness::Unpublished
        );
    }

    #[test]
    fn sync_refuses_when_the_selected_generation_is_no_longer_current() {
        let (_root, location, selected, snapshot, pin) = sync_fixture();
        let alternate = location.generations_dir.join("generation-alternate");
        fs::create_dir(&alternate).unwrap();
        fs::write(
            alternate.join("state.json"),
            serde_json::to_vec(&selected.manifest).unwrap(),
        )
        .unwrap();
        fs::write(
            location.state_root.join("current"),
            "generation-alternate\n",
        )
        .unwrap();
        let mut runner = RecordingCommandRunner::default();

        let error = synchronize_with_clock(
            &mut runner,
            &location,
            &selected,
            &snapshot,
            &pin,
            "aarch64-apple-darwin",
            &FixedClock("2026-09-02T00:00:00Z"),
        )
        .unwrap_err();

        assert_eq!(error.code(), "store_changed");
        assert!(runner.commands().is_empty());
        assert!(
            fs::read_dir(&location.generations_dir)
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".staging-")),
            "the stale selected generation must refuse before creating a network staging root"
        );
    }
}
