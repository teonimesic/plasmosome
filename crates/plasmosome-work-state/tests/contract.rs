use plasmosome_work_state::command::{CommandOutput, CommandSpec, RecordingCommandRunner};
use plasmosome_work_state::contract::{
    Publication, PushFailure, assert_no_ls_remote, classify_push, dispose_fixture_root,
    execute_publication_command, leased_ref_update, prepare_store_fixture, publish_candidate,
    recover_after_lost_response, retry_after_transport, run_scripted_case, run_scripted_cases,
    run_scripted_contract_case, scripted_outcomes, validate_independent_stores,
    validate_logical_export, validate_scripted_history,
};

const G0: &str = "0000000000000000000000000000000000000000";
const G1: &str = "1111111111111111111111111111111111111111";
const G2: &str = "2222222222222222222222222222222222222222";

fn observation(generation: &str) -> CommandOutput {
    CommandOutput::success(format!("{generation}\trefs/dolt/data\n"))
}

fn observation_with_operation(generation: &str, operation: &str) -> CommandOutput {
    CommandOutput::success(format!(
        "{generation}\trefs/dolt/data\toperation:{operation}\n"
    ))
}

fn assert_isolated_plan(command: &CommandSpec) {
    assert_eq!(
        command.cwd.as_deref(),
        Some(std::path::Path::new("/contract-isolated/repository"))
    );
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
        "PATH",
    ] {
        assert!(command.environment.contains_key(key), "missing {key}");
    }
    assert!(
        command
            .environment
            .iter()
            .filter(|(key, _)| key.as_str() != "PATH")
            .all(|(_, value)| !value.contains("/Users/"))
    );
}

#[test]
fn publication_plan_is_non_forcing_and_observes_before_and_after() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Ok(CommandOutput::success("published")),
        Ok(observation(G1)),
    ]);
    let result = publish_candidate(&mut runner, "winner").unwrap();
    assert_eq!(
        result,
        Publication::Published {
            operation: "winner".into(),
            generation: G1.into()
        }
    );
    assert_eq!(runner.commands().len(), 3);
    assert_eq!(
        runner.commands()[0].argv,
        vec!["ls-remote", "--exit-code", "origin", "refs/dolt/data"]
    );
    assert_eq!(
        runner.commands()[1].argv,
        vec!["--sandbox", "dolt", "push", "--remote", "origin"]
    );
    assert!(
        !runner.commands()[1]
            .argv
            .iter()
            .any(|arg| arg.contains("force"))
    );
    assert_eq!(
        runner.commands()[2].argv,
        vec!["ls-remote", "--exit-code", "origin", "refs/dolt/data"]
    );
    for command in runner.commands() {
        assert_isolated_plan(command);
    }
    assert!(runner.finish().is_ok());
}

#[test]
fn leased_ref_update_requires_the_exact_expected_generation() {
    let command = leased_ref_update(G1, G2).expect("40 hex expected base is accepted");
    assert_eq!(
        command.argv,
        vec![
            "push".to_owned(),
            "origin".to_owned(),
            format!("--force-with-lease=refs/dolt/data:{G1}"),
            format!("{G2}:refs/dolt/data")
        ]
    );
    assert!(leased_ref_update("missing", G2).is_err());
    assert!(leased_ref_update(G1, "candidate").is_err());
    assert_isolated_plan(&command);
}

#[test]
fn stale_contender_is_classified_separately_from_transport_failure() {
    assert_eq!(classify_push("non-fast-forward"), PushFailure::StaleBase);
    assert_eq!(
        classify_push("connection reset by peer"),
        PushFailure::Transport
    );
}

#[test]
fn the_first_stale_push_preserves_the_winners_generation() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation_with_operation(G1, "lost-response")),
    ]);
    let result = publish_candidate(&mut runner, "stale").unwrap();
    assert_eq!(
        result,
        Publication::StaleBase {
            operation: "stale".into(),
            generation: G1.into()
        }
    );
    assert_eq!(runner.commands().len(), 3);
    assert!(runner.finish().is_ok());
}

