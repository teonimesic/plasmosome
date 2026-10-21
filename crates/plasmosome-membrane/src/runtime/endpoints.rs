use crate::runtime::RemoveOutcome;
use crate::runtime::path::RecipePath;
use crate::socket::OwnedSocketPath;
use std::os::fd::OwnedFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

/// How many bytes of the helper's diagnostics a [`Transcript`] keeps. Later
/// bytes are read and counted, never kept.
pub const TRANSCRIPT_CAP: usize = 1_048_576;

/// A control or data listener the supervisor binds at a recipe path before it
/// launches the helper, and keeps until teardown.
///
/// Get one from [`RuntimeListener::bind`]. Its socket file has mode 0600, and
/// its descriptor is nonblocking and close-on-exec. Closing or dropping it
/// closes the listener, then removes the path only if a no-follow inspection
/// still finds the socket it bound there.
#[derive(Debug)]
pub struct RuntimeListener {
    path: RecipePath,
    listener: UnixListener,
    owned: OwnedSocketPath,
}

/// Why a listener could not be bound. When the refusal returns, no listener
/// is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointRefusal {
    /// Something already exists at `path`: a socket, live or stale, a file,
    /// a directory or a symlink, dangling or not. It was left exactly as it
    /// was.
    AddressInUse { path: RecipePath },
    /// Inspecting `path`, binding the socket or making it nonblocking failed
    /// with `errno`. Nothing this call created remains at `path`.
    Bind { path: RecipePath, errno: i32 },
    /// After the bind, reading what is at `path` without following a symlink
    /// failed with `errno`, or found something other than a socket, when
    /// `errno` is `None`. That entry was left in place, since nothing shows it
    /// is the socket this call bound.
    Identity {
        path: RecipePath,
        errno: Option<i32>,
    },
    /// Setting the socket file to mode 0600 failed with `errno`. The socket
    /// was removed.
    Permissions { path: RecipePath, errno: i32 },
}

impl RuntimeListener {
    /// Binds a listening socket at `path`, which must name nothing yet.
    ///
    /// It never unlinks an entry it did not create: an existing entry of any
    /// kind refuses with [`EndpointRefusal::AddressInUse`] and is left as it
    /// was. The socket is created holding the write side of the spawn lock, so
    /// no `VmmChild::spawn` forks while it is still inheritable. The path is
    /// checked before the bind and the socket's identity is read after it.
    /// Those are separate calls, so pass a path under a directory only this
    /// supervisor can write; this call does not check the directory.
    pub fn bind(path: &RecipePath) -> Result<RuntimeListener, EndpointRefusal> {
        let _ = path;
        todo!()
    }

    /// The path this listener was bound at.
    pub fn path(&self) -> &RecipePath {
        todo!()
    }

    /// Accepts one pending connection without blocking, holding the write
    /// side of the spawn lock, and returns it nonblocking and close-on-exec
    /// with the process id the kernel reports for its peer. Returns `None`
    /// when no connection is pending.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "plasmosome-08k's handshake is the first caller")
    )]
    pub(crate) fn accept_ready(&self) -> std::io::Result<Option<(UnixStream, libc::pid_t)>> {
        todo!()
    }

    /// Closes the listener, then removes its socket file if a no-follow
    /// inspection still finds the socket it bound, and says what it found.
    /// Dropping the listener does the same and discards the outcome.
    pub fn close(self) -> RemoveOutcome {
        todo!()
    }
}

/// The process id the kernel recorded for the peer of `stream` when it
/// connected: `LOCAL_PEERPID` on Darwin, `SO_PEERCRED` on Linux. It is the
/// process that called `connect`, not one that later received the descriptor.
pub fn peer_pid(stream: &UnixStream) -> std::io::Result<libc::pid_t> {
    let _ = stream;
    todo!()
}

/// A thread that reads the helper's diagnostic pipe as fast as it is written,
/// so a helper that writes a lot never blocks on descriptor 2.
///
/// Start it with the read end `HelperExec::prepare` returns as
/// `PreparedHelper::diagnostics`, right after `VmmChild::spawn`, and end it
/// with [`DiagnosticTranscript::finish`]. Dropping it without `finish` stops
/// the thread within one poll interval and discards what it read.
#[derive(Debug)]
pub struct DiagnosticTranscript {
    stop: Arc<AtomicBool>,
    done: mpsc::Receiver<()>,
    thread: Option<JoinHandle<Transcript>>,
}

/// What a [`DiagnosticTranscript`] read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    /// The first bytes read, at most [`TRANSCRIPT_CAP`] of them.
    pub bytes: Vec<u8>,
    /// How many bytes were read after the cap and discarded.
    pub dropped: u64,
    /// The pipe reached end of file: every holder of its write end closed it.
    /// After the helper's process group is killed, that shows no process
    /// still holding descriptor 2 survived.
    pub eof: bool,
}

impl DiagnosticTranscript {
    /// Starts reading `read_end` on a new thread, which owns it from now on.
    /// Fails only when the thread cannot be started.
    pub fn start(read_end: OwnedFd) -> std::io::Result<DiagnosticTranscript> {
        let _ = read_end;
        todo!()
    }

    /// Waits up to `wait` for end of file, then stops the thread and returns
    /// what it read. It returns within `wait` plus one poll interval even when
    /// a process still holds the write end; `eof` is then false.
    pub fn finish(self, wait: Duration) -> Transcript {
        let _ = wait;
        todo!()
    }
}

impl Drop for DiagnosticTranscript {
    fn drop(&mut self) {}
}

impl std::fmt::Display for EndpointRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::error::Error for EndpointRefusal {}
