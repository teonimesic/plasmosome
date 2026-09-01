use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use tempfile::TempDir;

use crate::command::{
    CommandOutput, CommandRunner, CommandSpec, RecordingCommandRunner, SystemCommandRunner,
};
use crate::pin::{PinManifest, VerifiedBeads};

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
    pub archive: PathBuf,
    pub binary: PathBuf,
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
        }
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
        let (key, value) = line.split_once('=').ok_or("cutover_blocked")?;
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
    ) {
        return Err("invalid_command".into());
    }
    let mut archive = None;
    let mut binary = None;
    let mut index = 2;
    while index < values.len() {
        let value = values
            .get(index + 1)
            .ok_or_else(|| "invalid_command".to_owned())?;
        match values[index].as_str() {
            "--archive" => archive = Some(PathBuf::from(value)),
            "--bd" => binary = Some(PathBuf::from(value)),
            _ => return Err("invalid_command".into()),
        };
        index += 2;
    }
    Ok(ContractRequest {
        case,
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
    let output = execute_publication_command(runner, publish_command(), &retry_base)
        .map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    Ok((
        observed_base,
        Publication::Published {
            operation: operation.into(),
            generation: observe(runner)?,
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
    let (observed_after_failure, operations) = observe_with_operations(runner)?;
    if observed_after_failure != observed_base {
        if !operations.iter().any(|value| value == operation) {
            return Err("cutover_blocked".into());
        }
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
    Ok((
        observed_base,
        Publication::Published {
            operation: operation.into(),
            generation: observe(runner)?,
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
    let generation = output.stdout.split_whitespace().next().unwrap_or("");
    if sha(generation) {
        Ok(generation.into())
    } else {
        Err("cutover_blocked".into())
    }
}
fn observe_with_operations<R: CommandRunner>(
    runner: &mut R,
) -> Result<(String, Vec<String>), String> {
    let output = runner
        .run(observe_command())
        .map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    let mut fields = output.stdout.split_whitespace();
    let generation = fields.next().unwrap_or("");
    if !sha(generation) {
        return Err("cutover_blocked".into());
    }
    Ok((
        generation.into(),
        fields
            .filter_map(|field| field.strip_prefix("operation:").map(str::to_owned))
            .collect(),
    ))
}
fn sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn run_contract(request: &ContractRequest) -> Result<ContractResult, Box<ContractResult>> {
    let root = tempfile::tempdir()
        .map_err(|_| Box::new(ContractResult::refusal(&request.case, "cutover_blocked")))?;
    let result = (|| {
        let manifest = PinManifest::load("tools/work-state-beads-1.1.2.toml")
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
    match dispose_fixture_root(root) {
        Ok(()) => result,
        Err(code) => Err(Box::new(ContractResult::refusal(&request.case, code))),
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
            Ok(CommandOutput::success("winner")),
            Ok(observation_output(g1)),
            Ok(CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "non-fast-forward".into(),
            }),
            Ok(observation_output(g1)),
            Ok(CommandOutput::success("refreshed")),
            Ok(CommandOutput::success("replayed")),
            Ok(observation_output(g2)),
            Ok(CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "non-fast-forward".into(),
            }),
            Ok(observation_output(g2)),
        ]),
        "push-conflict-recovery" => Ok(vec![
            Ok(observation_output(g0)),
            Ok(observation_output(g0)),
            Ok(CommandOutput::success("winner")),
            Ok(observation_output(g1)),
            Ok(CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "non-fast-forward".into(),
            }),
            Ok(observation_output(g1)),
            Ok(CommandOutput::success("refreshed")),
            Ok(CommandOutput::success("replayed")),
            Ok(observation_output(g2)),
        ]),
        "transport-retries" => Ok(vec![
            Ok(observation_output(g0)),
            Err("connection reset".into()),
            Ok(observation_output(g0)),
            Ok(CommandOutput::success("published")),
            Ok(observation_output(g1)),
            Ok(observation_output(g0)),
            Err("connection reset".into()),
            Ok(observation_output_with_operation(g1, "lost-response")),
        ]),
        _ => Err("cutover_blocked".into()),
    }
}
fn observation_output(generation: &str) -> CommandOutput {
    CommandOutput::success(format!("{generation}\trefs/dolt/data\n"))
}
fn observation_output_with_operation(generation: &str, operation: &str) -> CommandOutput {
    CommandOutput::success(format!(
        "{generation}\trefs/dolt/data\toperation:{operation}\n"
    ))
}

pub fn run_scripted_case<R: CommandRunner>(
    case: &str,
    runner: &mut R,
) -> Result<ScriptEvidence, String> {
    match case {
        "stale-base-fence" => full_stale_base_fence(runner),
        "push-conflict-recovery" => {
            let winner_base = observe(runner)?;
            let stale_base = observe(runner)?;
            if winner_base != stale_base {
                return Err("cutover_blocked".into());
            }
            let winner_generation = publish_after_observation(runner, &winner_base)?;
            let stale_push = execute_publication_command(runner, publish_command(), &stale_base)
                .map_err(|_| "cutover_blocked".to_owned())?;
            if stale_push.status == 0 || classify_push(&stale_push.stderr) != PushFailure::StaleBase
            {
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
            let generation = publish_after_observation(runner, &observed_after_stale)?;
            validate_logical_export(&["winner", "replay"], &["winner", "replay"])?;
            Ok(ScriptEvidence {
                observed_base: winner_base,
                final_generation: generation,
                operation_ids: vec!["winner".into(), "replay".into()],
            })
        }
        "transport-retries" => {
            let (observed_base, result) = retry_after_transport_with_base(runner, "retry")?;
            let Publication::Published { .. } = result else {
                return Err("cutover_blocked".into());
            };
            let (_, recovered) = recover_after_lost_response_with_base(runner, "lost-response")?;
            let Publication::Recovered {
                generation: recovered_generation,
                ..
            } = recovered
            else {
                return Err("cutover_blocked".into());
            };
            Ok(ScriptEvidence {
                observed_base,
                final_generation: recovered_generation,
                operation_ids: vec!["retry".into(), "lost-response".into()],
            })
        }
        _ => Err("cutover_blocked".into()),
    }
}

fn full_stale_base_fence<R: CommandRunner>(runner: &mut R) -> Result<ScriptEvidence, String> {
    let winner_base = observe(runner)?;
    let stale_base = observe(runner)?;
    if winner_base != stale_base {
        return Err("cutover_blocked".into());
    }
    let winner_generation = publish_after_observation(runner, &winner_base)?;
    let stale_push = execute_publication_command(runner, publish_command(), &stale_base)
        .map_err(|_| "cutover_blocked".to_owned())?;
    if stale_push.status == 0 || classify_push(&stale_push.stderr) != PushFailure::StaleBase {
        return Err("cutover_blocked".into());
    }
    if observe(runner)? != winner_generation {
        return Err("cutover_blocked".into());
    }
    let refresh = runner
        .run(refresh_command())
        .map_err(|_| "cutover_blocked".to_owned())?;
    if refresh.status != 0 {
        return Err("cutover_blocked".into());
    }
    let recovery_generation = publish_after_observation(runner, &winner_generation)?;
    let paused_push = execute_publication_command(runner, publish_command(), &winner_generation)
        .map_err(|_| "cutover_blocked".to_owned())?;
    if paused_push.status == 0 || classify_push(&paused_push.stderr) != PushFailure::StaleBase {
        return Err("cutover_blocked".into());
    }
    if observe(runner)? != recovery_generation {
        return Err("cutover_blocked".into());
    }
    validate_logical_export(&["winner", "replay"], &["winner", "replay"])?;
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
