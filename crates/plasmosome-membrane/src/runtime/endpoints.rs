use crate::runtime::RemoveOutcome;
use crate::runtime::path::RecipePath;
use crate::socket::OwnedSocketPath;
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

/// How many bytes of the helper's diagnostics a [`Transcript`] keeps. Later
/// bytes are read and counted, never kept.
pub const TRANSCRIPT_CAP: usize = 1_048_576;

const POLL_INTERVAL: Duration = Duration::from_millis(25);

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

/// One connection [`RuntimeListener::accept_ready`] took from the listener's
/// queue.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "plasmosome-08k's handshake is the first caller")
)]
#[derive(Debug)]
pub(crate) enum Connection {
    /// The stream, nonblocking and close-on-exec, with the process id
    /// [`peer_pid`] read for its peer.
    Identified {
        stream: UnixStream,
        pid: libc::pid_t,
    },
    /// Making the stream nonblocking or reading its peer's process id failed
    /// with `errno`, so the connection was closed. It says nothing about the
    /// listener, which can accept again. On Darwin a peer that closed its end
    /// before the accept gives `ENOTCONN` here; on Linux the same peer is
    /// `Identified` with the pid it had.
    Unidentified { errno: i32 },
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
        let bind_failed = |error: std::io::Error| EndpointRefusal::Bind {
            path: path.clone(),
            errno: errno_of(&error),
        };
        match std::fs::symlink_metadata(path.as_path()) {
            Ok(_) => return Err(EndpointRefusal::AddressInUse { path: path.clone() }),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(bind_failed(error)),
        }
        let listener =
            crate::spawn_lock::with_descriptors_held(|_held| UnixListener::bind(path.as_path()))
                .map_err(|error| match error.raw_os_error() {
                    Some(libc::EADDRINUSE) => EndpointRefusal::AddressInUse { path: path.clone() },
                    _ => bind_failed(error),
                })?;
        let owned = OwnedSocketPath::capture(path.as_path().to_owned()).map_err(|error| {
            EndpointRefusal::Identity {
                path: path.clone(),
                errno: error.raw_os_error(),
            }
        })?;
        restrict_to_owner(path).map_err(|errno| EndpointRefusal::Permissions {
            path: path.clone(),
            errno,
        })?;
        listener.set_nonblocking(true).map_err(bind_failed)?;
        Ok(RuntimeListener {
            path: path.clone(),
            listener,
            owned,
        })
    }

    /// The path this listener was bound at.
    pub fn path(&self) -> &RecipePath {
        &self.path
    }

    /// Accepts one pending connection without blocking, holding the write
    /// side of the spawn lock. It makes the stream nonblocking and reads the
    /// process id [`peer_pid`] reports for its peer right after the accept.
    /// Returns `None` when no connection is pending, and an error only when
    /// the listener's own accept fails.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "plasmosome-08k's handshake is the first caller")
    )]
    pub(crate) fn accept_ready(&self) -> std::io::Result<Option<Connection>> {
        let stream = match crate::spawn_lock::with_descriptors_held(|_held| {
            self.listener.accept().map(|(stream, _)| stream)
        }) {
            Ok(stream) => stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        };
        let identified = stream
            .set_nonblocking(true)
            .and_then(|()| peer_pid(&stream));
        Ok(Some(match identified {
            Ok(pid) => Connection::Identified { stream, pid },
            Err(error) => Connection::Unidentified {
                errno: errno_of(&error),
            },
        }))
    }

    /// Closes the listener, then removes its socket file if a no-follow
    /// inspection still finds the socket it bound, and says what it found.
    /// Dropping the listener does the same and discards the outcome.
    pub fn close(self) -> RemoveOutcome {
        let RuntimeListener {
            listener, owned, ..
        } = self;
        drop(listener);
        owned.remove()
    }
}

