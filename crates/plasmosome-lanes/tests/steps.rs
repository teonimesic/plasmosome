mod common;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use plasmosome_lanes::run::Outcome;
use plasmosome_lanes::steps::{Step, StepsRun, run_lane_with_steps};

const TIMEOUT: Duration = Duration::from_secs(20);

fn quoted(path: &Path) -> String {
    format!("'{}'", path.display())
}

fn result_line(probe: &str) -> String {
    format!("LANE-RESULT {}", common::result_json(probe, 1000))
}

fn wait_for(path: &Path) -> String {
    format!(
        "n=0\nwhile [ ! -e {path} ]; do n=$((n+1)); [ $n -gt 1000 ] && exit 9; sleep 0.01; done\n",
        path = quoted(path),
    )
}

fn waits_for(flag: &Path, console: &Path, after: &str) -> String {
    format!(
        "{wait}printf '%s\\n' '{line}' >> {console}\n",
        wait = wait_for(flag),
        console = quoted(console),
        line = result_line(after),
    )
}

fn header(console: &Path, first: &str) -> String {
    format!(
        "cat > /dev/null\nprintf '%s\\n' 'LANE-BEGIN run-1' '{line}' > {console}\n",
        console = quoted(console),
        line = result_line(first),
    )
}

fn end(console: &Path, count: usize) -> String {
    format!(
        "printf '%s\\n' 'LANE-END run-1 {count}' >> {}\n",
        quoted(console)
    )
}

fn run(dir: &Path, body: &str, probes: &[&str], steps: Vec<Step>) -> StepsRun {
    let lane = common::lane_config(dir, "run-1");
    let launcher = common::script(dir, "steps", body);
    let guest = common::guest_config("run-1", probes);
    run_lane_with_steps(&launcher, &lane, &guest, TIMEOUT, steps).expect("the launcher starts")
}

fn finished(run: &StepsRun) -> Vec<String> {
    match &run.run.outcome {
        Outcome::Finished(results) => results.iter().map(|result| result.probe.clone()).collect(),
        other => panic!("the run did not finish: {other:?}"),
    }
}

#[test]
fn a_step_runs_once_its_probe_reports_and_its_flag_releases_the_guest() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let console = dir.path().join("console.log");
    let flag = dir.path().join("flag-a");
    let order = Arc::new(Mutex::new(Vec::new()));
    let seen_by_action = Arc::clone(&order);
    let console_for_action = console.clone();
    let flag_for_action = flag.clone();
    let flagged_early = Arc::new(Mutex::new(None));
    let flagged_by_action = Arc::clone(&flagged_early);
    let body = format!(
        "{}{}{}",
        header(&console, "a"),
        waits_for(&flag, &console, "b"),
        end(&console, 2)
    );
    let steps = vec![Step {
        after: "a".to_string(),
        flag: flag.clone(),
        action: Box::new(move || {
            *flagged_by_action.lock().expect("the flag check") = Some(flag_for_action.exists());
            let text = fs::read_to_string(&console_for_action)?;
            seen_by_action
                .lock()
                .expect("the order")
                .push(text.lines().count());
            Ok("moved the tree".to_string())
        }),
    }];

    let run = run(dir.path(), &body, &["a", "b"], steps);

    assert_eq!(finished(&run), ["a", "b"]);
    assert_eq!(run.steps.len(), 1);
    assert_eq!(run.steps[0].after, "a");
    assert_eq!(run.steps[0].outcome, Ok("moved the tree".to_string()));
    assert!(run.steps[0].flagged_ms >= run.steps[0].started_ms);
    assert_eq!(*order.lock().expect("the order"), [2]);
    assert_eq!(
        *flagged_early.lock().expect("the flag check"),
        Some(false),
        "the flag existed while the action ran"
    );
    assert!(flag.exists());
    let seen: Vec<&str> = run.seen.iter().map(|(probe, _)| probe.as_str()).collect();
    assert_eq!(seen, ["a", "b"]);
    assert!(run.seen[0].1 <= run.steps[0].started_ms);
    assert!(run.seen[1].1 >= run.steps[0].flagged_ms);
}

