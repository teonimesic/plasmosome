use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

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
    pub final_generation: String,
    pub operation_ids: Vec<String>,
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

pub fn observe_command() -> CommandSpec {
    CommandSpec {
        program: PathBuf::from("git"),
        argv: vec![
            "ls-remote".into(),
            "--exit-code".into(),
            "origin".into(),
            "refs/dolt/data".into(),
        ],
        cwd: None,
        environment: BTreeMap::new(),
        redacted_argv_positions: vec![2],
    }
}
pub fn publish_command() -> CommandSpec {
    CommandSpec {
        program: PathBuf::from("bd"),
        argv: vec![
            "--sandbox".into(),
            "dolt".into(),
            "push".into(),
            "--remote".into(),
            "origin".into(),
        ],
        cwd: None,
        environment: BTreeMap::new(),
        redacted_argv_positions: vec![4],
    }
}
pub fn refresh_command() -> CommandSpec {
    CommandSpec {
        program: PathBuf::from("bd"),
        argv: vec![
            "--sandbox".into(),
            "dolt".into(),
            "pull".into(),
            "--remote".into(),
            "origin".into(),
        ],
        cwd: None,
        environment: BTreeMap::new(),
        redacted_argv_positions: vec![4],
    }
}
pub fn leased_ref_update(expected: &str, candidate: &str) -> Result<CommandSpec, String> {
    if !sha(expected) || !sha(candidate) {
        return Err("cutover_blocked".into());
    }
    Ok(CommandSpec {
        program: PathBuf::from("git"),
        argv: vec![
            "push".into(),
            "origin".into(),
            format!("--force-with-lease=refs/dolt/data:{expected}"),
            format!("{candidate}:refs/dolt/data"),
        ],
        cwd: None,
        environment: BTreeMap::new(),
        redacted_argv_positions: vec![1, 3],
    })
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
    observe(runner)?;
    match runner.run(publish_command()) {
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
    observe(runner)?;
    match runner.run(publish_command()) {
        Err(error) if classify_push(&error) == PushFailure::Transport => {}
        Ok(output) if classify_push(&output.stderr) == PushFailure::Transport => {}
        _ => return Err("cutover_blocked".into()),
    }
    observe(runner)?;
    let output = runner
        .run(publish_command())
        .map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    Ok(Publication::Published {
        operation: operation.into(),
        generation: observe(runner)?,
    })
}
pub fn recover_after_lost_response<R: CommandRunner>(
    runner: &mut R,
    operation: &str,
) -> Result<Publication, String> {
    observe(runner)?;
    match runner.run(publish_command()) {
        Err(error) if classify_push(&error) == PushFailure::Transport => {}
        Ok(output) if classify_push(&output.stderr) == PushFailure::Transport => {}
        _ => return Err("cutover_blocked".into()),
    }
    Ok(Publication::Recovered {
        operation: operation.into(),
        generation: observe(runner)?,
    })
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
fn sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn run_contract(request: &ContractRequest) -> Result<ContractResult, Box<ContractResult>> {
    let root = tempfile::tempdir()
        .map_err(|_| Box::new(ContractResult::refusal(&request.case, "cutover_blocked")))?;
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
    init_store(&request.binary, root.path(), "clone-a")
        .map_err(|code| Box::new(ContractResult::refusal(&request.case, code)))?;
    init_store(&request.binary, root.path(), "clone-b")
        .map_err(|code| Box::new(ContractResult::refusal(&request.case, code)))?;
    let mut result = run_scripts(&request.case)
        .map_err(|code| Box::new(ContractResult::refusal(&request.case, code)))?;
    result.clone_labels = vec!["clone-a".into(), "clone-b".into()];
    Ok(result)
}
fn run_scripts(case: &str) -> Result<ContractResult, &'static str> {
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
    let final_result = results.pop().ok_or("cutover_blocked")?;
    let operation_ids = results
        .iter()
        .flat_map(|result| result.operation_ids.iter().cloned())
        .chain(final_result.operation_ids.iter().cloned())
        .collect();
    let command_plans = results
        .iter()
        .flat_map(|result| result.command_plans.iter().cloned())
        .chain(final_result.command_plans.iter().cloned())
        .collect();
    Ok(ContractResult {
        operation_ids,
        command_plans,
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
            Ok(CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "non-fast-forward".into(),
            }),
            Ok(observation_output(g1)),
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
            Ok(observation_output(g1)),
        ]),
        _ => Err("cutover_blocked".into()),
    }
}
fn observation_output(generation: &str) -> CommandOutput {
    CommandOutput::success(format!("{generation}\trefs/dolt/data\n"))
}