#[allow(unsafe_code)]
fn restrict_to_owner(path: &RecipePath) -> Result<(), i32> {
    // SAFETY: The path is a NUL-terminated string `path` keeps alive for the call, and fchmodat
    // keeps no pointer to it.
    let changed = unsafe {
        libc::fchmodat(
            libc::AT_FDCWD,
            path.as_c_str().as_ptr(),
            0o600,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if changed == 0 {
        Ok(())
    } else {
        Err(errno_of(&std::io::Error::last_os_error()))
    }
}

/// The process id the kernel reports for the peer of `stream`. What it names
/// differs by platform.
///
/// On Linux it is `SO_PEERCRED`'s: the process that called `connect`, recorded
/// then. A process that later inherits or receives the peer's descriptor, and
/// sends on it, does not change it.
///
/// On Darwin it is `LOCAL_PEERPID`'s: the last process that created, connected
/// or sent on the peer's socket, as of this call. A process that inherited the
/// connection and sent one byte before the call is the one reported, not the
/// one that connected (measured on macOS 26.6.2). A match with a child's pid
/// then shows only that the child was the last to use that socket.
///
/// When the peer closed its end before the call, Darwin fails with
/// `ENOTCONN`, and Linux still reports the pid the connector had. On neither
/// platform does a pid show that the peer is still running.
#[allow(unsafe_code)]
pub fn peer_pid(stream: &UnixStream) -> std::io::Result<libc::pid_t> {
    #[cfg(target_os = "linux")]
    let (mut found, level, name) = (
        libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        },
        libc::SOL_SOCKET,
        libc::SO_PEERCRED,
    );
    #[cfg(not(target_os = "linux"))]
    let (mut found, level, name): (libc::pid_t, _, _) = (0, libc::SOL_LOCAL, libc::LOCAL_PEERPID);
    let mut length = std::mem::size_of_val(&found) as libc::socklen_t;
    // SAFETY: `found` is a local of `length` bytes that getsockopt may write, and `stream` keeps
    // its descriptor open for the call.
    let got = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            level,
            name,
            (&raw mut found).cast(),
            &mut length,
        )
    };
    if got != 0 {
        return Err(std::io::Error::last_os_error());
    }
    #[cfg(target_os = "linux")]
    let pid = found.pid;
    #[cfg(not(target_os = "linux"))]
    let pid = found;
    Ok(pid)
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
        let stop = Arc::new(AtomicBool::new(false));
        let (ended, done) = mpsc::channel();
        let stopping = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("diagnostic-transcript".to_owned())
            .spawn(move || drain(std::fs::File::from(read_end), &stopping, &ended))?;
        Ok(DiagnosticTranscript {
            stop,
            done,
            thread: Some(thread),
        })
    }

    /// Waits up to `wait` for end of file, then stops the thread and returns
    /// what it read. It returns within `wait` plus one poll interval even when
    /// a process still holds the write end; `eof` is then false.
    pub fn finish(mut self, wait: Duration) -> Transcript {
        let _ = self.done.recv_timeout(wait);
        self.stop.store(true, Ordering::Release);
        self.thread
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default()
    }
}

impl Drop for DiagnosticTranscript {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

enum Taken {
    Bytes(usize),
    Nothing,
    End,
    Failed,
}

fn drain(mut pipe: std::fs::File, stop: &AtomicBool, ended: &mpsc::Sender<()>) -> Transcript {
    let mut transcript = Transcript::default();
    let mut chunk = vec![0u8; 65_536];
    while !stop.load(Ordering::Acquire) {
        if !readable_within(&pipe, POLL_INTERVAL) {
            continue;
        }
        match take(&mut pipe, &mut chunk, &mut transcript) {
            Taken::Bytes(_) | Taken::Nothing => {}
            Taken::End => {
                let _ = ended.send(());
                return transcript;
            }
            Taken::Failed => return transcript,
        }
    }
    let held = bytes_held(&pipe);
    let mut taken = 0;
    while taken <= held && readable_within(&pipe, Duration::ZERO) {
        match take(&mut pipe, &mut chunk, &mut transcript) {
            Taken::Bytes(count) => taken += count,
            Taken::Nothing | Taken::End | Taken::Failed => break,
        }
    }
    transcript
}

fn take(pipe: &mut std::fs::File, chunk: &mut [u8], transcript: &mut Transcript) -> Taken {
    match pipe.read(chunk) {
        Ok(0) => {
            transcript.eof = true;
            Taken::End
        }
        Ok(count) => {
            keep(transcript, &chunk[..count]);
            Taken::Bytes(count)
        }
        Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
            Taken::Nothing
        }
        Err(_) => Taken::Failed,
    }
}

