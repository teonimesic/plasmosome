use std::collections::BTreeMap;
use std::path::PathBuf;

use plasmosome_work_state::command::{
    CommandOutput, CommandRunner, CommandSpec, RecordingCommandRunner,
};
use plasmosome_work_state::freshness::{Freshness, FreshnessEnvelope, PendingMutationEnvelope};
use plasmosome_work_state::project::compiled_project_config;
use plasmosome_work_state::sync::{
    RemoteObservation, SyncCommandBinding, SyncCommandRunner, SyncResult, render_sync_human,
};
use tempfile::tempdir;

fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("PATH".into(), "/test/bin".into()),
        ("HOME".into(), "/test/home".into()),
        ("XDG_CONFIG_HOME".into(), "/test/xdg-config".into()),
        ("XDG_CACHE_HOME".into(), "/test/xdg-cache".into()),
        ("XDG_DATA_HOME".into(), "/test/xdg-data".into()),
        ("TMPDIR".into(), "/test/tmp".into()),
        ("GIT_CONFIG_GLOBAL".into(), "/test/git-config".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_NO_LAZY_FETCH".into(), "1".into()),
        ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
        ("BD_DISABLE_METRICS".into(), "1".into()),
        ("BD_DISABLE_EVENT_FLUSH".into(), "1".into()),
        ("BD_NON_INTERACTIVE".into(), "1".into()),
        ("CI".into(), "true".into()),
    ])
}

fn binding(root: &std::path::Path) -> SyncCommandBinding {
    SyncCommandBinding::new(
        compiled_project_config().unwrap(),
        root.to_path_buf(),
        root.join("repository"),
        root.join("bd"),
        environment(),
    )
    .expect("test binding is valid")
}

fn observation(root: &std::path::Path) -> CommandSpec {
    CommandSpec {
        program: PathBuf::from("git"),
        argv: vec![
            "ls-remote".into(),
            "--exit-code".into(),
            "https://github.com/teonimesic/plasmosome.git".into(),
            "refs/dolt/data".into(),
        ],
        cwd: Some(root.to_path_buf()),
        environment: environment(),
        redacted_argv_positions: vec![2],
    }
}

fn init(root: &std::path::Path) -> CommandSpec {
    CommandSpec {
        program: root.join("bd"),
        argv: vec![
            "--sandbox".into(),
            "init".into(),
            "--remote".into(),
            "git+https://github.com/teonimesic/plasmosome.git".into(),
            "--stealth".into(),
            "--skip-agents".into(),
            "--skip-hooks".into(),
            "--non-interactive".into(),
        ],
        cwd: Some(root.join("repository")),
        environment: environment(),
        redacted_argv_positions: vec![3],
    }
}

fn remote_list(root: &std::path::Path) -> CommandSpec {
    CommandSpec {
        program: root.join("bd"),
        argv: vec![
            "--sandbox".into(),
            "--json".into(),
            "dolt".into(),
            "remote".into(),
            "list".into(),
        ],
        cwd: Some(root.join("repository")),
        environment: environment(),
        redacted_argv_positions: Vec::new(),
    }
}

