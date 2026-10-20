mod common;

use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

use plasmosome_lanes::config::LaneConfig;
use plasmosome_lanes::run::{Outcome, run_lane};

fn process_exists(pid: u32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string()])
        .output()
        .expect("ps runs")
        .status
        .success()
}

fn complete_run(console: &std::path::Path) -> String {
    format!(
        "printf 'LANE-BEGIN run-1\\nLANE-RESULT %s\\nLANE-END run-1 1\\n' '{}' > '{}'",
        common::result_json("id", 1000),
        console.display()
    )
}

#[test]
fn a_launcher_that_never_exits_times_out_and_is_reaped() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let launcher = common::script(dir.path(), "hang", "exec sleep 30");
    let lane = common::lane_config(dir.path(), "run-1");
    let guest = common::guest_config("run-1", &["id"]);
    let started = Instant::now();

    let run = run_lane(&launcher, &lane, &guest, Duration::from_millis(300)).expect("spawned");

    assert_eq!(run.outcome, Outcome::TimedOut);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "run_lane returned only after {:?}",
        started.elapsed()
    );
    assert!(run.pid > 0, "the run reports the launcher's pid");
    assert!(
        !process_exists(run.pid),
        "pid {} still exists after a timed-out run",
        run.pid
    );
    assert_eq!(
        run.status, None,
        "a killed launcher reports no exit of its own"
    );
}

#[test]
fn a_launcher_that_exits_0_without_an_end_marker_is_not_run() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let lane = common::lane_config(dir.path(), "run-1");
    let body = format!(
        "cat > /dev/null\nprintf 'LANE-BEGIN run-1\\nLANE-RESULT %s\\n' '{}' > '{}'\nexit 0",
        common::result_json("id", 1000),
        lane.console.display()
    );
    let launcher = common::script(dir.path(), "partial", &body);
    let guest = common::guest_config("run-1", &["id"]);

    let run = run_lane(&launcher, &lane, &guest, Duration::from_secs(10)).expect("spawned");

    match &run.outcome {
        Outcome::NotRun(reason) => assert!(reason.contains("LANE-END"), "{reason}"),
        other => panic!("a run without LANE-END must be not run, it was {other:?}"),
    }
    assert!(run.status.expect("it exited").success());
    assert!(run.console.contains("LANE-BEGIN run-1"), "{}", run.console);
}

#[test]
fn a_launcher_that_writes_a_complete_run_finishes_with_its_results() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let lane = common::lane_config(dir.path(), "run-1");
    let launcher = common::script(
        dir.path(),
        "complete",
        &format!("cat > /dev/null\n{}", complete_run(&lane.console)),
    );
    let guest = common::guest_config("run-1", &["id"]);

    let run = run_lane(&launcher, &lane, &guest, Duration::from_secs(10)).expect("spawned");

    match &run.outcome {
        Outcome::Finished(results) => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].probe, "id");
        }
        other => panic!("a complete run must finish, it was {other:?}"),
    }
}

#[test]
fn the_launcher_reads_the_lane_configuration_on_stdin_with_an_empty_environment() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let lane = common::lane_config(dir.path(), "run-1");
    let seen = dir.path().join("stdin.json");
    let home = dir.path().join("home.txt");
    let cwd = dir.path().join("cwd.txt");
    let body = format!(
        "cat > '{}'\nprintf '%s' \"$HOME\" > '{}'\npwd > '{}'\n{}",
        seen.display(),
        home.display(),
        cwd.display(),
        complete_run(&lane.console)
    );
    let launcher = common::script(dir.path(), "record", &body);
    let guest = common::guest_config("run-1", &["id"]);

    run_lane(&launcher, &lane, &guest, Duration::from_secs(10)).expect("spawned");

    let text = fs::read_to_string(&seen).expect("the launcher saved its stdin");
    assert_eq!(LaneConfig::from_json(&text), Ok(lane));
    assert_eq!(fs::read_to_string(&home).expect("saved"), "");
    assert_eq!(fs::read_to_string(&cwd).expect("saved").trim(), "/");
}

#[test]
fn a_stale_console_file_from_an_earlier_run_is_not_read_as_this_one() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let lane = common::lane_config(dir.path(), "run-1");
    fs::write(
        &lane.console,
        format!(
            "LANE-BEGIN run-1\nLANE-RESULT {}\nLANE-END run-1 1\n",
            common::result_json("id", 1000)
        ),
    )
    .expect("a stale console is written");
    let launcher = common::script(dir.path(), "silent", "cat > /dev/null\nexit 0");
    let guest = common::guest_config("run-1", &["id"]);

    let run = run_lane(&launcher, &lane, &guest, Duration::from_secs(10)).expect("spawned");

    assert!(
        matches!(run.outcome, Outcome::NotRun(_)),
        "a run that wrote nothing must not finish on an earlier run's console: {:?}",
        run.outcome
    );
}

#[test]
fn a_launcher_that_exits_70_is_not_run_and_reports_its_status() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let launcher = common::script(
        dir.path(),
        "refuse",
        "cat > /dev/null\necho 'lane-vmm: self-check failed' >&2\nexit 70",
    );
    let lane = common::lane_config(dir.path(), "run-1");
    let guest = common::guest_config("run-1", &["id"]);

    let run = run_lane(&launcher, &lane, &guest, Duration::from_secs(10)).expect("spawned");

    assert_eq!(run.status.and_then(|status| status.code()), Some(70));
    match &run.outcome {
        Outcome::NotRun(reason) => assert!(reason.contains("70"), "{reason}"),
        other => panic!("an exit of 70 must be not run, it was {other:?}"),
    }
    assert!(run.stderr.contains("self-check failed"), "{}", run.stderr);
}

#[test]
fn a_lane_and_guest_with_different_run_ids_are_refused_before_spawning() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let marker = dir.path().join("spawned");
    let launcher = common::script(
        dir.path(),
        "touch",
        &format!("touch '{}'", marker.display()),
    );
    let lane = common::lane_config(dir.path(), "run-1");
    let guest = common::guest_config("run-2", &["id"]);

    let error =
        run_lane(&launcher, &lane, &guest, Duration::from_secs(10)).expect_err("different run IDs");

    assert!(error.to_string().contains("run-2"), "{error}");
    assert!(!marker.exists(), "the launcher was spawned anyway");
}
