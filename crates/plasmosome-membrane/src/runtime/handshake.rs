use super::digest::Digest;
use super::endpoints::RuntimeListener;
use super::hello::{BootToken, HelloRefusal};
use crate::readiness::{Interest, ProbeBudget};
use crate::vmm::{SupervisionError, VmmState};
use plasmosome_core::private_socket::PrivateSocketError;
use std::os::fd::RawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// The host ends of the shim's three streams after a complete handshake: the
/// normal lane, the withdrawal lane and the 4091 data association, with the
/// boot token both hellos returned, the guest policy they named and the peer
/// process the kernel reported for each stream.
///
/// It proves both hellos and the data association. Spec 001 §4.2 asks more
/// before a cell is ready: the workload namespaces and the workload control
/// group, which this handshake does not observe.
///
/// Keep it for as long as the cell lives. Dropping it closes all three host
/// ends, and the shim then sees each lane closed.
#[derive(Debug)]
pub struct Association {
    /// The boot token both hellos returned.
    pub boot: BootToken,
    /// The guest policy digest both hellos named, equal to the one expected.
    pub policy: Digest,
    /// The helper's process id, which every stream's peer read matched.
    pub peer: libc::pid_t,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "held so the lane stays open until the drop")
    )]
    normal: UnixStream,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "held so the lane stays open until the drop")
    )]
    withdrawal: UnixStream,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "held so the association stays open until the drop"
        )
    )]
    data: UnixStream,
}

/// Which step of the handshake a failure happened at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Accepting the first 4090 connection.
    NormalAccept,
    /// Sending the normal lane's hello and reading its reply.
    NormalReply,
    /// Accepting the second 4090 connection.
    WithdrawalAccept,
    /// Sending the withdrawal lane's hello and reading its reply.
    WithdrawalReply,
    /// Accepting the 4091 data connection.
    DataAccept,
}

/// What the caller's liveness check says about the helper.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Liveness {
    /// The helper has not ended.
    Running,
    /// The helper ended in this state.
    Ended(VmmState),
    /// The helper's state could not be read.
    Unknown(SupervisionError),
}

/// Why the handshake did not complete. Every stream it had accepted is closed
/// when it returns one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HandshakeFailure {
    /// Shutdown was asked for. Cancellation wins when the allowance is spent
    /// at the same check.
    Cancelled { stage: Stage },
    /// The budget's allowance was spent, or the step finished after it.
    TimedOut { stage: Stage },
    /// The liveness check said the helper ended.
    HelperEnded { stage: Stage, state: VmmState },
    /// The liveness check could not tell whether the helper runs.
    LivenessUnknown {
        stage: Stage,
        error: SupervisionError,
    },
    /// A stream's peer is process `found`, not the helper `expected`. A pid of
    /// 0 or below never matches.
    ForeignPeer {
        stage: Stage,
        expected: libc::pid_t,
        found: libc::pid_t,
    },
    /// The listener refused a peer, or the peer's process id could not be
    /// read: a peer of another uid, credentials the kernel would not give, or
    /// a stream that could not be set up.
    PeerRefused {
        stage: Stage,
        refusal: PrivateSocketError,
    },
    /// The hello reply was refused by the judge.
    Refused { stage: Stage, refusal: HelloRefusal },
    /// More than 1,048,576 bytes came before the reply's newline.
    FrameTooLarge { stage: Stage },
    /// Bytes followed the reply's newline in the same read.
    UnexpectedBytes { stage: Stage },
    /// The peer closed its end before the reply's newline.
    Closed { stage: Stage },
    /// An accept, read, write or wait failed with this errno.
    Io { stage: Stage, errno: i32 },
}

/// Runs the host side of the shim's handshake over the cell's two listeners,
/// and returns the [`Association`] once it is complete.
///
/// It accepts the first connection on `control` and checks that its peer is
/// `expected_peer`, the helper's pid. It sends the normal hello and judges the
/// reply against `expected_policy`. It accepts the second connection on
/// `control`, checks its peer and does the same for the withdrawal lane, whose
/// boot token must equal the normal lane's. Only then does it accept one
/// connection on `data` and check its peer. A data connection that came
/// earlier waits in that listener's queue until then.
///
/// Everything spends `budget`'s one allowance, and nothing renews it. Before
/// every wait it checks the budget, then calls `alive`; a helper that ended,
/// or a state it cannot read, fails at once. Waits last at most 25 ms of what
/// is left. A handshake that completes after the allowance is `TimedOut`.
///
/// A peer pid is read from the kernel when each stream is accepted, and again
/// after each hello reply is read. What a match proves differs by platform.
/// On Linux each read is `SO_PEERCRED`'s, the process that called `connect`,
/// so the reads after the replies add nothing. On Darwin each read is
/// `LOCAL_PEERPID`'s, the last process that created, connected or sent on the
/// peer's socket. A match at the accept shows only that the helper was the
/// last to use that socket, not that it connected; a match after a reply
/// shows that the helper was the last to send on it, so a reply sent by
/// another process fails. On neither platform does a pid show that the
/// process still runs.
///
/// Pass the listeners the cell bound before it launched the helper. On a
/// failure this closes what it accepted but not the listeners: a connection
/// still in a listener's queue closes when the caller closes the listener.
pub fn handshake(
    control: &RuntimeListener,
    data: &RuntimeListener,
    expected_peer: libc::pid_t,
    expected_policy: &Digest,
    budget: &ProbeBudget<'_>,
    alive: impl FnMut() -> Liveness,
) -> Result<Association, HandshakeFailure> {
    handshake_with(
        control,
        data,
        Expected {
            peer: expected_peer,
            policy: expected_policy,
        },
        budget,
        alive,
        Instant::now,
        crate::readiness::wait_fd,
    )
}

