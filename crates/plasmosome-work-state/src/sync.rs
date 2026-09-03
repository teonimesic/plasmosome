use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::command::{CommandOutput, CommandRunner, CommandSpec};
use crate::document::is_lower_hex_sha;
use crate::freshness::{Freshness, FreshnessEnvelope};
use crate::project::ProjectConfig;

/// A stable refusal raised while validating the online-sync command sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncError {
    code: &'static str,
}

impl SyncError {
    /// Returns the stable machine-readable refusal code.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for SyncError {}

fn refusal(code: &'static str) -> SyncError {
    SyncError { code }
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
    pub fn require_stable_observation(&self) -> Result<String, SyncError> {
        if self.phase != Phase::Complete {
            return Err(refusal("invalid_sync_command"));
        }
        match (self.first.as_ref(), self.second.as_ref()) {
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
        }
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

    fn valid(&self, command: &CommandSpec) -> bool {
        match self.phase {
            Phase::AwaitFirstObservation | Phase::AwaitSecondObservation => {
                self.observation_command(command)
            }
            Phase::AwaitInit => self.init_command(command),
            Phase::AwaitRemoteList => self.remote_list_command(command),
            Phase::AwaitCloneDecision | Phase::Complete | Phase::Terminal => false,
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
                self.phase = Phase::Complete;
                self.second = Some(observation);
            }
            Phase::AwaitCloneDecision | Phase::Complete | Phase::Terminal => {
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