#[test]
fn sync_runner_binds_every_command_before_dispatch() {
    let root = tempdir().unwrap();
    let sha = "a".repeat(40);
    let remote_json = r#"[{"name":"origin","url":"git+https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok"}]"#;
    let mut inner = RecordingCommandRunner::scripted(vec![
        Ok(CommandOutput::success(format!("{sha}\trefs/dolt/data\n"))),
        Ok(CommandOutput::success("")),
        Ok(CommandOutput::success(remote_json)),
        Ok(CommandOutput::success(format!("{sha}\trefs/dolt/data\n"))),
    ]);
    {
        let mut runner = SyncCommandRunner::new(&mut inner, binding(root.path()));
        let wrong_program = CommandSpec {
            program: root.path().join("wrong-git"),
            ..observation(root.path())
        };
        assert_eq!(
            runner.run(wrong_program).unwrap_err(),
            "invalid_sync_command"
        );
        let wrong_url = CommandSpec {
            argv: vec![
                "ls-remote".into(),
                "--exit-code".into(),
                "https://github.com/other/project.git".into(),
                "refs/dolt/data".into(),
            ],
            ..observation(root.path())
        };
        assert_eq!(runner.run(wrong_url).unwrap_err(), "invalid_sync_command");
        let wrong_ref = CommandSpec {
            argv: vec![
                "ls-remote".into(),
                "--exit-code".into(),
                "https://github.com/teonimesic/plasmosome.git".into(),
                "refs/heads/main".into(),
            ],
            ..observation(root.path())
        };
        assert_eq!(runner.run(wrong_ref).unwrap_err(), "invalid_sync_command");
        let wrong_cwd = CommandSpec {
            cwd: Some(root.path().join("repository")),
            ..observation(root.path())
        };
        assert_eq!(runner.run(wrong_cwd).unwrap_err(), "invalid_sync_command");
        let wrong_environment = CommandSpec {
            environment: BTreeMap::new(),
            ..observation(root.path())
        };
        assert_eq!(
            runner.run(wrong_environment).unwrap_err(),
            "invalid_sync_command"
        );

        runner.run(observation(root.path())).unwrap();
        assert_eq!(runner.first_observation().as_deref(), Some(sha.as_str()));
        assert_eq!(
            runner.run(init(root.path())).unwrap_err(),
            "invalid_sync_command",
            "an explicit clone decision is required after R0"
        );
        runner.authorize_fresh_clone(&[]).unwrap();
        let wrong_staged_binary = CommandSpec {
            program: root.path().join("other/bd"),
            ..init(root.path())
        };
        assert_eq!(
            runner.run(wrong_staged_binary).unwrap_err(),
            "invalid_sync_command"
        );
        runner.run(init(root.path())).unwrap();
        runner.run(remote_list(root.path())).unwrap();
        runner.run(observation(root.path())).unwrap();
        assert_eq!(runner.second_observation().as_deref(), Some(sha.as_str()));
        assert_eq!(runner.require_stable_observation().unwrap(), sha);

        let replay = observation(root.path());
        assert_eq!(runner.run(replay).unwrap_err(), "invalid_sync_command");
    }
    assert_eq!(inner.commands().len(), 4);
    inner.finish().unwrap();
}

#[test]
fn sync_runner_rejects_every_remote_write_shape() {
    let root = tempdir().unwrap();
    let mut inner = RecordingCommandRunner::default();
    {
        let mut runner = SyncCommandRunner::new(&mut inner, binding(root.path()));
        let base = CommandSpec {
            program: root.path().join("bd"),
            argv: Vec::new(),
            cwd: Some(root.path().join("repository")),
            environment: environment(),
            redacted_argv_positions: Vec::new(),
        };
        for forbidden in [
            CommandSpec {
                argv: vec![
                    "--sandbox".into(),
                    "dolt".into(),
                    "remote".into(),
                    "add".into(),
                    "origin".into(),
                    "git+https://github.com/teonimesic/plasmosome.git".into(),
                ],
                redacted_argv_positions: vec![5],
                ..base.clone()
            },
            CommandSpec {
                argv: vec![
                    "--sandbox".into(),
                    "dolt".into(),
                    "pull".into(),
                    "--remote".into(),
                    "origin".into(),
                ],
                ..base.clone()
            },
            CommandSpec {
                argv: vec!["--sandbox".into(), "bootstrap".into()],
                ..base.clone()
            },
            CommandSpec {
                program: PathBuf::from("git"),
                argv: vec!["push".into(), "origin".into(), "refs/dolt/data".into()],
                cwd: Some(root.path().to_path_buf()),
                redacted_argv_positions: vec![1],
                ..base.clone()
            },
            CommandSpec {
                program: PathBuf::from("git"),
                argv: vec!["fetch".into(), "--force".into(), "origin".into()],
                cwd: Some(root.path().to_path_buf()),
                redacted_argv_positions: vec![2],
                ..base.clone()
            },
            CommandSpec {
                program: PathBuf::from("git"),
                argv: vec![
                    "update-ref".into(),
                    "refs/dolt/data".into(),
                    "deadbeef".into(),
                ],
                cwd: Some(root.path().to_path_buf()),
                ..base.clone()
            },
            CommandSpec {
                program: PathBuf::from("sh"),
                argv: vec!["-c".into(), "git push".into()],
                cwd: Some(root.path().to_path_buf()),
                ..base
            },
        ] {
            assert_eq!(
                runner.run(forbidden).unwrap_err(),
                "invalid_sync_command",
                "remote-write command must not reach the runner"
            );
        }
    }
    assert!(inner.commands().is_empty());
    inner.finish().unwrap();
}

