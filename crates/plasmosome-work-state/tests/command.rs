use std::fs;
use std::path::PathBuf;

use plasmosome_work_state::command::{CommandRunner, CommandSpec, SystemCommandRunner};
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
    assert_eq!(command.display(), "bd push <redacted>");
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
    assert!(
        environment
            .iter()
            .filter(|(key, _)| key.as_str() != "PATH")
            .all(|(_, value)| !value.contains("secret"))
    );
}

#[cfg(unix)]
#[test]
fn system_runner_refuses_non_utf8_output_instead_of_replacing_it() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    let program = root.path().join("invalid-output");
    fs::write(&program, b"#!/bin/sh\nprintf '\\377'\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();

    let mut runner = SystemCommandRunner;
    let result = runner.run(CommandSpec {
        program,
        argv: Vec::new(),
        cwd: None,
        environment: Default::default(),
        redacted_argv_positions: Vec::new(),
    });

    assert_eq!(result, Err("command_output_not_utf8".into()));
}
