use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::{GuestConfig, LaneConfig};
use crate::console::{ConsoleRun, ProbeResult, parse_console};

const POLL: Duration = Duration::from_millis(10);

/// How one lane ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The VMM exited and its console holds a complete run.
    Finished(Vec<ProbeResult>),
    /// The timeout passed; the VMM was killed and reaped.
    TimedOut,
    /// The VMM exited without a complete run on its console, with the reason.
    NotRun(String),
}

/// One lane run: the VMM's process ID, its exit status if it exited, how the run ended, and
/// what it wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneRun {
    pub pid: u32,
    pub status: Option<ExitStatus>,
    pub outcome: Outcome,
    pub console: String,
    pub stderr: String,
}

fn wait_until(child: &mut Child, deadline: Instant) -> io::Result<Option<ExitStatus>> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(POLL);
    }
}

/// Spawn `launcher` (normally `lane-vmm`) with `lane` on its standard input, an empty
/// environment and `/` as its directory. Wait up to `timeout`; past it, kill and reap the process
/// and return `TimedOut`. Otherwise parse `lane.console` against `guest`'s run ID and probes.
pub fn run_lane(
    launcher: &Path,
    lane: &LaneConfig,
    guest: &GuestConfig,
    timeout: Duration,
) -> io::Result<LaneRun> {
    if lane.run_id != guest.run_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "the lane is run `{}` and the guest is run `{}`",
                lane.run_id, guest.run_id
            ),
        ));
    }
    match fs::remove_file(&lane.console) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let deadline = Instant::now() + timeout;
    let mut child = Command::new(launcher)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let pid = child.id();
    let mut stderr_pipe = child.stderr.take();
    let stderr_reader = thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_string(&mut text);
        }
        text
    });
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(lane.to_json().as_bytes());
    }
    let status = match wait_until(&mut child, deadline) {
        Ok(status) => status,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    if status.is_none() {
        child.kill()?;
        child.wait()?;
    }
    let stderr = stderr_reader.join().unwrap_or_default();
    let console = fs::read(&lane.console)
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
        .unwrap_or_default();
    let outcome = match status {
        None => Outcome::TimedOut,
        Some(status) => match parse_console(&console, &guest.run_id, &guest.probe_ids()) {
            ConsoleRun::Finished(results) => Outcome::Finished(results),
            ConsoleRun::NotRun(reason) if status.success() => Outcome::NotRun(reason),
            ConsoleRun::NotRun(reason) => {
                Outcome::NotRun(format!("the VMM exited with {status}; {reason}"))
            }
        },
    };
    Ok(LaneRun {
        pid,
        status,
        outcome,
        console,
        stderr,
    })
}
