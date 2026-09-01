use plasmosome_work_state::command::{CommandOutput, RecordingCommandRunner};
use plasmosome_work_state::contract::{
    Publication, PushFailure, classify_push, leased_ref_update, publish_candidate,
    recover_after_lost_response, retry_after_transport, run_scripted_case, scripted_outcomes,
};

const G0: &str = "0000000000000000000000000000000000000000";
const G1: &str = "1111111111111111111111111111111111111111";
const G2: &str = "2222222222222222222222222222222222222222";

fn observation(generation: &str) -> CommandOutput {
    CommandOutput::success(format!("{generation}\trefs/dolt/data\n"))
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
        Ok(observation(G1)),
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
        Ok(observation(G1)),
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
        Ok(CommandOutput::success("winner")),
        Ok(observation(G1)),
        Ok(observation(G0)),
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(G1)),
        Ok(CommandOutput::success("refreshed")),
        Ok(observation(G1)),
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
        Ok(CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: "non-fast-forward".into(),
        }),
        Ok(observation(G1)),
    ]);
    let evidence = run_scripted_case("stale-base-fence", &mut runner).unwrap();
    assert_eq!(evidence.final_generation, G1);
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
fn contradictory_scripted_result_is_cutover_blocked() {
    let mut runner = RecordingCommandRunner::scripted(vec![
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