#[test]
fn steps_run_in_order_each_after_its_own_probe() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let console = dir.path().join("console.log");
    let first = dir.path().join("flag-a");
    let second = dir.path().join("flag-b");
    let body = format!(
        "{}{}{}{}",
        header(&console, "a"),
        waits_for(&first, &console, "b"),
        waits_for(&second, &console, "c"),
        end(&console, 3)
    );
    let calls = Arc::new(Mutex::new(Vec::new()));
    let step = |after: &str, flag: &PathBuf| {
        let calls = Arc::clone(&calls);
        let name = after.to_string();
        Step {
            after: after.to_string(),
            flag: flag.clone(),
            action: Box::new(move || {
                calls.lock().expect("calls").push(name.clone());
                Ok(name)
            }),
        }
    };
    let steps = vec![step("a", &first), step("b", &second)];

    let run = run(dir.path(), &body, &["a", "b", "c"], steps);

    assert_eq!(finished(&run), ["a", "b", "c"]);
    assert_eq!(*calls.lock().expect("calls"), ["a", "b"]);
    assert_eq!(
        run.steps
            .iter()
            .map(|step| step.after.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}

#[test]
fn a_failed_action_is_recorded_and_writes_no_flag() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let console = dir.path().join("console.log");
    let flag = dir.path().join("flag-a");
    let acted = dir.path().join("acted");
    let marker = acted.clone();
    let body = format!(
        "{}{}{}",
        header(&console, "a"),
        wait_for(&acted),
        end(&console, 1)
    );
    let steps = vec![Step {
        after: "a".to_string(),
        flag: flag.clone(),
        action: Box::new(move || {
            fs::write(&marker, b"")?;
            Err(io::Error::other("the move failed"))
        }),
    }];

    let run = run(dir.path(), &body, &["a"], steps);

    assert_eq!(finished(&run), ["a"]);
    assert_eq!(run.steps.len(), 1);
    let error = run.steps[0]
        .outcome
        .clone()
        .expect_err("the error is recorded");
    assert!(error.contains("the move failed"), "{error}");
    assert!(!flag.exists(), "a failed action released the guest");
}

#[test]
fn a_step_whose_probe_never_reports_is_not_taken() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let console = dir.path().join("console.log");
    let flag = dir.path().join("flag-z");
    let body = format!("{}{}", header(&console, "a"), end(&console, 1));
    let taken = Arc::new(Mutex::new(false));
    let marker = Arc::clone(&taken);
    let steps = vec![Step {
        after: "z".to_string(),
        flag: flag.clone(),
        action: Box::new(move || {
            *marker.lock().expect("the marker") = true;
            Ok(String::new())
        }),
    }];

    let run = run(dir.path(), &body, &["a"], steps);

    assert_eq!(finished(&run), ["a"]);
    assert!(run.steps.is_empty());
    assert!(!*taken.lock().expect("the marker"));
    assert!(!flag.exists());
}

#[test]
fn a_result_line_still_being_written_does_not_start_a_step() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let console = dir.path().join("console.log");
    let flag = dir.path().join("flag-a");
    let partial = result_line("a");
    let (head, tail) = partial.split_at(partial.len() / 2);
    let body = format!(
        "cat > /dev/null\nprintf '%s\\n' 'LANE-BEGIN run-1' > {console}\nprintf '%s' '{head}' >> {console}\nsleep 0.2\n[ -e {flag} ] && exit 8\nprintf '%s\\n' '{tail}' >> {console}\n{wait}{end}",
        console = quoted(&console),
        flag = quoted(&flag),
        wait = waits_for(&flag, &console, "b"),
        end = end(&console, 2),
    );
    let steps = vec![Step {
        after: "a".to_string(),
        flag: flag.clone(),
        action: Box::new(|| Ok(String::new())),
    }];

    let run = run(dir.path(), &body, &["a", "b"], steps);

    assert_eq!(finished(&run), ["a", "b"], "{:?}", run.run.status);
    assert_eq!(run.steps.len(), 1);
}

#[test]
fn a_whole_result_line_with_no_newline_yet_does_not_start_a_step() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let console = dir.path().join("console.log");
    let flag = dir.path().join("flag-a");
    let body = format!(
        "cat > /dev/null\nprintf '%s\\n' 'LANE-BEGIN run-1' > {console}\nprintf '%s' '{line}' >> {console}\nsleep 0.2\n[ -e {flag} ] && exit 8\nprintf '\\n' >> {console}\n{wait}{end}",
        console = quoted(&console),
        flag = quoted(&flag),
        line = result_line("a"),
        wait = waits_for(&flag, &console, "b"),
        end = end(&console, 2),
    );
    let steps = vec![Step {
        after: "a".to_string(),
        flag: flag.clone(),
        action: Box::new(|| Ok(String::new())),
    }];

    let run = run(dir.path(), &body, &["a", "b"], steps);

    assert_eq!(finished(&run), ["a", "b"], "{:?}", run.run.status);
    assert_eq!(run.steps.len(), 1);
}

#[test]
fn a_flag_that_cannot_be_written_is_recorded_as_the_steps_error() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let console = dir.path().join("console.log");
    let flag = dir.path().join("missing/flag-a");
    let acted = dir.path().join("acted");
    let marker = acted.clone();
    let body = format!(
        "{}{}{}",
        header(&console, "a"),
        wait_for(&acted),
        end(&console, 1)
    );
    let steps = vec![Step {
        after: "a".to_string(),
        flag,
        action: Box::new(move || {
            fs::write(&marker, b"")?;
            Ok("moved".to_string())
        }),
    }];

    let run = run(dir.path(), &body, &["a"], steps);

    assert_eq!(finished(&run), ["a"]);
    let error = run.steps[0]
        .outcome
        .clone()
        .expect_err("the flag error is recorded");
    assert!(error.contains("could not be written"), "{error}");
}