#[allow(unsafe_code)]
fn bytes_held(pipe: &std::fs::File) -> usize {
    let mut held: libc::c_int = 0;
    // SAFETY: FIONREAD writes one c_int to `held`, a local that outlives the call, and `pipe`
    // keeps its descriptor open for the call.
    let asked = unsafe { libc::ioctl(pipe.as_raw_fd(), libc::FIONREAD, &raw mut held) };
    if asked == 0 {
        usize::try_from(held).unwrap_or(0)
    } else {
        0
    }
}

#[allow(unsafe_code)]
fn readable_within(pipe: &std::fs::File, interval: Duration) -> bool {
    let mut watched = libc::pollfd {
        fd: pipe.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let wait_ms = libc::c_int::try_from(interval.as_millis()).unwrap_or(libc::c_int::MAX);
    // SAFETY: `watched` is one local pollfd, matching the count of 1, and `pipe` keeps its
    // descriptor open for the call.
    unsafe { libc::poll(&mut watched, 1, wait_ms) > 0 }
}

fn keep(transcript: &mut Transcript, read: &[u8]) {
    let room = TRANSCRIPT_CAP.saturating_sub(transcript.bytes.len());
    let (kept, over) = read.split_at(read.len().min(room));
    transcript.bytes.extend_from_slice(kept);
    transcript.dropped += over.len() as u64;
}

fn errno_of(error: &std::io::Error) -> i32 {
    error.raw_os_error().unwrap_or(libc::EIO)
}

impl std::fmt::Display for EndpointRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EndpointRefusal::AddressInUse { path } => write!(
                f,
                "{} already exists, so no listener was bound there",
                path.as_str()
            ),
            EndpointRefusal::Bind { path, errno } => write!(
                f,
                "binding a listener at {} failed: errno {errno}",
                path.as_str()
            ),
            EndpointRefusal::Identity {
                path,
                errno: Some(errno),
            } => write!(
                f,
                "the entry bound at {} could not be inspected: errno {errno}",
                path.as_str()
            ),
            EndpointRefusal::Identity { path, errno: None } => write!(
                f,
                "the entry at {} after the bind is not a socket",
                path.as_str()
            ),
            EndpointRefusal::Permissions { path, errno } => write!(
                f,
                "{} could not be made private to its owner: errno {errno}",
                path.as_str()
            ),
        }
    }
}

impl std::error::Error for EndpointRefusal {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::launch::{HelperExec, PreparedHelper};
    use crate::runtime::test_fixtures::{fake, recipe, recipe_value};
    use crate::vmm::{Launch, VmmChild, VmmState};
    use std::fs::File;
    use std::io::{ErrorKind, Write};
    use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::path::PathBuf;
    use std::time::Instant;

    const PATIENCE: Duration = Duration::from_secs(10);

    struct Scratch {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }

    impl Scratch {
        fn new() -> Scratch {
            let dir = tempfile::tempdir().expect("a temporary directory");
            let root = dir
                .path()
                .canonicalize()
                .expect("the temporary directory has a canonical path");
            Scratch { _dir: dir, root }
        }

        fn path(&self, name: &str) -> RecipePath {
            let joined = self.root.join(name);
            RecipePath::parse(joined.to_str().expect("the temporary path is UTF-8"))
                .expect("the temporary path is canonical")
        }
    }