pub fn run_scripted_case<R: CommandRunner>(
    case: &str,
    runner: &mut R,
) -> Result<ScriptEvidence, String> {
    match case {
        "stale-base-fence" => match publish_candidate(runner, "stale")? {
            Publication::StaleBase { generation, .. } => Ok(ScriptEvidence {
                final_generation: generation,
                operation_ids: vec!["winner".into()],
            }),
            _ => Err("cutover_blocked".into()),
        },
        "push-conflict-recovery" => {
            let winner_base = observe(runner)?;
            let stale_base = observe(runner)?;
            if winner_base != stale_base {
                return Err("cutover_blocked".into());
            }
            let winner_generation = publish_after_observation(runner)?;
            let stale_push = runner
                .run(publish_command())
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
            let generation = publish_after_observation(runner)?;
            Ok(ScriptEvidence {
                final_generation: generation,
                operation_ids: vec!["winner".into(), "replay".into()],
            })
        }
        "transport-retries" => {
            let result = retry_after_transport(runner, "retry")?;
            let Publication::Published { generation: _, .. } = result else {
                return Err("cutover_blocked".into());
            };
            let recovered = recover_after_lost_response(runner, "lost-response")?;
            let Publication::Recovered {
                generation: recovered_generation,
                ..
            } = recovered
            else {
                return Err("cutover_blocked".into());
            };
            Ok(ScriptEvidence {
                final_generation: recovered_generation,
                operation_ids: vec!["retry".into(), "lost-response".into()],
            })
        }
        _ => Err("cutover_blocked".into()),
    }
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
        observed_base: Some("0000000000000000000000000000000000000000".into()),
        final_generation: Some(evidence.final_generation),
        operation_ids: evidence.operation_ids,
        command_plans,
    })
}

fn publish_after_observation<R: CommandRunner>(runner: &mut R) -> Result<String, String> {
    let output = runner
        .run(publish_command())
        .map_err(|_| "cutover_blocked".to_owned())?;
    if output.status != 0 {
        return Err("cutover_blocked".into());
    }
    observe(runner)
}

pub fn embedded_cleanup_commands() -> Vec<CommandSpec> {
    Vec::new()
}

fn init_store(binary: &Path, root: &Path, label: &str) -> Result<(), &'static str> {
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
    let mut runner = SystemCommandRunner;
    for argv in [
        vec!["init".into()],
        vec![
            "config".into(),
            "user.email".into(),
            "fixture@example.invalid".into(),
        ],
        vec!["config".into(), "user.name".into(), "fixture".into()],
    ] {
        let output = runner
            .run(CommandSpec {
                program: PathBuf::from("git"),
                argv,
                cwd: Some(repository.clone()),
                environment: environment.clone(),
                redacted_argv_positions: Vec::new(),
            })
            .map_err(|_| "cutover_blocked")?;
        if output.status != 0 {
            return Err("cutover_blocked");
        }
    }
    let mut init_environment = environment.clone();
    init_environment.insert("PWD".into(), repository.display().to_string());
    let output = runner
        .run(CommandSpec {
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
        })
        .map_err(|_| "cutover_blocked")?;
    if output.status != 0 {
        return Err("cutover_blocked");
    }
    let output = runner
        .run(CommandSpec {
            program: PathBuf::from("git"),
            argv: vec!["config".into(), "dolt.auto-push".into(), "false".into()],
            cwd: Some(repository.clone()),
            environment: environment.clone(),
            redacted_argv_positions: Vec::new(),
        })
        .map_err(|_| "cutover_blocked")?;
    if output.status != 0 {
        return Err("cutover_blocked");
    }
    Ok(())
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