#[derive(Clone, Copy)]
struct Expected<'a> {
    peer: libc::pid_t,
    policy: &'a Digest,
}

fn handshake_with<A, N, W>(
    control: &RuntimeListener,
    data: &RuntimeListener,
    expected: Expected<'_>,
    budget: &ProbeBudget<'_>,
    alive: A,
    now: N,
    wait: W,
) -> Result<Association, HandshakeFailure>
where
    A: FnMut() -> Liveness,
    N: Fn() -> Instant,
    W: FnMut(RawFd, Interest, Duration) -> std::io::Result<bool>,
{
    let _ = (control, data, expected, budget, alive, now, wait);
    todo!()
}

fn authentic(expected: libc::pid_t, found: libc::pid_t) -> bool {
    let _ = (expected, found);
    todo!()
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::fmt::Display for HandshakeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::error::Error for HandshakeFailure {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readiness::{MAX_RESPONSE_BYTES, MAX_WAIT, wait_fd};
    use crate::runtime::hello::{ControlLane, hello_request, request_id};
    use crate::runtime::path::RecipePath;
    use crate::runtime::test_fixtures::private_root;
    use crate::vmm::{Launch, SupervisionOperation, VmmChild};
    use plasmosome_core::private_socket::effective_uid;
    use std::cell::Cell;
    use std::io::{ErrorKind, Read, Write};
    use std::net::Shutdown;
    use std::os::fd::AsRawFd;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::thread::JoinHandle;

    const PATIENCE: Duration = Duration::from_secs(10);
    const ALLOWANCE: Duration = Duration::from_millis(1000);
    const BOOT: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    const OTHER_BOOT: &str = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";

    fn policy() -> Digest {
        Digest::of(b"guest policy bytes")
    }

    fn boot(text: &str) -> BootToken {
        BootToken::parse_hex(text).expect("a 64-digit lowercase token parses")
    }

    fn this_process() -> libc::pid_t {
        libc::pid_t::try_from(std::process::id()).expect("a pid fits pid_t")
    }

    fn reply(lane: ControlLane, boot: &str, policy: &Digest) -> Vec<u8> {
        format!(
            "{{\"id\":{},\"result\":{{\"boot\":\"{boot}\",\"policy\":\"{}\"}}}}\n",
            request_id(lane),
            policy.hex()
        )
        .into_bytes()
    }

    fn running() -> Liveness {
        Liveness::Running
    }

    struct Endpoints {
        control: RuntimeListener,
        data: RuntimeListener,
        _dir: tempfile::TempDir,
    }

    impl Endpoints {
        fn new() -> Endpoints {
            Endpoints::trusting(None, None)
        }

        fn trusting(control: Option<u32>, data: Option<u32>) -> Endpoints {
            let dir = private_root();
            let root = dir.path().canonicalize().expect("the root resolves");
            let bind = |name: &str, uid: Option<u32>| {
                let path = RecipePath::parse(root.join(name).to_str().expect("UTF-8"))
                    .expect("the temporary path is canonical");
                match uid {
                    None => RuntimeListener::bind(&path),
                    Some(uid) => RuntimeListener::bind_for_uid(&path, uid),
                }
                .expect("a free path binds")
            };
            Endpoints {
                control: bind("control.sock", control),
                data: bind("data.sock", data),
                _dir: dir,
            }
        }

        fn control_path(&self) -> PathBuf {
            self.control.path().as_path().to_owned()
        }

        fn data_path(&self) -> PathBuf {
            self.data.path().as_path().to_owned()
        }

        fn listeners(&self) -> [RawFd; 2] {
            [self.control.as_raw_fd(), self.data.as_raw_fd()]
        }
    }

    struct Shim {
        control: PathBuf,
        data: PathBuf,
        opened: Vec<UnixStream>,
        requests: Vec<Vec<u8>>,
        data_queued: Option<bool>,
    }

    impl Shim {
        fn connect(&mut self, path: &Path) -> usize {
            let stream = UnixStream::connect(path).expect("the shim connects");
            stream
                .set_read_timeout(Some(PATIENCE))
                .expect("a read timeout is set");
            self.opened.push(stream);
            self.opened.len() - 1
        }

        fn control(&mut self) -> usize {
            let path = self.control.clone();
            self.connect(&path)
        }

        fn data(&mut self) -> usize {
            let path = self.data.clone();
            self.connect(&path)
        }

        fn request(&mut self, lane: usize) {
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            while let Ok(1) = self.opened[lane].read(&mut byte) {
                line.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            self.requests.push(line);
        }

        fn send(&mut self, lane: usize, bytes: &[u8]) {
            let _ = self.opened[lane].write_all(bytes);
        }

        fn hello(&mut self, lane: usize, answer: &[u8]) {
            self.request(lane);
            self.send(lane, answer);
        }

        fn received(&mut self, lane: usize, length: usize) -> Vec<u8> {
            let mut bytes = vec![0u8; length];
            self.opened[lane]
                .read_exact(&mut bytes)
                .expect("the bytes arrive");
            bytes
        }

        fn closed_everywhere(&mut self) -> bool {
            self.opened.iter_mut().all(sees_end)
        }
    }

    fn sees_end(stream: &mut UnixStream) -> bool {
        let deadline = Instant::now() + PATIENCE;
        let mut chunk = [0u8; 8192];
        while Instant::now() < deadline {
            match stream.read(&mut chunk) {
                Ok(0) => return true,
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::ConnectionReset => return true,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
        false
    }

    fn spawn_shim(
        endpoints: &Endpoints,
        script: impl FnOnce(&mut Shim) + Send + 'static,
    ) -> JoinHandle<Shim> {
        let (control, data) = (endpoints.control_path(), endpoints.data_path());
        std::thread::spawn(move || {
            let mut shim = Shim {
                control,
                data,
                opened: Vec::new(),
                requests: Vec::new(),
                data_queued: None,
            };
            script(&mut shim);
            shim
        })
    }

    fn both_hellos(shim: &mut Shim) {
        let normal = shim.control();
        shim.hello(normal, &reply(ControlLane::Normal, BOOT, &policy()));
        let withdrawal = shim.control();
        shim.hello(withdrawal, &reply(ControlLane::Withdrawal, BOOT, &policy()));
    }

    fn closed(shim: JoinHandle<Shim>) -> bool {
        shim.join()
            .expect("the shim thread ends")
            .closed_everywhere()
    }

    fn received(stream: &UnixStream, length: usize) -> Vec<u8> {
        stream.set_nonblocking(false).expect("blocking mode is set");
        stream
            .set_read_timeout(Some(PATIENCE))
            .expect("a read timeout is set");
        let mut bytes = vec![0u8; length];
        (&*stream).read_exact(&mut bytes).expect("the bytes arrive");
        bytes
    }

    #[derive(Clone)]
    struct FakeClock {
        base: Instant,
        now_ms: Rc<Cell<u64>>,
    }

    impl FakeClock {
        fn new() -> FakeClock {
            FakeClock {
                base: Instant::now(),
                now_ms: Rc::new(Cell::new(0)),
            }
        }

        fn now(&self) -> impl Fn() -> Instant + use<> {
            let (base, now_ms) = (self.base, Rc::clone(&self.now_ms));
            move || base + Duration::from_millis(now_ms.get())
        }

        fn set(&self, at: Duration) {
            self.now_ms.set(at.as_millis() as u64);
        }

        fn advance(&self, by: Duration) {
            self.now_ms.set(self.now_ms.get() + by.as_millis() as u64);
        }

        fn elapsed(&self) -> Duration {
            Duration::from_millis(self.now_ms.get())
        }

        fn budget<'a>(&self, flag: &'a AtomicBool) -> ProbeBudget<'a> {
            ProbeBudget::new_at(ALLOWANCE, flag, self.base)
        }
    }

    fn real(
        endpoints: &Endpoints,
        flag: &AtomicBool,
        alive: impl FnMut() -> Liveness,
    ) -> Result<Association, HandshakeFailure> {
        handshake(
            &endpoints.control,
            &endpoints.data,
            this_process(),
            &policy(),
            &ProbeBudget::new(PATIENCE, flag),
            alive,
        )
    }

    fn faked(
        endpoints: &Endpoints,
        budget: &ProbeBudget<'_>,
        alive: impl FnMut() -> Liveness,
        now: impl Fn() -> Instant,
        wait: impl FnMut(RawFd, Interest, Duration) -> std::io::Result<bool>,
    ) -> Result<Association, HandshakeFailure> {
        handshake_with(
            &endpoints.control,
            &endpoints.data,
            Expected {
                peer: this_process(),
                policy: &policy(),
            },
            budget,
            alive,
            now,
            wait,
        )
    }

    #[allow(unsafe_code)]
    fn address(path: &Path) -> libc::sockaddr_un {
        // SAFETY: sockaddr_un is a plain C struct, for which all zeroes is a valid value.
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        let bytes = path.as_os_str().as_encoded_bytes();
        assert!(bytes.len() < address.sun_path.len(), "{path:?} fits");
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
            *slot = *byte as libc::c_char;
        }
        address
    }

    #[derive(Clone, Copy)]
    enum Then {
        ReadsToEnd,
        Exits,
    }

    struct Connects {
        address: libc::sockaddr_un,
        then: Then,
    }

    impl Launch for Connects {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            let length = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
            let mut byte = [0u8; 1];
            // SAFETY: socket, connect, read and _exit are async-signal-safe and allocate nothing.
            // connect reads `length` bytes of `self.address`, a sockaddr_un this child owns, and
            // read writes at most one byte into `byte`, a local of one byte.
            unsafe {
                let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
                if fd < 0 || libc::connect(fd, (&raw const self.address).cast(), length) != 0 {
                    libc::_exit(1)
                }
                if let Then::Exits = self.then {
                    libc::_exit(0)
                }
                loop {
                    match libc::read(fd, byte.as_mut_ptr().cast(), 1) {
                        0 => libc::_exit(0),
                        1 => {}
                        _ => libc::_exit(2),
                    }
                }
            }
        }
    }

    #[test]
    fn a_cell_is_ready_after_both_hellos_and_the_data_association() {
        let endpoints = Endpoints::new();
        let shim = spawn_shim(&endpoints, |shim| {
            both_hellos(shim);
            let data = shim.data();
            shim.send(data, b"4091");
        });
        let flag = AtomicBool::new(false);
        let association = real(&endpoints, &flag, running).expect("the handshake completes");
        let mut shim = shim.join().expect("the shim thread ends");
        assert_eq!(association.boot, boot(BOOT));
        assert_eq!(association.policy, policy());
        assert_eq!(association.peer, this_process());
        assert_eq!(
            shim.requests,
            [
                hello_request(ControlLane::Normal).to_vec(),
                hello_request(ControlLane::Withdrawal).to_vec()
            ],
            "the shim saw exactly the two requests, the normal lane's first"
        );
        assert_eq!(
            received(&association.data, 4),
            b"4091",
            "the data stream is the shim's 4091 connection"
        );
        for (lane, stream) in [(0, &association.normal), (1, &association.withdrawal)] {
            (&*stream).write_all(b"lane").expect("the host writes");
            assert_eq!(shim.received(lane, 4), b"lane", "lane {lane} is the shim's");
        }
        drop(association);
        assert!(
            shim.closed_everywhere(),
            "dropping the association closes all three host ends"
        );
    }

    #[test]
    fn an_early_data_connection_waits_for_both_hellos_then_becomes_the_data_association() {
        let endpoints = Endpoints::new();
        let data_listener = endpoints.data.as_raw_fd();
        let shim = spawn_shim(&endpoints, move |shim| {
            let data = shim.data();
            shim.send(data, b"early");
            let normal = shim.control();
            shim.hello(normal, &reply(ControlLane::Normal, BOOT, &policy()));
            let withdrawal = shim.control();
            shim.request(withdrawal);
            shim.data_queued =
                Some(wait_fd(data_listener, Interest::Readable, Duration::ZERO).unwrap_or(false));
            shim.send(withdrawal, &reply(ControlLane::Withdrawal, BOOT, &policy()));
        });
        let flag = AtomicBool::new(false);
        let association = real(&endpoints, &flag, running).expect("the handshake completes");
        let mut shim = shim.join().expect("the shim thread ends");
        assert_eq!(
            shim.data_queued,
            Some(true),
            "the data connection was still queued while the withdrawal hello was answered"
        );
        assert_eq!(
            shim.requests,
            [
                hello_request(ControlLane::Normal).to_vec(),
                hello_request(ControlLane::Withdrawal).to_vec()
            ],
            "both hellos went to the control connections"
        );
        assert_eq!(
            received(&association.data, 5),
            b"early",
            "the early connection is the data association"
        );
        drop(association);
        assert!(shim.closed_everywhere());
    }

    #[test]
    fn both_hellos_without_a_data_connection_time_out_at_the_data_accept() {
        let endpoints = Endpoints::new();
        let shim = spawn_shim(&endpoints, both_hellos);
        let (clock, flag) = (FakeClock::new(), AtomicBool::new(false));
        let advance = clock.clone();
        let data_listener = endpoints.data.as_raw_fd();
        let result = faked(
            &endpoints,
            &clock.budget(&flag),
            running,
            clock.now(),
            |fd, interest, limit| {
                if fd == data_listener {
                    advance.set(ALLOWANCE);
                    return Ok(false);
                }
                wait_fd(fd, interest, limit)
            },
        );
        assert_eq!(
            result.err(),
            Some(HandshakeFailure::TimedOut {
                stage: Stage::DataAccept
            })
        );
        assert!(closed(shim), "both lanes are closed");
    }

    #[test]
    fn a_data_connection_accepted_when_the_allowance_ends_is_timed_out() {
        let endpoints = Endpoints::new();
        let (connect_data, may_connect) = mpsc::channel::<()>();
        let shim = spawn_shim(&endpoints, move |shim| {
            both_hellos(shim);
            if may_connect.recv_timeout(PATIENCE).is_ok() {
                shim.data();
            }
        });
        let (clock, flag) = (FakeClock::new(), AtomicBool::new(false));
        let advance = clock.clone();
        let data_listener = endpoints.data.as_raw_fd();
        let result = faked(
            &endpoints,
            &clock.budget(&flag),
            running,
            clock.now(),
            |fd, interest, limit| {
                if fd != data_listener {
                    return wait_fd(fd, interest, limit);
                }
                let _ = connect_data.send(());
                let ready = wait_fd(fd, interest, PATIENCE)?;
                advance.set(ALLOWANCE);
                Ok(ready)
            },
        );
        assert_eq!(
            result.err(),
            Some(HandshakeFailure::TimedOut {
                stage: Stage::DataAccept
            }),
            "a handshake that completes at the end of its allowance is late"
        );
        assert!(closed(shim), "all three streams are closed");
    }

    #[test]
    fn a_helper_that_already_ended_fails_before_any_wait() {
        let endpoints = Endpoints::new();
        let (clock, flag) = (FakeClock::new(), AtomicBool::new(false));
        let advance = clock.clone();
        let result = faked(
            &endpoints,
            &clock.budget(&flag),
            || Liveness::Ended(VmmState::Exited { code: 66 }),
            clock.now(),
            |_, _, limit| {
                advance.advance(limit);
                Ok(false)
            },
        );
        assert_eq!(
            result.err(),
            Some(HandshakeFailure::HelperEnded {
                stage: Stage::NormalAccept,
                state: VmmState::Exited { code: 66 }
            })
        );
        assert!(
            clock.elapsed() < MAX_WAIT,
            "no wait ran: {:?}",
            clock.elapsed()
        );
    }

    #[test]
    fn a_helper_that_ends_while_its_reply_is_awaited_fails_at_the_reply() {
        let endpoints = Endpoints::new();
        let shim = spawn_shim(&endpoints, |shim| {
            let normal = shim.control();
            shim.request(normal);
        });
        let (clock, flag) = (FakeClock::new(), AtomicBool::new(false));
        let lane_waited = Rc::new(Cell::new(false));
        let ended = Rc::clone(&lane_waited);
        let listeners = endpoints.listeners();
        let result = faked(
            &endpoints,
            &clock.budget(&flag),
            move || {
                if ended.get() {
                    Liveness::Ended(VmmState::Signaled { signal: 9 })
                } else {
                    Liveness::Running
                }
            },
            clock.now(),
            |fd, interest, limit| {
                if !listeners.contains(&fd) {
                    lane_waited.set(true);
                }
                wait_fd(fd, interest, limit)
            },
        );
        assert_eq!(
            result.err(),
            Some(HandshakeFailure::HelperEnded {
                stage: Stage::NormalReply,
                state: VmmState::Signaled { signal: 9 }
            })
        );
        assert!(closed(shim), "the normal lane is closed");
    }

    #[test]
    fn an_unknown_liveness_fails_closed() {
        let endpoints = Endpoints::new();
        let error = SupervisionError {
            operation: SupervisionOperation::Observe,
            errno: libc::EINVAL,
            observed: None,
        };
        let flag = AtomicBool::new(false);
        assert_eq!(
            real(&endpoints, &flag, || Liveness::Unknown(error)).err(),
            Some(HandshakeFailure::LivenessUnknown {
                stage: Stage::NormalAccept,
                error
            })
        );
    }

    #[test]
    fn a_connection_from_another_process_is_a_foreign_peer_and_is_closed() {
        let endpoints = Endpoints::new();
        let mut child = VmmChild::spawn(Connects {
            address: address(&endpoints.control_path()),
            then: Then::ReadsToEnd,
        })
        .expect("the fork succeeds");
        let flag = AtomicBool::new(false);
        assert_eq!(
            real(&endpoints, &flag, running).err(),
            Some(HandshakeFailure::ForeignPeer {
                stage: Stage::NormalAccept,
                expected: this_process(),
                found: child.pid()
            })
        );
        assert_eq!(
            child.wait_terminal(PATIENCE),
            Ok(VmmState::Exited { code: 0 }),
            "the stranger read end of file, so its stream was closed"
        );
    }

    #[test]
    fn a_peer_pid_of_0_or_below_never_matches() {
        assert!(authentic(42, 42));
        assert!(!authentic(42, 43));
        assert!(!authentic(42, 0), "pid 0 is not the helper");
        assert!(!authentic(0, 0), "pid 0 never matches, even when expected");
        assert!(!authentic(-1, -1), "a negative pid never matches");
    }

    #[test]
    fn a_peer_of_another_uid_is_refused_at_the_accept_of_either_listener() {
        let stranger = effective_uid().wrapping_add(1);
        let mismatch = PrivateSocketError::PeerMismatch {
            trusted: stranger,
            found: effective_uid(),
        };
        let flag = AtomicBool::new(false);

        let endpoints = Endpoints::trusting(Some(stranger), None);
        let mut client = UnixStream::connect(endpoints.control_path()).expect("connects");
        client
            .set_read_timeout(Some(PATIENCE))
            .expect("a read timeout is set");
        assert_eq!(
            real(&endpoints, &flag, running).err(),
            Some(HandshakeFailure::PeerRefused {
                stage: Stage::NormalAccept,
                refusal: mismatch.clone()
            })
        );
        assert!(sees_end(&mut client), "the refused peer's stream is closed");

        let endpoints = Endpoints::trusting(None, Some(stranger));
        let shim = spawn_shim(&endpoints, |shim| {
            both_hellos(shim);
            shim.data();
        });
        assert_eq!(
            real(&endpoints, &flag, running).err(),
            Some(HandshakeFailure::PeerRefused {
                stage: Stage::DataAccept,
                refusal: mismatch
            })
        );
        assert!(closed(shim), "every stream the shim opened is closed");
    }

    #[test]
    fn a_connector_that_left_before_the_accept_is_refused_on_darwin_and_foreign_on_linux() {
        let endpoints = Endpoints::new();
        let mut child = VmmChild::spawn(Connects {
            address: address(&endpoints.control_path()),
            then: Then::Exits,
        })
        .expect("the fork succeeds");
        assert_eq!(
            child.wait_terminal(PATIENCE),
            Ok(VmmState::Exited { code: 0 }),
            "the child connected, then exited"
        );
        let expected = if cfg!(target_os = "linux") {
            HandshakeFailure::ForeignPeer {
                stage: Stage::NormalAccept,
                expected: this_process(),
                found: child.pid(),
            }
        } else {
            HandshakeFailure::PeerRefused {
                stage: Stage::NormalAccept,
                refusal: PrivateSocketError::PeerCredentials {
                    errno: libc::ENOTCONN,
                },
            }
        };
        let flag = AtomicBool::new(false);
        assert_eq!(real(&endpoints, &flag, running).err(), Some(expected));
    }

    #[cfg(target_os = "macos")]
    struct RepliesOnInherited {
        connection: libc::c_int,
        reply: *const u8,
        length: usize,
    }

    #[cfg(target_os = "macos")]
    impl Launch for RepliesOnInherited {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            let mut request = [0u8; 256];
            let mut sent = 0;
            // SAFETY: read, write, pause and _exit are async-signal-safe and allocate nothing. read
            // writes at most 256 bytes into `request`, a local of 256 bytes. Each write reads
            // bytes `sent..length` of `reply`, a buffer the parent made before the fork, which
            // this child's copy of the address space still holds.
            unsafe {
                if libc::read(self.connection, request.as_mut_ptr().cast(), request.len()) <= 0 {
                    libc::_exit(1)
                }
                while sent < self.length {
                    let wrote = libc::write(
                        self.connection,
                        self.reply.add(sent).cast(),
                        self.length - sent,
                    );
                    if wrote <= 0 {
                        libc::_exit(2)
                    }
                    sent += wrote as usize;
                }
                loop {
                    libc::pause();
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_darwin_a_hello_reply_sent_by_another_process_is_a_foreign_peer() {
        let endpoints = Endpoints::new();
        let connection = UnixStream::connect(endpoints.control_path()).expect("connects");
        let answer = reply(ControlLane::Normal, BOOT, &policy());
        let child = VmmChild::spawn(RepliesOnInherited {
            connection: connection.as_raw_fd(),
            reply: answer.as_ptr(),
            length: answer.len(),
        })
        .expect("the fork succeeds");
        let flag = AtomicBool::new(false);
        assert_eq!(
            real(&endpoints, &flag, running).err(),
            Some(HandshakeFailure::ForeignPeer {
                stage: Stage::NormalReply,
                expected: this_process(),
                found: child.pid()
            }),
            "this process connected, but another process sent the reply"
        );
        drop(connection);
    }

    #[test]
    fn a_reply_at_the_bound_is_judged_and_one_byte_more_is_refused_before_parsing() {
        let padded = |length: usize| {
            let mut line = reply(ControlLane::Normal, BOOT, &policy());
            line.pop();
            line.resize(length, b' ');
            line.push(b'\n');
            line
        };
        let flag = AtomicBool::new(false);

        let endpoints = Endpoints::new();
        let at_bound = padded(MAX_RESPONSE_BYTES);
        let shim = spawn_shim(&endpoints, move |shim| {
            let normal = shim.control();
            shim.hello(normal, &at_bound);
            let withdrawal = shim.control();
            shim.hello(withdrawal, &reply(ControlLane::Withdrawal, BOOT, &policy()));
            shim.data();
        });
        let association = real(&endpoints, &flag, running);
        assert!(
            association.is_ok(),
            "a reply of exactly 1,048,576 bytes before its newline is judged and accepted"
        );
        drop(association);
        assert!(closed(shim));

        let endpoints = Endpoints::new();
        let over = padded(MAX_RESPONSE_BYTES + 1);
        let shim = spawn_shim(&endpoints, move |shim| {
            let normal = shim.control();
            shim.hello(normal, &over);
        });
        assert_eq!(
            real(&endpoints, &flag, running).err(),
            Some(HandshakeFailure::FrameTooLarge {
                stage: Stage::NormalReply
            }),
            "one byte more is refused, although the same JSON was accepted"
        );
        assert!(closed(shim));
    }

    #[test]
    fn a_reply_cut_short_or_followed_by_more_bytes_is_refused() {
        let flag = AtomicBool::new(false);

        let endpoints = Endpoints::new();
        let shim = spawn_shim(&endpoints, |shim| {
            let normal = shim.control();
            shim.hello(normal, &reply(ControlLane::Normal, BOOT, &policy())[..20]);
            let _ = shim.opened[normal].shutdown(Shutdown::Write);
        });
        assert_eq!(
            real(&endpoints, &flag, running).err(),
            Some(HandshakeFailure::Closed {
                stage: Stage::NormalReply
            })
        );
        assert!(closed(shim));

        let endpoints = Endpoints::new();
        let shim = spawn_shim(&endpoints, |shim| {
            let normal = shim.control();
            let mut answer = reply(ControlLane::Normal, BOOT, &policy());
            answer.extend_from_slice(b"{\"id\":1}\n");
            shim.hello(normal, &answer);
        });
        assert_eq!(
            real(&endpoints, &flag, running).err(),
            Some(HandshakeFailure::UnexpectedBytes {
                stage: Stage::NormalReply
            })
        );
        assert!(closed(shim));
    }

    #[test]
    fn a_reply_trickled_one_byte_per_step_times_out_inside_the_one_allowance() {
        let endpoints = Endpoints::new();
        let (step, steps) = mpsc::channel::<()>();
        let shim = spawn_shim(&endpoints, move |shim| {
            let normal = shim.control();
            shim.request(normal);
            for byte in reply(ControlLane::Normal, BOOT, &policy()) {
                if steps.recv_timeout(PATIENCE).is_err() {
                    break;
                }
                shim.send(normal, &[byte]);
            }
        });
        let (clock, flag) = (FakeClock::new(), AtomicBool::new(false));
        let advance = clock.clone();
        let listeners = endpoints.listeners();
        let result = faked(
            &endpoints,
            &clock.budget(&flag),
            running,
            clock.now(),
            |fd, interest, limit| {
                if listeners.contains(&fd) {
                    let ready = wait_fd(fd, interest, PATIENCE)?;
                    if !ready {
                        advance.set(ALLOWANCE);
                    }
                    return Ok(ready);
                }
                advance.advance(limit);
                let _ = step.send(());
                wait_fd(fd, interest, PATIENCE)
            },
        );
        assert_eq!(
            result.err(),
            Some(HandshakeFailure::TimedOut {
                stage: Stage::NormalReply
            }),
            "a trickle cannot renew the allowance"
        );
        assert!(clock.elapsed() >= ALLOWANCE);
        assert!(closed(shim));
    }

    #[test]
    fn shutdown_while_waiting_is_cancelled_even_when_the_allowance_is_spent_at_the_same_check() {
        for spent_too in [false, true] {
            let endpoints = Endpoints::new();
            let (clock, flag) = (FakeClock::new(), AtomicBool::new(false));
            let advance = clock.clone();
            let result = faked(
                &endpoints,
                &clock.budget(&flag),
                running,
                clock.now(),
                |_, _, _| {
                    flag.store(true, Ordering::Relaxed);
                    if spent_too {
                        advance.set(ALLOWANCE);
                    }
                    Ok(false)
                },
            );
            assert_eq!(
                result.err(),
                Some(HandshakeFailure::Cancelled {
                    stage: Stage::NormalAccept
                }),
                "allowance spent too: {spent_too}"
            );
        }

        let endpoints = Endpoints::new();
        let shim = spawn_shim(&endpoints, |shim| {
            let normal = shim.control();
            shim.request(normal);
        });
        let (clock, flag) = (FakeClock::new(), AtomicBool::new(false));
        let listeners = endpoints.listeners();
        let result = faked(
            &endpoints,
            &clock.budget(&flag),
            running,
            clock.now(),
            |fd, interest, limit| {
                if !listeners.contains(&fd) {
                    flag.store(true, Ordering::Relaxed);
                }
                wait_fd(fd, interest, limit)
            },
        );
        assert_eq!(
            result.err(),
            Some(HandshakeFailure::Cancelled {
                stage: Stage::NormalReply
            })
        );
        assert!(closed(shim));
    }

    #[test]
    fn a_wrong_policy_or_a_changed_boot_is_refused_at_its_reply() {
        let flag = AtomicBool::new(false);
        let other = Digest::of(b"another policy");

        let endpoints = Endpoints::new();
        let shim = spawn_shim(&endpoints, move |shim| {
            let normal = shim.control();
            shim.hello(normal, &reply(ControlLane::Normal, BOOT, &other));
        });
        assert_eq!(
            real(&endpoints, &flag, running).err(),
            Some(HandshakeFailure::Refused {
                stage: Stage::NormalReply,
                refusal: HelloRefusal::PolicyMismatch {
                    expected: policy(),
                    found: other
                }
            })
        );
        assert!(closed(shim));

        let endpoints = Endpoints::new();
        let shim = spawn_shim(&endpoints, |shim| {
            let normal = shim.control();
            shim.hello(normal, &reply(ControlLane::Normal, BOOT, &policy()));
            let withdrawal = shim.control();
            shim.hello(
                withdrawal,
                &reply(ControlLane::Withdrawal, OTHER_BOOT, &policy()),
            );
        });
        assert_eq!(
            real(&endpoints, &flag, running).err(),
            Some(HandshakeFailure::Refused {
                stage: Stage::WithdrawalReply,
                refusal: HelloRefusal::BootChanged {
                    expected: boot(BOOT),
                    found: boot(OTHER_BOOT)
                }
            })
        );
        assert!(closed(shim), "both lanes are closed");
    }

    #[test]
    fn each_failure_names_its_stage_and_cause() {
        let stage = Stage::WithdrawalReply;
        let texts = [
            HandshakeFailure::Cancelled { stage },
            HandshakeFailure::TimedOut { stage },
            HandshakeFailure::HelperEnded {
                stage,
                state: VmmState::Exited { code: 66 },
            },
            HandshakeFailure::LivenessUnknown {
                stage,
                error: SupervisionError {
                    operation: SupervisionOperation::Observe,
                    errno: libc::EINVAL,
                    observed: None,
                },
            },
            HandshakeFailure::ForeignPeer {
                stage,
                expected: 10,
                found: 11,
            },
            HandshakeFailure::PeerRefused {
                stage,
                refusal: PrivateSocketError::PeerCredentials { errno: 57 },
            },
            HandshakeFailure::Refused {
                stage,
                refusal: HelloRefusal::NotUtf8,
            },
            HandshakeFailure::FrameTooLarge { stage },
            HandshakeFailure::UnexpectedBytes { stage },
            HandshakeFailure::Closed { stage },
            HandshakeFailure::Io { stage, errno: 5 },
        ]
        .map(|failure| failure.to_string());
        let peer_refusal = PrivateSocketError::PeerCredentials { errno: 57 }.to_string();
        let supervision = SupervisionError {
            operation: SupervisionOperation::Observe,
            errno: libc::EINVAL,
            observed: None,
        }
        .to_string();
        assert_eq!(
            texts,
            [
                "at the withdrawal lane's hello reply, shutdown cancelled the handshake".to_owned(),
                "at the withdrawal lane's hello reply, the handshake's allowance was spent"
                    .to_owned(),
                "at the withdrawal lane's hello reply, the helper had ended: Exited { code: 66 }"
                    .to_owned(),
                format!(
                    "at the withdrawal lane's hello reply, the helper's state could not be read: \
                     {supervision}"
                ),
                "at the withdrawal lane's hello reply, the peer was process 11, not the helper \
                 10"
                .to_owned(),
                format!(
                    "at the withdrawal lane's hello reply, the peer was refused: {peer_refusal}"
                ),
                "at the withdrawal lane's hello reply, the reply was refused: the hello reply is \
                 not UTF-8"
                    .to_owned(),
                "at the withdrawal lane's hello reply, more than 1048576 bytes came before the \
                 newline"
                    .to_owned(),
                "at the withdrawal lane's hello reply, bytes followed the reply's newline"
                    .to_owned(),
                "at the withdrawal lane's hello reply, the peer closed the stream".to_owned(),
                "at the withdrawal lane's hello reply, a call failed with errno 5".to_owned(),
            ]
        );
        assert_eq!(
            [
                Stage::NormalAccept,
                Stage::NormalReply,
                Stage::WithdrawalAccept,
                Stage::WithdrawalReply,
                Stage::DataAccept
            ]
            .map(|stage| stage.to_string()),
            [
                "the normal lane's accept",
                "the normal lane's hello reply",
                "the withdrawal lane's accept",
                "the withdrawal lane's hello reply",
                "the data association's accept",
            ]
        );
    }
}
