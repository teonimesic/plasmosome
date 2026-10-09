use plasmosome_core::{LogStep, SessionLog, SessionLogError};
use serde_json::json;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const LOG: &str = "session.ndjson";
const LOCK_HOLDER: &str = "PLASMOSOME_SESSION_LOG_LOCK_HOLDER";
const HOLDING: &str = "session log writer lock held";

fn assert_locked(result: Result<SessionLog, SessionLogError>, at: &Path) {
    match result {
        Err(SessionLogError::Locked { path }) => assert_eq!(path, at),
        Err(other) => panic!("expected Locked, got {other:?}"),
        Ok(_) => panic!("a second writer opened {at:?}"),
    }
}

fn mkfifo(path: &Path) {
    let made = Command::new("mkfifo").arg(path).status().unwrap();
    assert!(made.success(), "mkfifo {path:?}");
}

#[test]
fn hold_a_writer_lock_for_another_process() {
    let Some(path) = std::env::var_os(LOCK_HOLDER) else {
        return;
    };
    let _log = SessionLog::open(PathBuf::from(path)).unwrap();
    println!("{HOLDING}");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
}

#[test]
fn a_second_writer_in_another_process_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LOG);
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "hold_a_writer_lock_for_another_process",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(LOCK_HOLDER, &path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = std::io::BufReader::new(holder.stdout.take().unwrap()).lines();
    let held = lines
        .by_ref()
        .map_while(Result::ok)
        .any(|line| line.contains(HOLDING));
    assert!(held, "the other process took the writer lock");
    let refused = SessionLog::open(path.clone());
    drop(holder.stdin.take());
    let rest: Vec<String> = lines.map_while(Result::ok).collect();
    assert!(holder.wait().unwrap().success(), "{rest:?}");
    assert_locked(refused, &path);
    let after = SessionLog::open(path.clone()).unwrap();
    assert_eq!(after.append("a", json!({})).unwrap(), 1);
}

#[test]
fn open_refuses_a_log_that_is_not_a_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LOG);
    mkfifo(&path);
    let (sender, receiver) = mpsc::channel();
    let opening = path.clone();
    std::thread::spawn(move || sender.send(SessionLog::open(opening).err()));
    match receiver.recv_timeout(Duration::from_secs(10)) {
        Ok(Some(SessionLogError::Io {
            path: at,
            step,
            source,
        })) => {
            assert_eq!(step, LogStep::Open);
            assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput);
            assert_eq!(at, path);
        }
        Ok(other) => panic!("expected an Io error at Open, got {other:?}"),
        Err(timeout) => panic!("open did not return: {timeout:?}"),
    }
}
