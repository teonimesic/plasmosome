use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::command::{CommandRunner, CommandSpec, SystemCommandRunner};
use crate::pin::{PinManifest, VerifiedBeads};

const ISOLATED: &[(&str, &str)] = &[
    ("GIT_CONFIG_NOSYSTEM", "1"), ("BD_DISABLE_METRICS", "1"),
    ("BD_DISABLE_EVENT_FLUSH", "1"), ("BD_NON_INTERACTIVE", "1"),
    ("CI", "true"), ("GIT_TERMINAL_PROMPT", "0"),
];

#[derive(Clone, Debug)]
pub struct ContractRequest { pub case: String, pub archive: PathBuf, pub binary: PathBuf, pub github_remote: Option<String>, pub confirmation: Option<String> }

#[derive(Clone, Debug, Serialize)]
pub struct ContractResult { pub case: String, pub outcome: String, pub code: String, pub beads_version: String, pub clone_labels: Vec<String>, pub observed_base: Option<String>, pub final_generation: Option<String> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushFailure { StaleBase, Transport, Other }

impl ContractResult {
    fn refusal(case: &str, code: &str) -> Self { Self { case: case.to_owned(), outcome: "refused".to_owned(), code: code.to_owned(), beads_version: "1.1.2".to_owned(), clone_labels: Vec::new(), observed_base: None, final_generation: None } }
}

pub fn isolated_environment(root: &std::path::Path) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for (name, value) in ISOLATED { environment.insert((*name).to_owned(), (*value).to_owned()); }
    for name in ["HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "TMPDIR", "GIT_CONFIG_GLOBAL"] {
        environment.insert(name.to_owned(), root.join(name.to_ascii_lowercase()).display().to_string());
    }
    if let Some(path) = std::env::var_os("PATH") { environment.insert("PATH".to_owned(), path.to_string_lossy().into_owned()); }
    environment
}

pub fn run_contract(request: &ContractRequest) -> Result<ContractResult, ContractResult> {
    if matches!(request.case.as_str(), "github" | "all") && request.github_remote.is_none() { return Err(ContractResult::refusal(&request.case, "github_fixture_required")); }
    let manifest = PinManifest::load("tools/work-state-beads-1.1.2.toml").map_err(|error| ContractResult::refusal(&request.case, error.code()))?;
    let root = tempfile::tempdir().map_err(|_| ContractResult::refusal(&request.case, "cutover_blocked"))?;
    let mut runner = SystemCommandRunner;
    VerifiedBeads::verify(&manifest, host_target(), &request.archive, &request.binary, &mut runner).map_err(|error| ContractResult::refusal(&request.case, error.code()))?;
    if request.case == "github" || request.case == "all" || remote_case(&request.case) {
        let remote = request.github_remote.as_deref().ok_or_else(|| ContractResult::refusal(&request.case, "github_fixture_required"))?;
        if request.confirmation.as_deref() != Some("refs/dolt/data") { return Err(ContractResult::refusal(&request.case, "github_fixture_invalid")); }
        let mut runner = SystemCommandRunner;
        fixture_preflight(remote, "refs/dolt/data", &mut runner).map_err(|code| ContractResult::refusal(&request.case, code))?;
        return Err(ContractResult::refusal(&request.case, "cutover_blocked"));
    }
    if request.case == "stealth-init" || request.case == "hermetic" {
        run_stealth_init(&request.binary, root.path()).map_err(|code| ContractResult::refusal(&request.case, code))?;
    }
    Ok(ContractResult { case: request.case.clone(), outcome: "passed".to_owned(), code: "ok".to_owned(), beads_version: manifest.version, clone_labels: Vec::new(), observed_base: None, final_generation: None })
}

fn remote_case(case: &str) -> bool { matches!(case, "stale-base-fence" | "push-conflict-recovery" | "transport-retries") }
fn valid_fixture(remote: &str) -> bool {
    let no_credentials = !remote.contains('@') && !remote.contains("//:");
    let github = remote.starts_with("https://github.com/") || remote.starts_with("git@github.com:") || remote.starts_with("ssh://git@github.com/");
    let name = remote.trim_end_matches('/').rsplit('/').next().unwrap_or("").trim_end_matches(".git");
    github && no_credentials && name.starts_with("plasmosome-work-state-fixture")
}