#[test]
fn remote_observation_is_one_exact_lowercase_data_ref() {
    let root = tempdir().unwrap();
    let valid = CommandOutput::success(format!("{}\trefs/dolt/data\n", "a".repeat(40)));
    for malformed in [
        format!("{}\trefs/dolt/data\n", "A".repeat(40)),
        format!("{}\trefs/heads/main\n", "a".repeat(40)),
        format!(
            "{}\trefs/dolt/data\n{}\trefs/dolt/data\n",
            "a".repeat(40),
            "b".repeat(40)
        ),
        "\n".to_owned(),
        format!("{} refs/dolt/data\n", "a".repeat(40)),
    ] {
        let mut inner = RecordingCommandRunner::scripted(vec![
            Ok(CommandOutput::success(malformed)),
            Ok(valid.clone()),
        ]);
        {
            let mut runner = SyncCommandRunner::new(&mut inner, binding(root.path()));
            assert_eq!(
                runner.run(observation(root.path())).unwrap_err(),
                "invalid_remote_observation"
            );
            assert_eq!(
                runner.run(observation(root.path())).unwrap_err(),
                "invalid_sync_command",
                "a malformed successful observation is terminal"
            );
        }
        assert_eq!(
            inner.commands().len(),
            1,
            "the terminal rejection must preclude a second dispatch"
        );
    }
}

#[test]
fn remote_no_match_and_transport_are_distinct() {
    let root = tempdir().unwrap();
    for (output, expected) in [
        (
            CommandOutput {
                status: 2,
                stdout: String::new(),
                stderr: "missing ref".into(),
            },
            RemoteObservation::NoMatch,
        ),
        (
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "transport refused".into(),
            },
            RemoteObservation::Transport,
        ),
        (
            CommandOutput {
                status: 2,
                stdout: "unexpected output\n".into(),
                stderr: String::new(),
            },
            RemoteObservation::Transport,
        ),
    ] {
        let mut inner = RecordingCommandRunner::with_output(output);
        {
            let mut runner = SyncCommandRunner::new(&mut inner, binding(root.path()));
            runner.run(observation(root.path())).unwrap();
            assert_eq!(runner.first_outcome(), Some(&expected));
            assert_eq!(
                runner.run(init(root.path())).unwrap_err(),
                "invalid_sync_command",
                "no-match and transport terminally stop before every Beads remote command"
            );
        }
        assert_eq!(inner.commands().len(), 1);
        inner.finish().unwrap();
    }
}

#[test]
fn remote_list_accepts_only_the_exact_canonical_git_transport_binding() {
    let root = tempdir().unwrap();
    let sha = "a".repeat(40);
    let valid_observation = CommandOutput::success(format!("{sha}\trefs/dolt/data\n"));
    let valid_list = CommandOutput::success(
        r#"[{"name":"origin","url":"git+https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok"}]"#,
    );
    for invalid in [
        r#"[{"name":"origin","url":"https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok"}]"#,
        r#"[{"name":"origin","url":"git+https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok"},{"name":"other","url":"git+https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok"}]"#,
        r#"[{"name":"origin","url":"git+https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok","extra":"field"}]"#,
    ] {
        let mut inner = RecordingCommandRunner::scripted(vec![
            Ok(valid_observation.clone()),
            Ok(CommandOutput::success("")),
            Ok(CommandOutput::success(invalid)),
            Ok(valid_list.clone()),
        ]);
        {
            let mut runner = SyncCommandRunner::new(&mut inner, binding(root.path()));
            runner.run(observation(root.path())).unwrap();
            runner.authorize_fresh_clone(&[]).unwrap();
            runner.run(init(root.path())).unwrap();
            assert_eq!(
                runner.run(remote_list(root.path())).unwrap_err(),
                "remote_configuration_mismatch"
            );
            assert_eq!(
                runner.run(remote_list(root.path())).unwrap_err(),
                "invalid_sync_command",
                "bad remote configuration is terminal"
            );
        }
        assert_eq!(inner.commands().len(), 3);
    }

    let mut inner = RecordingCommandRunner::scripted(vec![
        Ok(valid_observation),
        Ok(CommandOutput::success("")),
        Ok(valid_list),
    ]);
    {
        let mut runner = SyncCommandRunner::new(&mut inner, binding(root.path()));
        runner.run(observation(root.path())).unwrap();
        runner.authorize_fresh_clone(&[]).unwrap();
        runner.run(init(root.path())).unwrap();
        runner.run(remote_list(root.path())).unwrap();
    }
    inner.finish().unwrap();
}

