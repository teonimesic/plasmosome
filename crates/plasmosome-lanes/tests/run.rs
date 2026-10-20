mod common;

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use plasmosome_lanes::config::{GuestConfig, LaneConfig};
use plasmosome_lanes::run::{LaneRun, Outcome, run_lane};

fn process_exists(pid: u32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string()])
        .output()
        .expect("ps runs")
        .status
        .success()
}

fn running(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    output.status.success()
        && !String::from_utf8_lossy(&output.stdout)
            .trim()
            .starts_with('Z')
}

fn stops_within(pid: u32, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while running(pid) {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
    true
}

const PROMPT: Duration = Duration::from_secs(15);

struct KillOnDrop(Vec<PathBuf>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        for file in &self.0 {
            if let Ok(pid) = fs::read_to_string(file) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
    }
}

fn read_pid(file: &Path) -> u32 {
    fs::read_to_string(file)
        .unwrap_or_else(|error| panic!("the launcher wrote no pid to {}: {error}", file.display()))
        .trim()
        .parse()
        .expect("the pid file holds a number")
}

fn run_within(
    limit: Duration,
    launcher: PathBuf,
    lane: LaneConfig,
    guest: GuestConfig,
    timeout: Duration,
) -> (LaneRun, Duration) {
    let (sender, receiver) = mpsc::channel();
    let started = Instant::now();
    thread::spawn(move || {
        let _ = sender.send(run_lane(&launcher, &lane, &guest, timeout));
    });
    let run = receiver
        .recv_timeout(limit)
        .unwrap_or_else(|_| panic!("run_lane had not returned after {limit:?}"))
        .expect("spawned");
    (run, started.elapsed())
}

const CONSOLE_BOUND: u64 = 16 * 1024 * 1024;
const STDERR_BOUND: usize = 1024 * 1024;

fn complete_run(console: &std::path::Path) -> String {
    format!(
        "printf 'LANE-BEGIN run-1\\nLANE-RESULT %s\\nLANE-END run-1 1\\n' '{}' > '{}'",
        common::result_json("id", 1000),
        console.display()
    )
}

fn complete_run_padded_to(console: &std::path::Path, total: u64) -> String {
    format!(
        "cat > /dev/null\n{}\nsize=$(wc -c < '{path}')\nyes x | head -c $(({total} - size)) >> '{path}'",
        complete_run(console),
        path = console.display(),
    )
}

#[test]
fn a_console_of_exactly_the_bound_is_read_whole() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let lane = common::lane_config(dir.path(), "run-1");
    let body = complete_run_padded_to(&lane.console, CONSOLE_BOUND);
    let launcher = common::script(dir.path(), "at-bound", &body);
    let guest = common::guest_config("run-1", &["id"]);

    let run = run_lane(&launcher, &lane, &guest, Duration::from_secs(30)).expect("spawned");

    assert!(
        matches!(run.outcome, Outcome::Finished(_)),
        "a console of {CONSOLE_BOUND} bytes must be read: {:?}",
        run.outcome
    );
    assert_eq!(run.console.len() as u64, CONSOLE_BOUND);
}

#[test]
fn a_console_past_the_bound_is_not_run_even_after_a_complete_run() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let lane = common::lane_config(dir.path(), "run-1");
    let body = complete_run_padded_to(&lane.console, CONSOLE_BOUND + 1);
    let launcher = common::script(dir.path(), "past-bound", &body);
    let guest = common::guest_config("run-1", &["id"]);

    let run = run_lane(&launcher, &lane, &guest, Duration::from_secs(30)).expect("spawned");

    match &run.outcome {
        Outcome::NotRun(reason) => {
            assert!(
                reason.contains(&format!("{CONSOLE_BOUND} bytes")),
                "{reason}"
            )
        }
        other => panic!("a console past the bound must be not run, it was {other:?}"),
    }
    assert!(
        run.console.len() as u64 <= CONSOLE_BOUND,
        "the run kept {} console bytes",
        run.console.len()
    );
}

#[test]
fn stderr_past_its_bound_keeps_the_first_mebibyte_and_the_launcher_keeps_writing() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let lane = common::lane_config(dir.path(), "run-1");
    let body = format!(
        "cat > /dev/null\nyes e | head -c {} >&2 || exit 1\n{}",
        2 * STDERR_BOUND,
        complete_run(&lane.console)
    );
    let launcher = common::script(dir.path(), "loud", &body);
    let guest = common::guest_config("run-1", &["id"]);

    let run = run_lane(&launcher, &lane, &guest, Duration::from_secs(30)).expect("spawned");

    assert!(
        matches!(run.outcome, Outcome::Finished(_)),
        "a launcher writing past the stderr bound must still finish: {:?}",
        run.outcome
    );
    assert_eq!(run.stderr.len(), STDERR_BOUND);
    assert!(run.stderr.starts_with("e\ne\n"), "{:?}", &run.stderr[..8]);
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

#[test]
fn a_child_holding_stderr_past_the_timeout_dies_with_the_launchers_group() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let held = dir.path().join("held.pid");
    let _cleanup = KillOnDrop(vec![held.clone()]);
    let body = format!(
        "sleep 60 &\necho $! > '{}'\ncat > /dev/null\nexec sleep 60",
        held.display()
    );
    let launcher = common::script(dir.path(), "holds-stderr", &body);
    let lane = common::lane_config(dir.path(), "run-1");
    let guest = common::guest_config("run-1", &["id"]);
    let timeout = Duration::from_secs(5);

    let (run, elapsed) = run_within(Duration::from_secs(30), launcher, lane, guest, timeout);

    assert_eq!(run.outcome, Outcome::TimedOut);
    assert!(
        elapsed < timeout + PROMPT,
        "run_lane returned only after {elapsed:?}"
    );
    assert!(
        !process_exists(run.pid),
        "the launcher {} still exists",
        run.pid
    );
    assert!(
        held.exists(),
        "the launcher had not started its child after {elapsed:?}; stderr: {:?}",
        run.stderr
    );
    let child = read_pid(&held);
    assert!(
        stops_within(child, Duration::from_secs(5)),
        "the launcher's child {child} still runs after the run timed out"
    );
}

