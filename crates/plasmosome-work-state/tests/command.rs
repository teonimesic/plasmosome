use std::path::PathBuf;

use plasmosome_work_state::command::CommandSpec;
use plasmosome_work_state::contract::isolated_environment;
use tempfile::tempdir;

#[test]
fn command_output_redacts_credentials_and_paths() {
    let command = CommandSpec {
        program: PathBuf::from("/private/secret/bd"),
        argv: vec!["push".into(), "https://token@example.test/repo".into()],
        cwd: None,
        environment: Default::default(),
        redacted_argv_positions: vec![1],
    };
    assert_eq!(command.display(), "/private/secret/bd push <redacted>");
}

#[test]
fn every_bd_child_has_the_isolated_environment() {
    let root = tempdir().unwrap();
    let environment = isolated_environment(root.path());
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "TMPDIR",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_NOSYSTEM",
        "BD_DISABLE_METRICS",
        "BD_DISABLE_EVENT_FLUSH",
        "BD_NON_INTERACTIVE",
        "CI",
        "GIT_TERMINAL_PROMPT",
    ] {
        assert!(environment.contains_key(key), "missing {key}");
    }
    assert!(!environment.values().any(|value| value.contains("secret")));
}