#[test]
fn a_paused_former_holder_cannot_publish_after_recovery_advances_the_ref() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G1)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(G2)),
    ]);
    let result = publish_candidate(&mut runner, "paused-a").unwrap();
    assert_eq!(
        result,
        Publication::StaleBase {
            operation: "paused-a".into(),
            generation: G2.into()
        }
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn failure_before_publication_retries_the_same_candidate_once() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Err("connection reset".into()),
        Ok(observation(G0)),
        Ok(CommandOutput::success("published")),
        Ok(observation(G1)),
    ]);
    let result = retry_after_transport(&mut runner, "same-operation").unwrap();
    assert_eq!(
        result,
        Publication::Published {
            operation: "same-operation".into(),
            generation: G1.into()
        }
    );
    assert_eq!(
        runner
            .commands()
            .iter()
            .filter(|command| command.argv[2] == "push")
            .count(),
        2
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn lost_response_is_recovered_without_a_second_push() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Err("connection reset".into()),
        Ok(observation_with_operation(G1, "same-operation")),
    ]);
    let result = recover_after_lost_response(&mut runner, "same-operation").unwrap();
    assert_eq!(
        result,
        Publication::Recovered {
            operation: "same-operation".into(),
            generation: G1.into()
        }
    );
    assert_eq!(
        runner
            .commands()
            .iter()
            .filter(|command| command.argv[2] == "push")
            .count(),
        1
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn lost_response_does_not_claim_an_unrelated_generation_as_our_operation() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Err("connection reset".into()),
        Ok(observation_with_operation(G1, "another-operation")),
    ]);
    assert_eq!(
        recover_after_lost_response(&mut runner, "same-operation"),
        Err("cutover_blocked".into())
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn lost_response_at_the_same_generation_retries_the_prepared_candidate_once() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Err("connection reset".into()),
        Ok(observation(G0)),
        Ok(CommandOutput::success("published")),
        Ok(observation(G1)),
    ]);
    let result = recover_after_lost_response(&mut runner, "same-operation").unwrap();
    assert_eq!(
        result,
        Publication::Published {
            operation: "same-operation".into(),
            generation: G1.into(),
        }
    );
    assert_eq!(
        runner
            .commands()
            .iter()
            .filter(|command| command.argv[2] == "push")
            .count(),
        2
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn execution_time_validation_refuses_unsafe_untrusted_force_plans_before_dispatch() {
    let unsafe_force = CommandSpec {
        program: "git".into(),
        argv: vec!["push".into(), "origin".into(), "--force".into()],
        cwd: None,
        environment: Default::default(),
        redacted_argv_positions: Vec::new(),
    };
    let wrong_lease = CommandSpec {
        program: "git".into(),
        argv: vec![
            "push".into(),
            "origin".into(),
            format!("--force-with-lease=refs/dolt/data:{G1}"),
            format!("{G2}:refs/dolt/data"),
        ],
        cwd: None,
        environment: Default::default(),
        redacted_argv_positions: Vec::new(),
    };
    let short_force = CommandSpec {
        program: "git".into(),
        argv: vec!["push".into(), "origin".into(), "-f".into()],
        cwd: None,
        environment: Default::default(),
        redacted_argv_positions: Vec::new(),
    };
    let forced_refspec = CommandSpec {
        program: "git".into(),
        argv: vec![
            "push".into(),
            "origin".into(),
            format!("+{G2}:refs/dolt/data"),
        ],
        cwd: None,
        environment: Default::default(),
        redacted_argv_positions: Vec::new(),
    };
    let unleased_ref_update = CommandSpec {
        program: "git".into(),
        argv: vec![
            "push".into(),
            "origin".into(),
            format!("{G2}:refs/dolt/data"),
        ],
        cwd: None,
        environment: Default::default(),
        redacted_argv_positions: Vec::new(),
    };
    for command in [
        unsafe_force,
        wrong_lease,
        short_force,
        forced_refspec,
        unleased_ref_update,
    ] {
        let mut runner = RecordingCommandRunner::scripted(vec![Ok(CommandOutput::success("no"))]);
        assert_eq!(
            execute_publication_command(&mut runner, command, G0),
            Err("cutover_blocked".into())
        );
        assert!(runner.commands().is_empty());
        assert!(runner.finish().is_err(), "unsafe command must not dispatch");
    }
}

#[test]
fn embedded_cleanup_never_plans_or_invokes_dolt_stop() {
    let commands = plasmosome_work_state::contract::embedded_cleanup_commands();
    assert!(
        commands.is_empty(),
        "embedded mode owns no server process to stop"
    );
}

#[test]
fn guarded_pull_replay_push_preserves_both_operations_once() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Ok(observation(G0)),
        Ok(CommandOutput::success("winner")),
        Ok(observation(G1)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(G1)),
        Ok(CommandOutput::success("refreshed")),
        Ok(CommandOutput::success("replayed")),
        Ok(observation(G2)),
    ]);
    let evidence = run_scripted_case("push-conflict-recovery", &mut runner).unwrap();
    assert_eq!(evidence.final_generation, G2);
    assert_eq!(evidence.operation_ids, vec!["winner", "replay"]);
    assert!(
        runner
            .commands()
            .iter()
            .any(|command| command.argv == vec!["--sandbox", "dolt", "pull", "--remote", "origin"])
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn stale_base_is_never_routed_through_transport_retry() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Ok(observation(G0)),
        Ok(CommandOutput::success("winner")),
        Ok(observation(G1)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(G1)),
        Ok(CommandOutput::success("refreshed")),
        Ok(CommandOutput::success("replayed")),
        Ok(observation(G2)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(G2)),
    ]);
    let evidence = run_scripted_case("stale-base-fence", &mut runner).unwrap();
    assert_eq!(evidence.final_generation, G2);
    assert_eq!(
        runner
            .commands()
            .iter()
            .filter(|command| command.argv[2] == "push")
            .count(),
        4
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn contradictory_scripted_result_is_cutover_blocked() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Ok(observation(G0)),
        Ok(CommandOutput::success("published")),
        Ok(observation(G0)),
    ]);
    assert_eq!(
        run_scripted_case("stale-base-fence", &mut runner).unwrap_err(),
        "cutover_blocked"
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn every_named_transport_case_has_its_own_exact_script() {
    for case in [
        "stale-base-fence",
        "push-conflict-recovery",
        "transport-retries",
    ] {
        assert!(!scripted_outcomes(case).unwrap().is_empty(), "{case}");
    }
    assert!(scripted_outcomes("unknown").is_err());
}

#[test]
fn recovery_observes_both_g0_candidates_before_the_winner_publishes_g1() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Ok(observation(G0)),
        Ok(CommandOutput::success("winner")),
        Ok(observation(G1)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(G1)),
        Ok(CommandOutput::success("refreshed")),
        Ok(CommandOutput::success("replayed")),
        Ok(observation(G2)),
    ]);
    let evidence = run_scripted_case("push-conflict-recovery", &mut runner).unwrap();
    assert_eq!(evidence.final_generation, G2);
    let commands = runner.commands();
    assert_eq!(commands[0].argv[0], "ls-remote");
    assert_eq!(commands[1].argv[0], "ls-remote");
    assert_eq!(commands[2].argv[2], "push");
    assert!(runner.finish().is_ok());
}

