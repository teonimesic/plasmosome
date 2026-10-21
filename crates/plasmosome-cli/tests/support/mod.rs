use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use plasmosome_cli::Env;
use plasmosome_core::private_socket::{Accepted, PrivateDir, PrivateListener, effective_uid};
use tempfile::TempDir;

pub const PATIENCE: Duration = Duration::from_secs(10);

const POLL: Duration = Duration::from_millis(5);

/// The directory private roots are made in: the canonical per-user temporary directory on macOS,
/// and `XDG_RUNTIME_DIR`, else the target's tmp, on Linux. Every ancestor passes the path checks.
#[cfg(target_os = "macos")]
pub fn base() -> PathBuf {
    let base = std::env::temp_dir();
    base.canonicalize()
        .unwrap_or_else(|error| panic!("{} does not resolve: {error}", base.display()))
}

#[cfg(not(target_os = "macos"))]
pub fn base() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|base| !base.is_empty())
        .map_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")), PathBuf::from)
}

/// A 0700 directory that `check_private_path` accepts: under the canonical per-user temporary
/// directory on macOS, and under `XDG_RUNTIME_DIR`, else the target's tmp, on Linux.
pub fn private_root() -> TempDir {
    let base = base();
    let root = tempfile::Builder::new()
        .prefix("pc")
        .tempdir_in(&base)
        .unwrap_or_else(|error| panic!("no private root under {}: {error}", base.display()));
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))
        .expect("the private root is made 0700");
    root
}

/// The two variables a test gives the command line, in place of the process's own.
#[derive(Clone, Default)]
pub struct Vars {
    pub home: Option<OsString>,
    pub kernel: Option<OsString>,
}

impl Env for Vars {
    fn home(&self) -> Option<OsString> {
        self.home.clone()
    }

    fn kernel(&self) -> Option<OsString> {
        self.kernel.clone()
    }
}

/// What the fake does once it has read a whole request line.
pub enum Answer {
    /// Writes these bytes, then reads until the client closes.
    Reply(Vec<u8>),
    /// Closes the connection.
    Close,
    /// Writes nothing, and reads until the client closes.
    Silent,
    /// Writes bytes with no newline until a write fails.
    Endless,
}

#[derive(Default)]
struct Seen {
    bytes: Vec<u8>,
    ended: bool,
}

/// A daemon on a real private socket. It records every connection and every byte it receives,
/// and gives the next answer to each connection in turn; past the last, it closes.
pub struct FakeDaemon {
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeDaemon {
    pub fn start(path: &Path, answers: Vec<Answer>) -> FakeDaemon {
        let parent = path.parent().expect("a socket path has a parent");
        let dir = PrivateDir::open(parent)
            .unwrap_or_else(|error| panic!("the fake's directory is refused: {error}"));
        let name = path.file_name().and_then(|name| name.to_str());
        let listener = PrivateListener::bind(dir, name.expect("a UTF-8 name"), effective_uid())
            .unwrap_or_else(|error| panic!("the fake cannot bind {}: {error}", path.display()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let (seen, stop) = (Arc::clone(&seen), Arc::clone(&stop));
            move || serve(&listener, answers, &seen, &stop)
        });
        FakeDaemon {
            seen,
            stop,
            thread: Some(thread),
        }
    }

    pub fn connections(&self) -> usize {
        lock(&self.seen).len()
    }

    /// Waits until `count` connections have ended, and returns the bytes each one sent.
    pub fn ended(&self, count: usize) -> Vec<Vec<u8>> {
        let until = Instant::now() + PATIENCE;
        loop {
            let seen = lock(&self.seen);
            if seen.iter().filter(|seen| seen.ended).count() >= count {
                return seen.iter().map(|seen| seen.bytes.clone()).collect();
            }
            assert!(
                Instant::now() < until,
                "{} of {count} connections ended within {PATIENCE:?}",
                seen.iter().filter(|seen| seen.ended).count()
            );
            drop(seen);
            thread::sleep(POLL);
        }
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let Some(thread) = self.thread.take() else {
            return;
        };
        let until = Instant::now() + Duration::from_secs(2);
        while !thread.is_finished() && Instant::now() < until {
            thread::sleep(POLL);
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

fn lock(seen: &Mutex<Vec<Seen>>) -> MutexGuard<'_, Vec<Seen>> {
    seen.lock().unwrap_or_else(PoisonError::into_inner)
}

fn serve(
    listener: &PrivateListener,
    answers: Vec<Answer>,
    seen: &Mutex<Vec<Seen>>,
    stop: &AtomicBool,
) {
    let mut answers = answers.into_iter();
    while !stop.load(Ordering::Relaxed) {
        let Ok(Some(Accepted::Trusted(stream))) = listener.accept() else {
            thread::sleep(POLL);
            continue;
        };
        let index = {
            let mut seen = lock(seen);
            seen.push(Seen::default());
            seen.len() - 1
        };
        let answer = answers.next().unwrap_or(Answer::Close);
        converse(
            stream,
            &answer,
            |bytes| lock(seen)[index].bytes.extend_from_slice(bytes),
            stop,
        );
        lock(seen)[index].ended = true;
    }
}

fn converse(
    mut stream: UnixStream,
    answer: &Answer,
    mut record: impl FnMut(&[u8]),
    stop: &AtomicBool,
) {
    let _ = stream.set_read_timeout(Some(POLL * 4));
    let mut buffer = [0_u8; 4096];
    let mut answered = false;
    while !stop.load(Ordering::Relaxed) {
        let count = match stream.read(&mut buffer) {
            Ok(0) => return,
            Ok(count) => count,
            Err(error) if waiting(error.kind()) => continue,
            Err(_) => return,
        };
        record(&buffer[..count]);
        if answered || !buffer[..count].contains(&b'\n') {
            continue;
        }
        answered = true;
        match answer {
            Answer::Reply(bytes) => {
                let _ = stream.write_all(bytes);
            }
            Answer::Close => return,
            Answer::Silent => {}
            Answer::Endless => return endless(&mut stream, stop),
        }
    }
}

fn endless(stream: &mut UnixStream, stop: &AtomicBool) {
    let chunk = vec![b'x'; 64 * 1024];
    let _ = stream.set_write_timeout(Some(POLL * 4));
    while !stop.load(Ordering::Relaxed) {
        match stream.write(&chunk) {
            Err(error) if !waiting(error.kind()) => return,
            _ => {}
        }
    }
}

fn waiting(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}
