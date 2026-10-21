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
            if let Some(accepted) = listener.accept_ready().expect("accepting does not fail") {
                return accepted;
            }
            assert!(
                Instant::now() < deadline,
                "a connection is pending within {PATIENCE:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    struct ConnectsAndPauses {
        address: libc::sockaddr_un,
    }

    impl ConnectsAndPauses {
        #[allow(unsafe_code)]
        fn to(path: &RecipePath) -> ConnectsAndPauses {
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
            ConnectsAndPauses { address }
        }
    }

    impl Launch for ConnectsAndPauses {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            let length = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
            // SAFETY: socket, connect, pause and _exit are async-signal-safe and allocate nothing.
            // connect reads `length` bytes of `self.address`, a sockaddr_un this child owns.
            unsafe {
                let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
                if fd >= 0 && libc::connect(fd, (&raw const self.address).cast(), length) == 0 {
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

        let child = VmmChild::spawn(ConnectsAndPauses::to(&path)).expect("the fork succeeds");
        let (_stream, pid) = accepted_within(&listener);
        assert_eq!(
            pid,
            child.pid(),
            "a forked connector reports its own pid, not this process's"
        );
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
        assert!(!transcript.eof, "a writer still holds the pipe");
        assert!(
            b"still running\n".starts_with(&transcript.bytes),
            "{transcript:?}"
        );
        assert_eq!(transcript.dropped, 0);
        drop(writer);
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
        let path = RecipePath::parse("/run/cell/control.sock").unwrap();
        for (refusal, text) in [
            (
                EndpointRefusal::AddressInUse { path: path.clone() },
                "/run/cell/control.sock already exists, so no listener was bound there",
            ),
            (
                EndpointRefusal::Bind {
                    path: path.clone(),
                    errno: libc::EACCES,
                },
                &format!(
                    "binding a listener at /run/cell/control.sock failed: errno {}",
                    libc::EACCES
                ),
            ),
            (
                EndpointRefusal::Identity {
                    path: path.clone(),
                    errno: Some(libc::EIO),
                },
                &format!(
                    "the entry bound at /run/cell/control.sock could not be inspected: errno {}",
                    libc::EIO
                ),
            ),
            (
                EndpointRefusal::Identity {
                    path: path.clone(),
                    errno: None,
                },
                "the entry at /run/cell/control.sock after the bind is not a socket",
            ),
            (
                EndpointRefusal::Permissions {
                    path: path.clone(),
                    errno: libc::EPERM,
                },
                &format!(
                    "/run/cell/control.sock could not be made private to its owner: errno {}",
                    libc::EPERM
                ),
            ),
        ] {
            assert_eq!(refusal.to_string(), text);
        }
    }
}