#[test]
fn scripted_result_names_generation_operation_ids_and_redacted_plans() {
    let mut runner =
        RecordingCommandRunner::scripted(scripted_outcomes("stale-base-fence").unwrap());
    let result = run_scripted_contract_case("stale-base-fence", &mut runner).unwrap();
    assert_eq!(result.observed_base.as_deref(), Some(G0));
    assert_eq!(result.final_generation.as_deref(), Some(G2));
    assert_eq!(result.operation_ids, vec!["winner", "replay"]);
    assert_eq!(result.command_plans.len(), 11);
    assert!(
        result
            .command_plans
            .iter()
            .all(|plan| !plan.contains("origin"))
    );
    validate_scripted_history(&mut runner, &[G0, G1, G2], &["winner", "replay"]).unwrap();
    assert!(runner.finish().is_ok());
}

#[test]
fn scripted_result_derives_its_observed_base_from_the_executed_script() {
    let h0 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let h1 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let h2 = "cccccccccccccccccccccccccccccccccccccccc";
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(h0)),
        Ok(observation(h0)),
        Ok(CommandOutput::success("winner")),
        Ok(observation(h1)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(h1)),
        Ok(CommandOutput::success("refreshed")),
        Ok(CommandOutput::success("replayed")),
        Ok(observation(h2)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(h2)),
    ]);
    let result = run_scripted_contract_case("stale-base-fence", &mut runner).unwrap();
    assert_eq!(result.observed_base.as_deref(), Some(h0));
    assert_eq!(result.final_generation.as_deref(), Some(h2));
    assert!(runner.finish().is_ok());
}

