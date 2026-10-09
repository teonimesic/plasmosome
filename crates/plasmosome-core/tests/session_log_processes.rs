use plasmosome_core::{LogFault, SessionLog, SessionLogError};
use serde_json::json;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const LOG: &str = "session.ndjson";
const LOCK_HOLDER: &str = "PLASMOSOME_SESSION_LOG_LOCK_HOLDER";
const HOLDING: &str = "session log writer lock held";
const PATIENCE: Duration = Duration::from_secs(60);

struct PausedChild(libc::pid_t);

impl PausedChild {
    fn fork() -> PausedChild {
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            loop {
                unsafe { libc::pause() };
            }
        }
        assert!(pid > 0, "fork: {}", std::io::Error::last_os_error());
        PausedChild(pid)
    }

    fn is_alive(&self) -> bool {
        unsafe { libc::kill(self.0, 0) == 0 }
    }
}

impl Drop for PausedChild {
    fn drop(&mut self) {
        unsafe { libc::kill(self.0, libc::SIGKILL) };
        let deadline = Instant::now() + PATIENCE;
        loop {
            let mut status = 0;
            let reaped = unsafe { libc::waitpid(self.0, &mut status, libc::WNOHANG) };
            if reaped == self.0 || reaped == -1 {
                return;
            }
            if Instant::now() >= deadline {
                assert!(
                    std::thread::panicking(),
                    "the forked child {} was not reaped within {PATIENCE:?}",
                    self.0
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn assert_locked(result: Result<SessionLog, SessionLogError>, at: &Path) {
    match result {
        Err(SessionLogError::Locked { path }) => assert_eq!(path, at),
        Err(other) => panic!("expected Locked, got {other:?}"),
        Ok(_) => panic!("a second writer opened {at:?}"),
    }
}

fn give_up(child: &mut Child, waiting_for: &str) -> ! {
    let _ = child.kill();
    let _ = child.wait();
    panic!("{waiting_for} within {PATIENCE:?}");
}

fn wait_within(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + PATIENCE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Ok(None) => give_up(child, "the holder exits"),
            Err(error) => give_up(child, &format!("the holder's state is readable ({error})")),
        }
    }
}

#[test]
#[ignore = "run by a_second_writer_in_another_process_is_refused"]
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
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(LOCK_HOLDER, &path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let output = holder.stdout.take().unwrap();
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(output)
            .lines()
            .map_while(Result::ok)
        {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + PATIENCE;
    loop {
        match lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) if line.contains(HOLDING) => break,
            Ok(_) => {}
            Err(_) => give_up(&mut holder, "the other process takes the writer lock"),
        }
    }
    let refused = SessionLog::open(path.clone());
    drop(holder.stdin.take());
    let status = wait_within(&mut holder);
    let rest: Vec<String> = lines.iter().collect();
    assert!(status.success(), "{rest:?}");
    assert_locked(refused, &path);
    let after = SessionLog::open(path.clone()).unwrap();
    assert_eq!(after.append("a", json!({})).unwrap(), 1);
}

#[test]
fn a_forked_child_does_not_keep_a_dropped_log_locked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LOG);
    let log = SessionLog::open(path.clone()).unwrap();
    assert_eq!(log.append("a", json!({})).unwrap(), 1);
    let child = PausedChild::fork();
    drop(log);
    let reopened = SessionLog::open(path.clone());
    assert!(child.is_alive(), "the forked child still shares the file");
    let log = match reopened {
        Ok(log) => log,
        Err(error) => panic!("reopen after drop while a forked child lives: {error:?}"),
    };
    assert_eq!(log.append("b", json!({})).unwrap(), 2);
}

#[test]
fn a_forked_child_does_not_keep_a_poisoned_log_locked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LOG);
    std::fs::write(
        &path,
        format!("{{\"seq\":{},\"kind\":\"a\"}}\n", u64::MAX - 1),
    )
    .unwrap();
    let log = SessionLog::open(path.clone()).unwrap();
    let child = PausedChild::fork();
    assert_eq!(log.append("b", json!({})).unwrap(), u64::MAX);
    let reopened = SessionLog::open(path.clone());
    assert!(child.is_alive(), "the forked child still shares the file");
    match reopened {
        Err(SessionLogError::Malformed { line, fault, .. }) => {
            assert_eq!((line, fault), (2, LogFault::SequenceExhausted));
        }
        Err(other) => panic!("reopen after poison while a forked child lives: {other:?}"),
        Ok(_) => panic!("a log whose last seq is u64::MAX opened"),
    }
}
