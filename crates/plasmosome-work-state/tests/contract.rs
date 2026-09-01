use plasmosome_work_state::command::{CommandOutput, RecordingCommandRunner};
use plasmosome_work_state::contract::{PushFailure, classify_push, fixture_preflight};

#[test]
fn a_fixture_with_credentials_wrong_name_or_existing_ref_is_refused_before_write() {
    for remote in [
        "https://token@github.com/acme/plasmosome-work-state-fixture.git",
        "https://github.com/acme/not-a-fixture.git",
    ] {
        let mut runner = RecordingCommandRunner::default();
        let error = fixture_preflight(remote, "refs/dolt/data", &mut runner).unwrap_err();
        assert_eq!(error, "github_fixture_invalid");
        assert!(runner.commands().is_empty());
    }
    let mut runner =
        RecordingCommandRunner::with_output(CommandOutput::success("deadbeef\trefs/dolt/data\n"));
    let error = fixture_preflight(
        "https://github.com/acme/plasmosome-work-state-fixture.git",
        "refs/dolt/data",
        &mut runner,
    )
    .unwrap_err();
    assert_eq!(error, "github_fixture_not_empty");
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
fn stale_base_fence_uses_two_independent_clone_paths() {
    let paths = plasmosome_work_state::contract::clone_paths("/tmp/root");
    assert_ne!(paths.0, paths.1);
    assert!(paths.0.ends_with("clone-a"));
    assert!(paths.1.ends_with("clone-b"));
}

#[test]
fn cleanup_deletes_only_the_generation_this_run_owns() {
    let command = plasmosome_work_state::contract::cleanup_command(
        "https://github.com/acme/plasmosome-work-state-fixture.git",
        "abc",
    );
    assert_eq!(
        command.argv,
        vec![
            "push",
            "https://github.com/acme/plasmosome-work-state-fixture.git",
            "--force-with-lease=refs/dolt/data:abc",
            ":refs/dolt/data"
        ]
    );
}

#[test]
fn stealth_init_uses_the_exact_non_integrating_command() {
    let command =
        plasmosome_work_state::contract::stealth_init_command("/tmp/bd", "/tmp/repo", "/tmp/root");
    assert_eq!(
        command.argv,
        vec![
            "--sandbox",
            "init",
            "--stealth",
            "--skip-agents",
            "--skip-hooks",
            "--non-interactive"
        ]
    );
    assert_eq!(command.cwd.unwrap(), std::path::PathBuf::from("/tmp/repo"));
    assert!(command.environment.contains_key("BD_DISABLE_METRICS"));
}