pub fn fixture_preflight<R: CommandRunner>(remote: &str, reference: &str, runner: &mut R) -> Result<(), &'static str> {
    if !valid_fixture(remote) || reference != "refs/dolt/data" { return Err("github_fixture_invalid"); }
    let output = runner.run(CommandSpec { program: PathBuf::from("git"), argv: vec!["ls-remote".into(), remote.to_owned(), reference.to_owned()], cwd: None, environment: BTreeMap::new(), redacted_argv_positions: Vec::new() }).map_err(|_| "github_fixture_invalid")?;
    if output.status != 0 { return Err("github_fixture_invalid"); }
    if !output.stdout.trim().is_empty() { return Err("github_fixture_not_empty"); }
    Ok(())
}

pub fn classify_push(stderr: &str) -> PushFailure {
    if stderr.contains("non-fast-forward") || stderr.contains("stale") { PushFailure::StaleBase }
    else if stderr.contains("connection") || stderr.contains("timed out") || stderr.contains("network") { PushFailure::Transport }
    else { PushFailure::Other }
}

pub fn clone_paths(root: impl AsRef<std::path::Path>) -> (PathBuf, PathBuf) { (root.as_ref().join("clone-a"), root.as_ref().join("clone-b")) }

pub fn cleanup_command(remote: &str, generation: &str) -> CommandSpec {
    CommandSpec { program: PathBuf::from("git"), argv: vec!["push".into(), remote.to_owned(), format!("--force-with-lease=refs/dolt/data:{generation}"), ":refs/dolt/data".into()], cwd: None, environment: BTreeMap::new(), redacted_argv_positions: Vec::new() }
}

pub fn stealth_init_command(binary: impl Into<PathBuf>, repository: impl Into<PathBuf>, root: impl AsRef<std::path::Path>) -> CommandSpec {
    let repository = repository.into();
    let mut environment = isolated_environment(root.as_ref());
    environment.insert("PWD".to_owned(), repository.display().to_string());
    CommandSpec { program: binary.into(), argv: vec!["--sandbox".into(), "init".into(), "--stealth".into(), "--skip-agents".into(), "--skip-hooks".into(), "--non-interactive".into()], cwd: Some(repository), environment, redacted_argv_positions: Vec::new() }
}

fn run_stealth_init(binary: &std::path::Path, root: &std::path::Path) -> Result<(), &'static str> {
    let repository = root.join("stealth-repository");
    std::fs::create_dir(&repository).map_err(|_| "cutover_blocked")?;
    for name in ["home", "xdg_config_home", "xdg_cache_home", "xdg_data_home", "tmpdir"] {
        std::fs::create_dir(root.join(name)).map_err(|_| "cutover_blocked")?;
    }
    let environment = isolated_environment(root);
    let mut runner = SystemCommandRunner;
    for argv in [vec!["init".to_owned()], vec!["config".to_owned(), "user.email".to_owned(), "fixture@example.invalid".to_owned()], vec!["config".to_owned(), "user.name".to_owned(), "fixture".to_owned()]] {
        let output = runner.run(CommandSpec { program: PathBuf::from("git"), argv, cwd: Some(repository.clone()), environment: environment.clone(), redacted_argv_positions: Vec::new() }).map_err(|_| "cutover_blocked")?;
        if output.status != 0 { return Err("cutover_blocked"); }
    }
    let output = runner.run(stealth_init_command(binary.to_path_buf(), repository, root)).map_err(|_| "cutover_blocked")?;
    if output.status != 0 { return Err("cutover_blocked"); }
    let output = runner.run(CommandSpec { program: PathBuf::from("git"), argv: vec!["config".into(), "dolt.auto-push".into(), "false".into()], cwd: Some(root.join("stealth-repository")), environment, redacted_argv_positions: Vec::new() }).map_err(|_| "cutover_blocked")?;
    if output.status == 0 { Ok(()) } else { Err("cutover_blocked") }
}

fn host_target() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) { "aarch64-apple-darwin" }
    else if cfg!(all(target_os = "linux", target_arch = "x86_64")) { "x86_64-unknown-linux-gnu" }
    else { "unsupported" }
}