#[test]
fn pending_mutations_are_observed_but_never_cloned_over() {
    let root = tempdir().unwrap();
    let mut inner = RecordingCommandRunner::with_output(CommandOutput::success(format!(
        "{}\trefs/dolt/data\n",
        "a".repeat(40)
    )));
    {
        let mut runner = SyncCommandRunner::new(&mut inner, binding(root.path()));
        runner.run(observation(root.path())).unwrap();
        assert_eq!(
            runner
                .authorize_fresh_clone(&["pending-1".to_owned()])
                .unwrap_err()
                .code(),
            "pending_mutations"
        );
        assert_eq!(
            runner.run(init(root.path())).unwrap_err(),
            "invalid_sync_command"
        );
    }
    assert_eq!(inner.commands().len(), 1);
    inner.finish().unwrap();
}

#[test]
fn moving_remote_never_activates_the_cloned_candidate() {
    let root = tempdir().unwrap();
    let mut inner = RecordingCommandRunner::scripted(vec![
        Ok(CommandOutput::success(format!(
            "{}\trefs/dolt/data\n",
            "a".repeat(40)
        ))),
        Ok(CommandOutput::success("")),
        Ok(CommandOutput::success(
            r#"[{"name":"origin","url":"git+https://github.com/teonimesic/plasmosome.git","sql_url":"git+https://github.com/teonimesic/plasmosome.git","status":"ok"}]"#,
        )),
        Ok(CommandOutput::success(format!(
            "{}\trefs/dolt/data\n",
            "b".repeat(40)
        ))),
    ]);
    {
        let mut runner = SyncCommandRunner::new(&mut inner, binding(root.path()));
        runner.run(observation(root.path())).unwrap();
        runner.authorize_fresh_clone(&[]).unwrap();
        runner.run(init(root.path())).unwrap();
        runner.run(remote_list(root.path())).unwrap();
        runner.run(observation(root.path())).unwrap();
        assert_eq!(
            runner.require_stable_observation().unwrap_err().code(),
            "remote_changed"
        );
    }
    inner.finish().unwrap();
}

#[test]
fn sync_human_and_json_results_carry_the_same_freshness() {
    let freshness = FreshnessEnvelope {
        last_successful_sync_at: Some("2026-09-02T12:34:56Z".into()),
        local_generation: "local-generation".into(),
        remote_generation: Some("a".repeat(40)),
        remote_observed_at: Some("2026-09-02T12:34:56Z".into()),
        pending_mutations: PendingMutationEnvelope {
            count: 0,
            operation_ids: Vec::new(),
        },
        freshness: Freshness::SynchronizedAsOf,
    };
    let result = SyncResult::synchronized("b".repeat(40), freshness.clone(), true);
    let json = serde_json::to_value(&result).unwrap();

    assert_eq!(json["command"], "sync");
    assert_eq!(json["project_id"], "plasmosome");
    assert_eq!(json["outcome"], "synchronized");
    assert_eq!(json["authority_mode"], "markdown-shadow");
    assert_eq!(json["state_changed"], true);
    assert_eq!(json["freshness"], serde_json::to_value(&freshness).unwrap());

    let human = render_sync_human(&result);
    assert!(human.contains("sync: synchronized as of 2026-09-02T12:34:56Z"));
    assert!(human.contains("authority mode: markdown-shadow"));
    assert!(human.contains("local generation: local-generation"));
    assert!(human.contains(&format!("remote generation: {}", "a".repeat(40))));
    assert!(human.contains("freshness: synchronized as of 2026-09-02T12:34:56Z"));
    assert!(!human.contains("current"));
}