#[test]
fn a_child_left_holding_stderr_after_a_complete_run_is_killed_and_the_run_finishes() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let held = dir.path().join("held.pid");
    let _cleanup = KillOnDrop(vec![held.clone()]);
    let lane = common::lane_config(dir.path(), "run-1");
    let body = format!(
        "cat > /dev/null\n{}\nsleep 60 &\necho $! > '{}'\nexit 0",
        complete_run(&lane.console),
        held.display()
    );
    let launcher = common::script(dir.path(), "leaves-child", &body);
    let guest = common::guest_config("run-1", &["id"]);

    let (run, elapsed) = run_within(
        Duration::from_secs(30),
        launcher,
        lane,
        guest,
        Duration::from_secs(20),
    );

    assert!(
        matches!(run.outcome, Outcome::Finished(_)),
        "{:?}",
        run.outcome
    );
    assert!(elapsed < PROMPT, "run_lane took {elapsed:?}");
    let child = read_pid(&held);
    assert!(
        stops_within(child, Duration::from_secs(5)),
        "the launcher's child {child} still runs after the launcher exited"
    );
}

#[test]
fn a_process_outside_the_launchers_group_holding_stderr_does_not_keep_run_lane_reading() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let escaped = dir.path().join("escaped.pid");
    let _cleanup = KillOnDrop(vec![escaped.clone()]);
    let lane = common::lane_config(dir.path(), "run-1");
    let body = format!(
        r#"cat > /dev/null
{}
echo 'before the escape' >&2
perl -e '$| = 1; use POSIX; POSIX::setsid(); print "$$\n"; sleep 60' > '{pid}' &
while [ ! -s '{pid}' ]; do sleep 0.05; done
exit 0"#,
        complete_run(&lane.console),
        pid = escaped.display()
    );
    let launcher = common::script(dir.path(), "escapes", &body);
    let guest = common::guest_config("run-1", &["id"]);

    let (run, elapsed) = run_within(
        Duration::from_secs(30),
        launcher,
        lane,
        guest,
        Duration::from_secs(20),
    );

    assert!(
        matches!(run.outcome, Outcome::Finished(_)),
        "{:?}",
        run.outcome
    );
    assert!(elapsed < PROMPT, "run_lane took {elapsed:?}");
    assert!(
        run.stderr.contains("before the escape"),
        "the stderr read before the grace ended is kept: {:?}",
        run.stderr
    );
    assert!(
        running(read_pid(&escaped)),
        "the escaped process must still run, or this test shows nothing"
    );
}

#[test]
fn a_fifo_at_the_console_path_is_refused_without_blocking() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let lane = common::lane_config(dir.path(), "run-1");
    let body = format!(
        "cat > /dev/null\nmkfifo '{}'\nexit 0",
        lane.console.display()
    );
    let launcher = common::script(dir.path(), "fifo", &body);
    let guest = common::guest_config("run-1", &["id"]);

    let (run, elapsed) = run_within(
        Duration::from_secs(30),
        launcher,
        lane,
        guest,
        Duration::from_secs(20),
    );

    match &run.outcome {
        Outcome::NotRun(reason) => {
            assert!(reason.contains("is not a regular file"), "{reason}")
        }
        other => panic!("a FIFO console must be not run, it was {other:?}"),
    }
    assert!(elapsed < PROMPT, "run_lane took {elapsed:?}");
}

#[test]
fn a_symlink_at_the_console_path_is_refused_and_not_followed() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let lane = common::lane_config(dir.path(), "run-1");
    let console = lane.console.clone();
    let body = format!(
        "cat > /dev/null\nln -s /dev/zero '{}'\nexit 0",
        console.display()
    );
    let launcher = common::script(dir.path(), "symlink", &body);
    let guest = common::guest_config("run-1", &["id"]);

    let (run, _) = run_within(
        Duration::from_secs(30),
        launcher,
        lane,
        guest,
        Duration::from_secs(20),
    );

    match &run.outcome {
        Outcome::NotRun(reason) => assert!(
            reason.contains(&console.display().to_string()),
            "the refusal names the console: {reason}"
        ),
        other => panic!("a symlinked console must be not run, it was {other:?}"),
    }
    assert!(
        run.console.is_empty(),
        "{} console bytes were read through the symlink",
        run.console.len()
    );
}

#[test]
fn a_lane_that_cannot_serialize_is_refused_before_spawning() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let marker = dir.path().join("spawned");
    let launcher = common::script(
        dir.path(),
        "touch",
        &format!("touch '{}'", marker.display()),
    );
    let mut lane = common::lane_config(dir.path(), "run-1");
    lane.bundle = PathBuf::from(OsStr::from_bytes(&[b'/', b'b', 0xff]));
    let guest = common::guest_config("run-1", &["id"]);

    let error = run_lane(&launcher, &lane, &guest, Duration::from_secs(10))
        .expect_err("a bundle path that is not UTF-8");

    assert!(error.to_string().contains("bundle"), "{error}");
    assert!(!marker.exists(), "the launcher was spawned anyway");
}