#[test]
fn aggregate_transport_retries_runs_lost_response_rediscovery_after_prepublication_retry() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Err("connection reset".into()),
        Ok(observation(G0)),
        Ok(CommandOutput::success("published")),
        Ok(observation(G1)),
        Ok(observation(G0)),
        Err("connection reset".into()),
        Ok(observation_with_operation(G1, "lost-response")),
    ]);
    let evidence = run_scripted_case("transport-retries", &mut runner).unwrap();
    assert_eq!(evidence.operation_ids, vec!["retry", "lost-response"]);
    assert_eq!(evidence.final_generation, G1);
    assert_eq!(
        runner
            .commands()
            .iter()
            .filter(|command| command.argv[2] == "push")
            .count(),
        3
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn aggregate_result_preserves_each_named_scenario_evidence() {
    let result = run_scripted_cases("transport").unwrap();
    assert_eq!(result.case, "transport");
    assert_eq!(result.scenarios.len(), 3);
    assert_eq!(result.scenarios[0].case, "stale-base-fence");
    assert_eq!(result.scenarios[0].final_generation, G2);
    assert_eq!(result.scenarios[1].case, "push-conflict-recovery");
    assert_eq!(result.scenarios[1].final_generation, G2);
    assert_eq!(result.scenarios[2].case, "transport-retries");
    assert_eq!(result.scenarios[2].final_generation, G1);
}

#[test]
fn logical_export_requires_each_replayed_operation_exactly_once() {
    assert!(validate_logical_export(&["winner", "replay"], &["winner", "replay"]).is_ok());
    assert_eq!(
        validate_logical_export(&["winner", "winner"], &["winner", "replay"]),
        Err("cutover_blocked")
    );
    assert_eq!(
        validate_logical_export(&["winner", "replay", "replay"], &["winner", "replay"]),
        Err("cutover_blocked")
    );
}

#[test]
fn scripted_history_requires_g0_winner_replay_and_their_contents() {
    let mut runner = RecordingCommandRunner::scripted(vec![Ok(CommandOutput::success(format!(
        "{G0}\tbase\n{G1}\toperation:winner\n{G2}\toperation:replay\n"
    )))]);
    validate_scripted_history(&mut runner, &[G0, G1, G2], &["winner", "replay"]).unwrap();
    assert_eq!(runner.commands()[0].argv, vec!["log", "--format=%H%x09%s"]);
    assert!(runner.finish().is_ok());
}

#[test]
fn stale_base_fence_keeps_g1_then_g2_across_recovery_and_paused_holder() {
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(observation(G0)),
        Ok(observation(G0)),
        Ok(CommandOutput::success("winner")),
        Ok(observation(G1)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(G1)),
        Ok(CommandOutput::success("refreshed")),
        Ok(CommandOutput::success("replayed")),
        Ok(observation(G2)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(G2)),
    ]);
    let evidence = run_scripted_case("stale-base-fence", &mut runner).unwrap();
    assert_eq!(evidence.final_generation, G2);
    assert_eq!(evidence.operation_ids, vec!["winner", "replay"]);
    assert_eq!(
        runner
            .commands()
            .iter()
            .filter(|command| command.argv[2] == "push")
            .count(),
        4
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn two_store_fixtures_have_independent_roots_and_snapshots() {
    let root = tempfile::tempdir().unwrap();
    let first = prepare_store_fixture(root.path(), "clone-a").unwrap();
    let second = prepare_store_fixture(root.path(), "clone-b").unwrap();

    assert_ne!(first.clone_root, second.clone_root);
    assert_ne!(first.repository, second.repository);
    assert_ne!(first.store_root, second.store_root);
    assert_ne!(first.environment["HOME"], second.environment["HOME"]);
    assert_ne!(
        first.environment["GIT_CONFIG_GLOBAL"],
        second.environment["GIT_CONFIG_GLOBAL"]
    );
    first.assert_unchanged().unwrap();
    second.assert_unchanged().unwrap();
    validate_independent_stores(&first, &second).unwrap();
}

#[test]
fn cleanup_removes_a_retained_fixture_root_without_a_server_stop_command() {
    let root = tempfile::tempdir().unwrap();
    let retained = root.path().to_path_buf();
    dispose_fixture_root(root).unwrap();
    assert!(!retained.exists());
    assert!(plasmosome_work_state::contract::embedded_cleanup_commands().is_empty());
}

#[test]
fn fixture_snapshot_detects_a_changed_hook_index_or_local_config() {
    let root = tempfile::tempdir().unwrap();
    let mut fixture = prepare_store_fixture(root.path(), "clone-a").unwrap();
    let git = fixture.repository.join(".git");
    std::fs::create_dir_all(git.join("hooks")).unwrap();
    std::fs::write(git.join("hooks/pre-commit"), "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(git.join("index"), "fixture-index\n").unwrap();
    std::fs::write(git.join("config"), "[core]\n\tbare = false\n").unwrap();
    fixture.snapshot_git_state().unwrap();
    fixture.assert_unchanged().unwrap();
    std::fs::write(git.join("hooks/pre-commit"), "changed\n").unwrap();
    assert_eq!(fixture.assert_unchanged(), Err("cutover_blocked"));
    std::fs::write(git.join("hooks/pre-commit"), "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(git.join("hooks/post-commit"), "unexpected\n").unwrap();
    assert_eq!(fixture.assert_unchanged(), Err("cutover_blocked"));
}

#[test]
fn fixture_allows_only_the_pinned_beads_role_local_config_delta() {
    let root = tempfile::tempdir().unwrap();
    let mut fixture = prepare_store_fixture(root.path(), "clone-a").unwrap();
    let git = fixture.repository.join(".git");
    std::fs::create_dir_all(git.join("hooks")).unwrap();
    std::fs::write(git.join("hooks/pre-commit"), "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(git.join("index"), "fixture-index\n").unwrap();
    std::fs::write(git.join("config"), "[core]\n\tbare = false\n").unwrap();
    fixture.snapshot_git_state().unwrap();
    std::fs::write(
        git.join("config"),
        "[core]\n\tbare = false\n[beads]\n\trole = maintainer\n",
    )
    .unwrap();
    fixture.assert_after_stealth_init().unwrap();
    std::fs::write(
        git.join("config"),
        "[core]\n\tbare = true\n[beads]\n\trole = maintainer\n",
    )
    .unwrap();
    assert_eq!(fixture.assert_after_stealth_init(), Err("cutover_blocked"));
}

#[test]
fn hermetic_init_rejects_a_planned_ls_remote_but_allows_local_git_commands() {
    let local = CommandSpec {
        program: "git".into(),
        argv: vec!["status".into(), "--porcelain".into()],
        cwd: None,
        environment: Default::default(),
        redacted_argv_positions: Vec::new(),
    };
    assert!(assert_no_ls_remote(&[local]).is_ok());
    assert_eq!(
        assert_no_ls_remote(&[plasmosome_work_state::contract::observe_command()]),
        Err("cutover_blocked")
    );
}
