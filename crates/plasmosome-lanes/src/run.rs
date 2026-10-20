use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::{GuestConfig, LaneConfig};
use crate::console::{ConsoleRun, ProbeResult, parse_console};

const POLL: Duration = Duration::from_millis(10);

/// The most console bytes `run_lane` reads. A longer console is `NotRun`, whatever it holds.
pub const MAX_CONSOLE_BYTES: u64 = 16 * 1024 * 1024;

/// The most stderr bytes a `LaneRun` keeps. The rest is read and dropped, so the launcher never
/// blocks on a full pipe.
pub const MAX_STDERR_BYTES: u64 = 1024 * 1024;

/// How long `run_lane` waits for stderr to close once the launcher's process group is dead. A
/// process that left the group can hold the pipe open; past this, the run keeps what was read.
pub const STDERR_GRACE: Duration = Duration::from_secs(1);

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
/// what it wrote: up to `MAX_CONSOLE_BYTES` of its console and `MAX_STDERR_BYTES` of its stderr.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneRun {
    pub pid: u32,
    pub status: Option<ExitStatus>,
    pub outcome: Outcome,
    pub console: String,
    pub stderr: String,
}

struct StderrReader {
    kept: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

impl StderrReader {
    fn start(pipe: Option<ChildStderr>) -> Self {
        let kept = Arc::new(Mutex::new(Vec::new()));
        let (sender, done) = mpsc::channel();
        let shared = Arc::clone(&kept);
        thread::spawn(move || {
            if let Some(mut pipe) = pipe {
                let mut chunk = [0_u8; 8192];
                loop {
                    match pipe.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(read) => {
                            let mut kept = shared.lock().unwrap_or_else(PoisonError::into_inner);
                            let room = (MAX_STDERR_BYTES as usize).saturating_sub(kept.len());
                            kept.extend(&chunk[..read.min(room)]);
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
            }
            let _ = sender.send(());
        });
        StderrReader { kept, done }
    }

    fn finish(self, grace: Duration) -> String {
        let _ = self.done.recv_timeout(grace);
        let kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&kept).to_string()
    }
}

#[allow(unsafe_code)]
fn kill_group(pid: u32) -> io::Result<()> {
    let group = libc::pid_t::try_from(pid)
        .ok()
        .filter(|group| *group > 1)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{pid} is not a launcher's process group"),
            )
        })?;
    // SAFETY: kill takes two integers and touches no memory of this process. `group` is above
    // 1, so `-group` names one process group, never every process or the caller's own group.
    if unsafe { libc::kill(-group, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(()),
        _ => Err(error),
    }
}

fn stop(child: &mut Child, pid: u32) -> io::Result<()> {
    let group = kill_group(pid);
    let leader = child.kill();
    child.wait()?;
    group.and(leader)
}

fn read_console(path: &Path) -> (String, Option<String>) {
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path);
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return (String::new(), None),
        Err(error) => {
            let reason = format!(
                "the console {} could not be opened without following a link: {error}",
                path.display()
            );
            return (String::new(), Some(reason));
        }
    };
    match file.metadata() {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            let reason = format!("the console {} is not a regular file", path.display());
            return (String::new(), Some(reason));
        }
        Err(error) => {
            let reason = format!("the console {}: {error}", path.display());
            return (String::new(), Some(reason));
        }
    }
    let mut bytes = Vec::new();
    let _ = file.take(MAX_CONSOLE_BYTES + 1).read_to_end(&mut bytes);
    let over = bytes.len() as u64 > MAX_CONSOLE_BYTES;
    bytes.truncate(MAX_CONSOLE_BYTES as usize);
    let text = String::from_utf8_lossy(&bytes).to_string();
    let refusal = over.then(|| format!("the console passed {MAX_CONSOLE_BYTES} bytes"));
    (text, refusal)
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

/// Spawn `launcher` (normally `lane-vmm`) in a new process group, with `lane` on its standard
/// input, an empty environment and `/` as its directory. Wait up to `timeout`; past it, kill the
/// group, reap the launcher and return `TimedOut`. When the launcher exits first, kill whatever is
/// left in its group. Either way, stop reading stderr `STDERR_GRACE` after that kill.
///
/// Then parse `lane.console` against `guest`'s run ID and probes. The console is opened without
/// following a final symlink and without blocking. A console that cannot be opened that way, is
/// not a regular file, or is longer than `MAX_CONSOLE_BYTES` is `NotRun`.
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
    let configuration = lane
        .to_json()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    match fs::remove_file(&lane.console) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let deadline = Instant::now() + timeout;
    let mut child = Command::new(launcher)
        .env_clear()
        .current_dir("/")
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let pid = child.id();
    let stderr_reader = StderrReader::start(child.stderr.take());
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(configuration.as_bytes());
    }
    let status = match wait_until(&mut child, deadline) {
        Ok(Some(status)) => {
            kill_group(pid)?;
            Some(status)
        }
        Ok(None) => {
            stop(&mut child, pid)?;
            None
        }
        Err(error) => {
            let _ = stop(&mut child, pid);
            return Err(error);
        }
    };
    let stderr = stderr_reader.finish(STDERR_GRACE);
    let (console, refusal) = read_console(&lane.console);
    let parsed = match refusal {
        Some(reason) => ConsoleRun::NotRun(reason),
        None => parse_console(&console, &guest.run_id, &guest.probe_ids()),
    };
    let outcome = match status {
        None => Outcome::TimedOut,
        Some(status) => match parsed {
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