    fn identity(path: &RecipePath) -> (u64, u64) {
        let found = std::fs::symlink_metadata(path.as_path()).expect("the entry is there");
        (found.dev(), found.ino())
    }

    fn is_socket(path: &std::path::Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|found| found.file_type().is_socket())
    }

    fn absent(path: &std::path::Path) -> bool {
        std::fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound)
    }

    #[allow(unsafe_code)]
    fn nonblocking_and_close_on_exec(fd: BorrowedFd<'_>) -> bool {
        // SAFETY: fcntl with F_GETFD or F_GETFL takes no pointer, and the borrow keeps `fd` open.
        let (descriptor, status) = unsafe {
            (
                libc::fcntl(fd.as_raw_fd(), libc::F_GETFD),
                libc::fcntl(fd.as_raw_fd(), libc::F_GETFL),
            )
        };
        descriptor != -1
            && descriptor & libc::FD_CLOEXEC != 0
            && status != -1
            && status & libc::O_NONBLOCK != 0
    }

    fn pipe() -> [OwnedFd; 2] {
        crate::spawn_lock::with_each_descriptor_held(|_held| {
            let (read_end, write_end) = std::io::pipe()?;
            Ok([read_end.into(), write_end.into()])
        })
        .expect("a close-on-exec pipe is created")
    }

    fn pattern(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index % 251) as u8).collect()
    }

    fn accepted_within(listener: &RuntimeListener) -> (UnixStream, libc::pid_t) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match listener.accept_ready().expect("accepting does not fail") {
                Some(Connection::Identified { stream, pid }) => return (stream, pid),
                Some(Connection::Unidentified { errno }) => {
                    panic!("the peer is identified, but errno {errno} came back")
                }
                None => {}
            }
            assert!(
                Instant::now() < deadline,
                "a connection is pending within {PATIENCE:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    struct Connects {
        address: libc::sockaddr_un,
        pauses: bool,
    }

    impl Connects {
        #[allow(unsafe_code)]
        fn to(path: &RecipePath) -> Connects {
            // SAFETY: sockaddr_un is a plain C struct, for which all zeroes is a valid value.
            let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
            assert!(
                path.as_str().len() < address.sun_path.len(),
                "{path:?} fits"
            );
            address.sun_family = libc::AF_UNIX as libc::sa_family_t;
            for (slot, byte) in address.sun_path.iter_mut().zip(path.as_str().bytes()) {
                *slot = byte as libc::c_char;
            }
            Connects {
                address,
                pauses: true,
            }
        }

        fn and_exits(self) -> Connects {
            Connects {
                pauses: false,
                ..self
            }
        }
    }

    impl Launch for Connects {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            let length = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
            // SAFETY: socket, connect, pause and _exit are async-signal-safe and allocate nothing.
            // connect reads `length` bytes of `self.address`, a sockaddr_un this child owns.
            unsafe {
                let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
                if fd >= 0 && libc::connect(fd, (&raw const self.address).cast(), length) == 0 {
                    if self.pauses {
                        loop {
                            libc::pause();
                        }
                    }
                    libc::_exit(0)
                }
                libc::_exit(1)
            }
        }
    }

    struct WritesOnInherited {
        connection: libc::c_int,
        sends: bool,
        sent: libc::c_int,
    }

    impl Launch for WritesOnInherited {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            let byte = b"x";
            // SAFETY: write, pause and _exit are async-signal-safe and allocate nothing. Each
            // write reads one byte of a static, on a descriptor this child inherited.
            unsafe {
                let on_connection =
                    !self.sends || libc::write(self.connection, byte.as_ptr().cast(), 1) == 1;
                if on_connection && libc::write(self.sent, byte.as_ptr().cast(), 1) == 1 {
                    loop {
                        libc::pause();
                    }
                }
                libc::_exit(1)
            }
        }
    }

    #[test]
    fn bind_refuses_every_existing_entry_and_leaves_it_as_it_was() {
        let scratch = Scratch::new();
        let victim = scratch.root.join("victim");
        std::fs::write(&victim, b"victim bytes").unwrap();
        let file = scratch.path("file.sock");
        std::fs::write(file.as_path(), b"someone else's file").unwrap();
        let directory = scratch.path("directory.sock");
        std::fs::create_dir(directory.as_path()).unwrap();
        let dangling = scratch.path("dangling.sock");
        std::os::unix::fs::symlink("nowhere.sock", dangling.as_path()).unwrap();
        let to_victim = scratch.path("victim.sock");
        std::os::unix::fs::symlink(&victim, to_victim.as_path()).unwrap();
        let stale = scratch.path("stale.sock");
        drop(UnixListener::bind(stale.as_path()).unwrap());
        let stale_identity = identity(&stale);

        for path in [&file, &directory, &dangling, &to_victim, &stale] {
            assert_eq!(
                RuntimeListener::bind(path).err(),
                Some(EndpointRefusal::AddressInUse { path: path.clone() }),
                "an existing entry at {path:?} refuses the bind"
            );
        }
        assert_eq!(
            std::fs::read(file.as_path()).unwrap(),
            b"someone else's file"
        );
        assert!(directory.as_path().is_dir(), "the directory stays");
        assert_eq!(
            std::fs::read_link(dangling.as_path()).unwrap(),
            PathBuf::from("nowhere.sock")
        );
        assert!(
            absent(&scratch.root.join("nowhere.sock")),
            "no socket was created at the dangling symlink's target"
        );
        assert_eq!(std::fs::read_link(to_victim.as_path()).unwrap(), victim);
        assert_eq!(std::fs::read(&victim).unwrap(), b"victim bytes");
        assert!(is_socket(stale.as_path()), "the stale socket stays");
        assert_eq!(identity(&stale), stale_identity, "it is the same socket");
    }

    #[test]
    fn a_bind_that_cannot_inspect_or_create_its_path_refuses_with_the_errno() {
        let scratch = Scratch::new();
        std::fs::write(scratch.root.join("file"), b"").unwrap();
        for (name, errno) in [
            ("missing/control.sock", libc::ENOENT),
            ("file/control.sock", libc::ENOTDIR),
        ] {
            let path = scratch.path(name);
            assert_eq!(
                RuntimeListener::bind(&path).err(),
                Some(EndpointRefusal::Bind {
                    path: path.clone(),
                    errno
                })
            );
        }
    }

    #[test]
    fn a_bound_socket_file_is_private_and_its_listener_nonblocking_and_close_on_exec() {
        let scratch = Scratch::new();
        let path = scratch.path("control.sock");
        let listener = RuntimeListener::bind(&path).expect("a free path binds");
        assert_eq!(listener.path(), &path);
        let entry = std::fs::symlink_metadata(path.as_path()).unwrap();
        assert!(entry.file_type().is_socket(), "the entry is a socket");
        assert_eq!(
            entry.permissions().mode() & 0o7777,
            0o600,
            "only the owner may connect"
        );
        assert!(nonblocking_and_close_on_exec(listener.listener.as_fd()));
    }

    #[test]
    fn close_removes_only_the_socket_it_bound() {
        let scratch = Scratch::new();
        let path = scratch.path("control.sock");
        let saved = scratch.root.join("saved.sock");

        let listener = RuntimeListener::bind(&path).unwrap();
        assert_eq!(listener.close(), RemoveOutcome::Removed);
        assert!(absent(path.as_path()), "the socket file is gone");

        let listener = RuntimeListener::bind(&path).unwrap();
        std::fs::rename(path.as_path(), &saved).unwrap();
        std::fs::write(path.as_path(), b"a settled replacement").unwrap();
        assert_eq!(listener.close(), RemoveOutcome::LeftReplacement);
        assert_eq!(
            std::fs::read(path.as_path()).unwrap(),
            b"a settled replacement"
        );
        assert!(is_socket(&saved), "the renamed socket stays");
        std::fs::remove_file(path.as_path()).unwrap();
        std::fs::remove_file(&saved).unwrap();

        let listener = RuntimeListener::bind(&path).unwrap();
        std::fs::rename(path.as_path(), &saved).unwrap();
        let replacement = UnixListener::bind(path.as_path()).unwrap();
        let settled = identity(&path);
        assert_eq!(listener.close(), RemoveOutcome::LeftReplacement);
        assert_eq!(identity(&path), settled, "another socket at the path stays");
        drop(replacement);
        std::fs::remove_file(path.as_path()).unwrap();
        std::fs::remove_file(&saved).unwrap();

        let listener = RuntimeListener::bind(&path).unwrap();
        std::fs::rename(path.as_path(), &saved).unwrap();
        assert_eq!(listener.close(), RemoveOutcome::AlreadyGone);
        assert!(is_socket(&saved), "the renamed socket stays");
    }

    #[test]
    fn dropping_a_listener_removes_its_socket_the_same_way() {
        let scratch = Scratch::new();
        let path = scratch.path("data.sock");

        drop(RuntimeListener::bind(&path).unwrap());
        assert!(absent(path.as_path()), "the socket file is gone");

        let listener = RuntimeListener::bind(&path).unwrap();
        let saved = scratch.root.join("saved.sock");
        std::fs::rename(path.as_path(), &saved).unwrap();
        std::fs::write(path.as_path(), b"a settled replacement").unwrap();
        drop(listener);
        assert_eq!(
            std::fs::read(path.as_path()).unwrap(),
            b"a settled replacement"
        );
        assert!(is_socket(&saved), "the renamed socket stays");
    }

    #[test]
    fn accept_ready_returns_nothing_pending_then_each_peer_with_its_kernel_pid() {
        let scratch = Scratch::new();
        let path = scratch.path("data.sock");
        let listener = RuntimeListener::bind(&path).unwrap();
        assert!(
            listener
                .accept_ready()
                .expect("accepting does not fail")
                .is_none(),
            "nothing is pending"
        );

        let _from_this_process = UnixStream::connect(path.as_path()).unwrap();
        let (stream, pid) = accepted_within(&listener);
        assert_eq!(
            pid,
            libc::pid_t::try_from(std::process::id()).unwrap(),
            "a connection from this process reports this process"
        );
        assert!(nonblocking_and_close_on_exec(stream.as_fd()));

        let child = VmmChild::spawn(Connects::to(&path)).expect("the fork succeeds");
        let (_stream, pid) = accepted_within(&listener);
        assert_eq!(
            pid,
            child.pid(),
            "a forked connector reports its own pid, not this process's"
        );
    }

    #[test]
    fn a_peer_that_left_before_the_accept_is_one_connection_never_pid_0() {
        let scratch = Scratch::new();
        let path = scratch.path("data.sock");
        let listener = RuntimeListener::bind(&path).unwrap();
        let mut child =
            VmmChild::spawn(Connects::to(&path).and_exits()).expect("the fork succeeds");
        assert_eq!(
            child.wait_terminal(PATIENCE),
            Ok(VmmState::Exited { code: 0 }),
            "the child connected, then exited"
        );
        let taken = listener.accept_ready();
        if cfg!(target_os = "linux") {
            assert!(
                matches!(&taken, Ok(Some(Connection::Identified { pid, .. })) if *pid == child.pid()),
                "Linux keeps the pid of a peer that exited: {taken:?}"
            );
        } else {
            assert!(
                matches!(
                    &taken,
                    Ok(Some(Connection::Unidentified {
                        errno: libc::ENOTCONN
                    }))
                ),
                "Darwin cannot read the pid of a peer that exited: {taken:?}"
            );
        }
        assert!(
            matches!(listener.accept_ready(), Ok(None)),
            "that connection was taken from the queue, and the listener still accepts"
        );
    }

    #[test]
    fn the_peer_pid_is_the_connector_on_linux_and_the_last_sender_on_darwin() {
        let scratch = Scratch::new();
        let path = scratch.path("data.sock");
        let listener = RuntimeListener::bind(&path).unwrap();
        let this_process = libc::pid_t::try_from(std::process::id()).unwrap();
        for sends in [false, true] {
            let connection = UnixStream::connect(path.as_path()).unwrap();
            let [ready, sent] = pipe();
            let child = VmmChild::spawn(WritesOnInherited {
                connection: connection.as_raw_fd(),
                sends,
                sent: sent.as_raw_fd(),
            })
            .expect("the fork succeeds");
            assert!(
                readable_within(&File::from(ready), PATIENCE),
                "the child reports within {PATIENCE:?} that it wrote"
            );
            let (mut stream, pid) = accepted_within(&listener);
            let expected = if sends && cfg!(not(target_os = "linux")) {
                child.pid()
            } else {
                this_process
            };
            assert_eq!(
                pid, expected,
                "the child inherited the connection; sends: {sends}"
            );
            let mut byte = [0u8; 1];
            assert_eq!(
                stream.read(&mut byte).map_err(|error| error.kind()),
                if sends {
                    Ok(1)
                } else {
                    Err(ErrorKind::WouldBlock)
                },
                "the child sent before the accept; sends: {sends}"
            );
        }
    }

    #[test]
    fn a_chatty_writer_is_drained_past_the_cap_and_never_blocks() {
        let [read_end, write_end] = pipe();
        let transcript = DiagnosticTranscript::start(read_end).expect("the thread starts");
        let (wrote, written) = mpsc::channel();
        std::thread::spawn(move || {
            let mut writer = File::from(write_end);
            let _ = wrote.send(
                writer
                    .write_all(&pattern(2 * TRANSCRIPT_CAP))
                    .map_err(|e| e.kind()),
            );
        });
        let transcript = transcript.finish(PATIENCE);
        assert_eq!(
            written.recv_timeout(PATIENCE),
            Ok(Ok(())),
            "the writer wrote 2 MiB without blocking for good"
        );
        assert_eq!(transcript.bytes.len(), TRANSCRIPT_CAP);
        assert!(
            transcript.bytes == pattern(TRANSCRIPT_CAP),
            "the first MiB is kept in order"
        );
        assert_eq!(transcript.dropped, TRANSCRIPT_CAP as u64);
        assert!(transcript.eof, "the writer closed the pipe");
    }

    #[test]
    fn finish_returns_within_its_wait_while_a_writer_keeps_the_pipe_open() {
        let [read_end, write_end] = pipe();
        let transcript = DiagnosticTranscript::start(read_end).expect("the thread starts");
        let mut writer = File::from(write_end);
        writer.write_all(b"still running\n").unwrap();
        let (sent, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sent.send(transcript.finish(Duration::from_millis(100)));
        });
        let transcript = finished
            .recv_timeout(Duration::from_secs(1))
            .expect("finish returns within 1 s while the write end is open");
        assert_eq!(
            transcript,
            Transcript {
                bytes: b"still running\n".to_vec(),
                dropped: 0,
                eof: false,
            },
            "a writer still holds the pipe, and what it wrote is kept"
        );
        drop(writer);
    }

    #[test]
    fn finish_without_a_wait_keeps_what_the_pipe_already_holds() {
        for writer_closed in [true, false] {
            let [read_end, write_end] = pipe();
            let mut writer = File::from(write_end);
            writer.write_all(b"last words\n").unwrap();
            let open_writer = if writer_closed {
                drop(writer);
                None
            } else {
                Some(writer)
            };
            let transcript = DiagnosticTranscript::start(read_end)
                .expect("the thread starts")
                .finish(Duration::ZERO);
            assert_eq!(
                transcript,
                Transcript {
                    bytes: b"last words\n".to_vec(),
                    dropped: 0,
                    eof: writer_closed,
                },
                "writer closed: {writer_closed}"
            );
            drop(open_writer);
        }
    }

    #[test]
    fn dropping_a_transcript_closes_its_read_end() {
        let [read_end, write_end] = pipe();
        drop(DiagnosticTranscript::start(read_end).expect("the thread starts"));
        let mut writer = File::from(write_end);
        let deadline = Instant::now() + PATIENCE;
        loop {
            match writer.write(b"x") {
                Err(error) if error.kind() == ErrorKind::BrokenPipe => break,
                Ok(_) => {}
                Err(error) => panic!("writing fails only once the read end is closed: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "the read end is closed within {PATIENCE:?} of the drop"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_record_fakes_whole_report_is_kept_and_ends_in_end_of_file() {
        let helper = fake::runtime_fake("record");
        let recipe = recipe(&recipe_value(
            helper.to_str().expect("the target directory is UTF-8"),
        ));
        let PreparedHelper { exec, diagnostics } =
            HelperExec::prepare(&recipe).expect("the record recipe prepares");
        let mut child = VmmChild::spawn(exec).expect("the fork succeeds");
        let transcript = DiagnosticTranscript::start(diagnostics).expect("the thread starts");
        assert_eq!(
            child.wait_terminal(PATIENCE),
            Ok(VmmState::Exited { code: 0 })
        );
        let transcript = transcript.finish(PATIENCE);
        assert!(
            transcript.eof,
            "the helper's exit closed the last write end"
        );
        assert_eq!(transcript.dropped, 0);
        let report = String::from_utf8(transcript.bytes).expect("the report is text");
        let line: String = recipe
            .to_ndjson()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert!(report.starts_with("argc=2\n"), "{report}");
        assert!(
            report.contains(&format!("\nrecipe_hex={line}\n")),
            "{report}"
        );
        assert!(report.ends_with("\nrecipe_eof=1\n"), "{report}");
    }

    #[test]
    fn each_refusal_names_its_path_and_what_failed() {
        let path = RecipePath::parse("/run/c.sock").unwrap();
        let texts = [
            EndpointRefusal::AddressInUse { path: path.clone() },
            EndpointRefusal::Bind {
                path: path.clone(),
                errno: 13,
            },
            EndpointRefusal::Identity {
                path: path.clone(),
                errno: Some(5),
            },
            EndpointRefusal::Identity {
                path: path.clone(),
                errno: None,
            },
            EndpointRefusal::Permissions { path, errno: 1 },
        ]
        .map(|refusal| refusal.to_string());
        assert_eq!(
            texts,
            [
                "/run/c.sock already exists, so no listener was bound there",
                "binding a listener at /run/c.sock failed: errno 13",
                "the entry bound at /run/c.sock could not be inspected: errno 5",
                "the entry at /run/c.sock after the bind is not a socket",
                "/run/c.sock could not be made private to its owner: errno 1",
            ]
        );
    }

    #[test]
    fn bind_and_accept_wait_while_a_fork_holds_the_spawn_lock() {
        let scratch = Scratch::new();
        let pending = RuntimeListener::bind(&scratch.path("data.sock")).unwrap();
        let _client = UnixStream::connect(pending.path().as_path()).unwrap();
        let binding = scratch.path("control.sock");
        let held = crate::spawn_lock::hold_fork_side();
        let (bound, bound_after) = mpsc::channel();
        let (accepted, accepted_after) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = bound.send(RuntimeListener::bind(&binding).map(drop));
        });
        std::thread::spawn(move || {
            let _ = accepted.send(pending.accept_ready().map(|ready| ready.is_some()));
        });
        std::thread::sleep(Duration::from_millis(250));
        let early = (
            bound_after.try_recv().is_ok(),
            accepted_after.try_recv().is_ok(),
        );
        drop(held);
        assert_eq!(
            early,
            (false, false),
            "(bind, accept) finished while a fork held the spawn lock"
        );
        assert_eq!(bound_after.recv_timeout(PATIENCE), Ok(Ok(())));
        let accepted = accepted_after.recv_timeout(PATIENCE);
        assert!(matches!(accepted, Ok(Ok(true))), "{accepted:?}");
    }
}
