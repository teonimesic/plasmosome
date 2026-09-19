use std::io::{ErrorKind, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The most raw bytes a broker status response may hold before its terminating
/// newline: the same magnitude as the §1 request cap, so one framing limit is
/// not replaced by two. Every byte counts, whitespace and CR included, before
/// UTF-8 decoding or trimming; the newline itself does not.
pub const MAX_RESPONSE_BYTES: usize = 1_048_576;

const DIAGNOSTIC_BYTES: usize = 1024;

const READ_CHUNK: usize = 8 * 1024;

const MAX_WAIT: Duration = Duration::from_millis(25);

const STATUS_REQUEST: &[u8] = b"{\"id\":0,\"method\":\"membrane.status\",\"params\":{}}\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    Ready { state: String },
    NotReady(NotReady),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotReady {
    Unreachable { path: PathBuf },
    TimedOut,
    Malformed { line: String },
    Reported { state: String },
}

impl Readiness {
    pub fn is_ready(&self) -> bool {
        matches!(self, Readiness::Ready { .. })
    }
}

/// One fixed elapsed-time allowance for one readiness query.
/// The clock starts once and never renews: connection establishment, request
/// transmission, reply acquisition and classification all spend the same
/// allowance, so a broker cannot stretch a query by trickling bytes, dribbling
/// an oversized reply, or withholding the frame terminator. Read it only
/// through [`ProbeBudget::remaining`]; nothing else observes time.
pub struct ProbeBudget<'a> {
    start: Instant,
    allowance: Duration,
    shutdown: &'a AtomicBool,
}

/// Why a probe stopped without an answer: the allowance was spent, or the
/// daemon is shutting down. Cancellation wins whenever both are observed at
/// the same check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStopped {
    TimedOut,
    Cancelled,
}

/// Cancellation is not a broker verdict and not a wire state: it means the
/// daemon itself is going away, so the query has no answer to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the readiness query was cancelled by shutdown")
    }
}

impl std::error::Error for Cancelled {}

impl<'a> ProbeBudget<'a> {
    pub fn new(allowance: Duration, shutdown: &'a AtomicBool) -> ProbeBudget<'a> {
        Self::new_at(allowance, shutdown, Instant::now())
    }

    pub(crate) fn new_at(
        allowance: Duration,
        shutdown: &'a AtomicBool,
        start: Instant,
    ) -> ProbeBudget<'a> {
        ProbeBudget {
            start,
            allowance,
            shutdown,
        }
    }

    /// How much of the allowance is left, or why the query must stop.
    ///
    /// Time is only ever subtracted from the allowance, so an extreme
    /// configuration cannot overflow an instant.
    pub fn remaining(&self) -> Result<Duration, ProbeStopped> {
        self.remaining_at(Instant::now())
    }

    pub(crate) fn remaining_at(&self, now: Instant) -> Result<Duration, ProbeStopped> {
        if self.shutdown.load(Ordering::Relaxed) {
            return Err(ProbeStopped::Cancelled);
        }
        self.allowance
            .checked_sub(now.duration_since(self.start))
            .filter(|left| !left.is_zero())
            .ok_or(ProbeStopped::TimedOut)
    }
}

/// Asks one broker socket whether it is serving, inside `budget`.
///
/// Time expiry is `Ok` with [`NotReady::TimedOut`]; cancellation is
/// `Err(Cancelled)`. A socket that cannot be reached at all is `Ok` with
/// [`NotReady::Unreachable`]. The socket is connected nonblockingly and stays
/// nonblocking, so no single step can outlive the budget unnoticed: whatever a
/// connect or a reply costs is paid out of the one allowance, and a connect
/// that comes back after it is discarded. Waits are requested in slices of at
/// most 25ms of what is left; a full nonblocking backlog and an interrupted
/// connect establish nothing and report as timed out. The reply is the first
/// newline-terminated frame within [`MAX_RESPONSE_BYTES`], everything after
/// that first newline is ignored, and no verdict that finished after the
/// allowance is accepted — a late answer reports as timed out, never ready.
pub fn probe(socket: &Path, budget: &ProbeBudget<'_>) -> Result<Readiness, Cancelled> {
    probe_with(
        socket,
        budget,
        |path| connect_budgeted(path, budget),
        Instant::now,
        wait_fd,
    )
}

fn probe_with<S, C, N, W>(
    socket: &Path,
    budget: &ProbeBudget<'_>,
    connect: C,
    now: N,
    wait: W,
) -> Result<Readiness, Cancelled>
where
    S: Read + Write + AsRawFd,
    C: FnOnce(&Path) -> Connected<S>,
    N: Fn() -> Instant,
    W: FnMut(RawFd, Interest, Duration) -> std::io::Result<bool>,
{
    match budget.remaining_at(now()) {
        Ok(_) => {}
        Err(ProbeStopped::Cancelled) => return Err(Cancelled),
        Err(ProbeStopped::TimedOut) => return Ok(spent()),
    }
    let stream = match connect(socket) {
        Connected::Yes(stream) => stream,
        Connected::Unreachable => {
            return Ok(Readiness::NotReady(NotReady::Unreachable {
                path: socket.to_path_buf(),
            }));
        }
        Connected::Stopped(ProbeStopped::Cancelled) => return Err(Cancelled),
        Connected::Stopped(ProbeStopped::TimedOut) => return Ok(spent()),
    };
    match budget.remaining_at(now()) {
        Ok(_) => {}
        Err(ProbeStopped::Cancelled) => return Err(Cancelled),
        Err(ProbeStopped::TimedOut) => return Ok(spent()),
    }
    exchange(stream, STATUS_REQUEST, budget, now, wait)
}

fn exchange<S, N, W>(
    mut stream: S,
    request: &[u8],
    budget: &ProbeBudget<'_>,
    now: N,
    mut wait: W,
) -> Result<Readiness, Cancelled>
where
    S: Read + Write + AsRawFd,
    N: Fn() -> Instant,
    W: FnMut(RawFd, Interest, Duration) -> std::io::Result<bool>,
{
    let fd = stream.as_raw_fd();
    let mut written = 0;
    while written < request.len() {
        match budget.remaining_at(now()) {
            Ok(_) => {}
            Err(ProbeStopped::Cancelled) => return Err(Cancelled),
            Err(ProbeStopped::TimedOut) => return Ok(spent()),
        };
        match stream.write(&request[written..]) {
            Ok(0) => return Ok(spent()),
            Ok(sent) => written += sent,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if !wait_ready(fd, Interest::Writable, budget, &now, &mut wait)? {
                    return Ok(spent());
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Ok(spent()),
        }
    }

    let mut frame: Vec<u8> = Vec::new();
    let mut chunk = [0u8; READ_CHUNK];
    loop {
        match budget.remaining_at(now()) {
            Ok(_) => {}
            Err(ProbeStopped::Cancelled) => return Err(Cancelled),
            Err(ProbeStopped::TimedOut) => return Ok(spent()),
        };
        let room = MAX_RESPONSE_BYTES + 1 - frame.len();
        match stream.read(&mut chunk[..READ_CHUNK.min(room)]) {
            Ok(0) => return Ok(spent()),
            Ok(read) => {
                match budget.remaining_at(now()) {
                    Ok(_) => {}
                    Err(ProbeStopped::Cancelled) => return Err(Cancelled),
                    Err(ProbeStopped::TimedOut) => return Ok(spent()),
                }
                let new_bytes = &chunk[..read];
                if let Some(end) = new_bytes.iter().position(|byte| *byte == b'\n') {
                    frame.extend_from_slice(&new_bytes[..end]);
                    let verdict = classify(&frame);
                    return match budget.remaining_at(now()) {
                        Ok(_) => Ok(verdict),
                        Err(ProbeStopped::Cancelled) => Err(Cancelled),
                        Err(ProbeStopped::TimedOut) => Ok(spent()),
                    };
                }
                frame.extend_from_slice(new_bytes);
                if frame.len() > MAX_RESPONSE_BYTES {
                    return Ok(Readiness::NotReady(NotReady::Malformed {
                        line: diagnostic(&frame),
                    }));
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if !wait_ready(fd, Interest::Readable, budget, &now, &mut wait)? {
                    return Ok(spent());
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Ok(spent()),
        }
    }
}

fn wait_ready<N, W>(
    fd: RawFd,
    interest: Interest,
    budget: &ProbeBudget<'_>,
    now: &N,
    wait: &mut W,
) -> Result<bool, Cancelled>
where
    N: Fn() -> Instant,
    W: FnMut(RawFd, Interest, Duration) -> std::io::Result<bool>,
{
    loop {
        let left = match budget.remaining_at(now()) {
            Ok(left) => left,
            Err(ProbeStopped::Cancelled) => return Err(Cancelled),
            Err(ProbeStopped::TimedOut) => return Ok(false),
        };
        match wait(fd, interest, left.min(MAX_WAIT)) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Ok(false),
        }
    }
}

fn spent() -> Readiness {
    Readiness::NotReady(NotReady::TimedOut)
}

/// The probe a running membrane asks its brokers with: it opens the broker's
/// control socket, sends `membrane.status`, and relays what came back. Every
/// call asks again, as `brokers::Probe` requires — a kept answer cannot report a
/// broker that has since stopped serving.
pub struct ControlSocketProbe;

impl crate::brokers::Probe for ControlSocketProbe {
    fn probe(&self, socket: &Path, budget: &ProbeBudget<'_>) -> Result<Readiness, Cancelled> {
        probe(socket, budget)
    }
}

fn classify(frame: &[u8]) -> Readiness {
    let malformed = |bytes: &[u8]| {
        Readiness::NotReady(NotReady::Malformed {
            line: diagnostic(bytes),
        })
    };
    let Ok(text) = std::str::from_utf8(frame) else {
        return malformed(frame);
    };
    let trimmed = text.trim();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return malformed(trimmed.as_bytes());
    };
    let result = value.get("result").unwrap_or(&serde_json::Value::Null);
    let state = result
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    match result.get("ready").and_then(serde_json::Value::as_bool) {
        Some(true) => Readiness::Ready { state },
        Some(false) => Readiness::NotReady(NotReady::Reported { state }),
        None => malformed(trimmed.as_bytes()),
    }
}

fn diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(DIAGNOSTIC_BYTES)]).into_owned()
}

enum Connected<S> {
    Yes(S),
    Unreachable,
    Stopped(ProbeStopped),
}

fn connect_budgeted(socket: &Path, budget: &ProbeBudget<'_>) -> Connected<UnixStream> {
    let (address, address_length) = match address_for(socket) {
        Ok(pair) => pair,
        Err(_) => return Connected::Unreachable,
    };
    let fd = match open_nonblocking() {
        Ok(fd) => fd,
        Err(_) => return Connected::Unreachable,
    };
    match unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            address_length,
        )
    } {
        0 => return Connected::Yes(UnixStream::from(fd)),
        _ => {
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EINPROGRESS | libc::EALREADY) => {}
                Some(libc::EINTR) => return Connected::Stopped(halted(budget)),
                Some(libc::EAGAIN) => return Connected::Stopped(halted(budget)),
                _ => return Connected::Unreachable,
            }
        }
    }
    loop {
        let left = match budget.remaining() {
            Ok(left) => left,
            Err(ProbeStopped::Cancelled) => return Connected::Stopped(ProbeStopped::Cancelled),
            Err(ProbeStopped::TimedOut) => return Connected::Stopped(ProbeStopped::TimedOut),
        };
        match wait_fd(fd.as_raw_fd(), Interest::Writable, left.min(MAX_WAIT)) {
            Ok(true) => match socket_error(fd.as_raw_fd()) {
                Ok(0) => return Connected::Yes(UnixStream::from(fd)),
                Ok(libc::EINPROGRESS | libc::EALREADY) => {}
                Ok(_) => return Connected::Unreachable,
                Err(_) => return Connected::Unreachable,
            },
            Ok(false) => {}
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Connected::Stopped(ProbeStopped::TimedOut),
        }
    }
}

fn halted(budget: &ProbeBudget<'_>) -> ProbeStopped {
    match budget.remaining() {
        Err(stop) => stop,
        Ok(_) => ProbeStopped::TimedOut,
    }
}

fn socket_error(fd: RawFd) -> Result<libc::c_int, ()> {
    let mut code: libc::c_int = 0;
    let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let outcome = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            (&mut code as *mut libc::c_int).cast(),
            &mut length,
        )
    };
    if outcome == 0 { Ok(code) } else { Err(()) }
}

fn open_nonblocking() -> std::io::Result<OwnedFd> {
    #[cfg(target_os = "linux")]
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    #[cfg(not(target_os = "linux"))]
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    let fd = match raw {
        -1 => return Err(std::io::Error::last_os_error()),
        raw => unsafe { OwnedFd::from_raw_fd(raw) },
    };
    #[cfg(not(target_os = "linux"))]
    {
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
        if flags < 0
            || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(fd)
}

fn address_for(socket: &Path) -> std::io::Result<(libc::sockaddr_un, libc::socklen_t)> {
    let bytes = socket.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as _;
    let path_at = std::mem::offset_of!(libc::sockaddr_un, sun_path);
    if bytes.is_empty() || bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the socket path does not fit a pathname address",
        ));
    }
    for (index, byte) in bytes.iter().enumerate() {
        address.sun_path[index] = *byte as libc::c_char;
    }
    Ok((address, (path_at + bytes.len() + 1) as libc::socklen_t))
}

#[derive(Debug, Clone, Copy)]
enum Interest {
    Readable,
    Writable,
}

impl Interest {
    fn events(self) -> libc::c_short {
        match self {
            Interest::Readable => libc::POLLIN,
            Interest::Writable => libc::POLLOUT,
        }
    }
}

fn wait_fd(fd: RawFd, interest: Interest, slice: Duration) -> std::io::Result<bool> {
    let mut polling = [libc::pollfd {
        fd,
        events: interest.events(),
        revents: 0,
    }];
    let timeout = slice.as_nanos().div_ceil(1_000_000).min(i32::MAX as u128) as i32;
    let counted = unsafe { libc::poll(polling.as_mut_ptr(), 1, timeout.max(1)) };
    if counted < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if counted == 0 {
        return Ok(false);
    }
    if polling[0].revents & libc::POLLNVAL != 0 {
        return Err(std::io::Error::other(
            "poll reported the descriptor invalid",
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::thread;

    const DEADLINE: Duration = Duration::from_millis(500);
    const READY_LINE: &[u8] = b"{\"id\":0,\"result\":{\"ready\":true,\"state\":\"serving\"}}\n";

    enum Answer {
        Ready(&'static str),
        NotReady(&'static str),
        Silent,
        Nonsense(&'static str),
    }

    fn serve(socket: PathBuf, answer: Answer) -> mpsc::Receiver<String> {
        let listener =
            UnixListener::bind(&socket).expect("the test broker binds its control socket");
        let (requests, received) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("the test broker accepts its one probe");
            let mut reader = BufReader::new(stream.try_clone().expect("clone for reading"));
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let _ = requests.send(line);
            let reply = match answer {
                Answer::Ready(state) => {
                    format!("{{\"id\":0,\"result\":{{\"ready\":true,\"state\":\"{state}\"}}}}\n")
                }
                Answer::NotReady(state) => {
                    format!("{{\"id\":0,\"result\":{{\"ready\":false,\"state\":\"{state}\"}}}}\n")
                }
                Answer::Nonsense(text) => format!("{text}\n"),
                Answer::Silent => return,
            };
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        });
        received
    }

    fn a_budget(flag: &AtomicBool) -> ProbeBudget<'_> {
        ProbeBudget::new(DEADLINE, flag)
    }

    #[test]
    fn a_control_socket_that_answers_status_is_ready() {
        let flag = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        serve(socket.clone(), Answer::Ready("serving"));
        let verdict = probe(&socket, &a_budget(&flag));
        assert_eq!(
            verdict,
            Ok(Readiness::Ready {
                state: "serving".to_string()
            })
        );
        assert!(verdict.as_ref().is_ok_and(Readiness::is_ready));
    }

    #[test]
    fn a_socket_that_accepts_but_never_answers_is_the_half_alive_broker() {
        let flag = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        serve(socket.clone(), Answer::Silent);
        assert_eq!(probe(&socket, &a_budget(&flag)), Ok(spent()));
    }

    #[test]
    fn a_missing_control_socket_is_unreachable_not_ready() {
        let flag = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let verdict = probe(&dir.path().join("absent.control"), &a_budget(&flag));
        assert!(!verdict.as_ref().is_ok_and(Readiness::is_ready));
        match verdict {
            Ok(Readiness::NotReady(NotReady::Unreachable { path })) => {
                assert!(path.ends_with("absent.control"));
            }
            other => panic!("a missing socket is unreachable, got {other:?}"),
        }
    }

    #[test]
    fn a_path_too_long_for_a_socket_address_is_unreachable() {
        let flag = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let long = dir.path().join("x".repeat(200));
        let verdict = probe(&long, &a_budget(&flag));
        assert!(matches!(
            verdict,
            Ok(Readiness::NotReady(NotReady::Unreachable { .. }))
        ));
    }

    #[test]
    fn an_answer_without_a_status_payload_is_malformed() {
        let flag = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        serve(socket.clone(), Answer::Nonsense("{\"id\":0,\"result\":42}"));
        let verdict = probe(&socket, &a_budget(&flag));
        assert!(!verdict.as_ref().is_ok_and(Readiness::is_ready));
        assert!(matches!(
            verdict,
            Ok(Readiness::NotReady(NotReady::Malformed { .. }))
        ));
    }

    #[test]
    fn a_server_reporting_not_ready_is_not_ready_even_though_it_answers() {
        let flag = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        serve(socket.clone(), Answer::NotReady("starting"));
        assert_eq!(
            probe(&socket, &a_budget(&flag)),
            Ok(Readiness::NotReady(NotReady::Reported {
                state: "starting".to_string()
            }))
        );
    }

    #[test]
    fn the_production_probe_asks_the_broker_socket_and_relays_its_answer() {
        use crate::brokers::Probe;

        let flag = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let serving = dir.path().join("s.uds");
        serve(serving.clone(), Answer::Ready("serving"));
        assert_eq!(
            ControlSocketProbe.probe(&serving, &a_budget(&flag)),
            Ok(Readiness::Ready {
                state: "serving".to_string()
            })
        );

        let silent = dir.path().join("q.uds");
        serve(silent.clone(), Answer::Silent);
        assert_eq!(
            ControlSocketProbe.probe(&silent, &a_budget(&flag)),
            Ok(spent())
        );
    }

    #[test]
    fn the_probe_sends_one_membrane_status_request_per_probe() {
        let flag = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        let requests = serve(socket.clone(), Answer::Ready("serving"));
        assert_eq!(
            probe(&socket, &a_budget(&flag)),
            Ok(Readiness::Ready {
                state: "serving".to_string()
            })
        );
        let line = requests
            .recv_timeout(DEADLINE)
            .expect("the test broker captured the request the probe sent");
        let request: serde_json::Value =
            serde_json::from_str(line.trim()).unwrap_or_else(|error| {
                panic!("the probe sends one JSON request per line, got {line:?}: {error}")
            });
        assert_eq!(
            request.get("method").and_then(serde_json::Value::as_str),
            Some("membrane.status"),
            "the probe asks for membrane.status, got {request}"
        );
        assert!(
            request
                .get("params")
                .is_some_and(serde_json::Value::is_object),
            "the probe's request carries an object params, got {request}"
        );
    }

    struct Scripted {
        events: Rc<RefCell<VecDeque<Event>>>,
        written: Rc<RefCell<Vec<u8>>>,
        closed: Rc<Cell<bool>>,
    }

    #[derive(Clone, PartialEq)]
    enum Event {
        Deliver(Vec<u8>),
        Stalled,
        Interrupted,
        Gone,
    }

    impl Scripted {
        fn new(events: VecDeque<Event>) -> Scripted {
            Scripted {
                events: Rc::new(RefCell::new(events)),
                written: Rc::new(RefCell::new(Vec::new())),
                closed: Rc::new(Cell::new(false)),
            }
        }

        fn handles(&self) -> Scripted {
            Scripted {
                events: Rc::clone(&self.events),
                written: Rc::clone(&self.written),
                closed: Rc::clone(&self.closed),
            }
        }

        fn unwritten_events(&self) -> VecDeque<Event> {
            self.events.borrow().clone()
        }

        fn written(&self) -> Vec<u8> {
            self.written.borrow().clone()
        }
    }

    impl Drop for Scripted {
        fn drop(&mut self) {
            self.closed.set(true);
        }
    }

    impl Read for Scripted {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let event = self.events.borrow_mut().pop_front();
            match event {
                None | Some(Event::Gone) => Ok(0),
                Some(Event::Deliver(bytes)) => {
                    let taken = bytes.len().min(buf.len());
                    buf[..taken].copy_from_slice(&bytes[..taken]);
                    if taken < bytes.len() {
                        self.events
                            .borrow_mut()
                            .push_front(Event::Deliver(bytes[taken..].to_vec()));
                    }
                    Ok(taken)
                }
                Some(Event::Stalled) => Err(std::io::Error::from(ErrorKind::WouldBlock)),
                Some(Event::Interrupted) => Err(std::io::Error::from(ErrorKind::Interrupted)),
            }
        }
    }

    impl Write for Scripted {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl AsRawFd for Scripted {
        fn as_raw_fd(&self) -> RawFd {
            -1
        }
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

        fn now(&self) -> impl Fn() -> Instant + '_ {
            let now_ms = Rc::clone(&self.now_ms);
            let base = self.base;
            move || Self::at(base, now_ms.get())
        }

        fn at(base: Instant, ms: u64) -> Instant {
            base.checked_add(Duration::from_millis(ms))
                .expect("the test clock stays in range")
        }

        fn set(&self, ms: u64) {
            self.now_ms.set(ms);
        }

        fn budget<'a>(&self, allowance: Duration, flag: &'a AtomicBool) -> ProbeBudget<'a> {
            ProbeBudget::new_at(allowance, flag, self.base)
        }
    }

    fn fake_wait<'a>(
        clock: &'a FakeClock,
        flag: Option<(&'a AtomicBool, u64)>,
    ) -> impl FnMut(RawFd, Interest, Duration) -> std::io::Result<bool> + 'a {
        let now_ms = Rc::clone(&clock.now_ms);
        move |_: RawFd, _: Interest, slice: Duration| {
            let next = now_ms.get() + slice.as_millis() as u64;
            now_ms.set(next);
            if let Some((shutdown, at)) = flag
                && next >= at
            {
                shutdown.store(true, Ordering::Relaxed);
            }
            Ok(true)
        }
    }

    fn run_scripted(
        budget: &ProbeBudget<'_>,
        events: VecDeque<Event>,
        now: impl Fn() -> Instant,
        wait: impl FnMut(RawFd, Interest, Duration) -> std::io::Result<bool>,
    ) -> (Result<Readiness, Cancelled>, Scripted) {
        let stream = Scripted::new(events);
        let handles = stream.handles();
        let verdict = probe_with(
            Path::new("/unused.uds"),
            budget,
            |_| Connected::Yes(stream),
            now,
            wait,
        );
        (verdict, handles)
    }

    #[test]
    fn a_trickling_reply_times_out_inside_the_fixed_budget() {
        let clock = FakeClock::new();
        let flag = AtomicBool::new(false);
        let budget = clock.budget(Duration::from_millis(100), &flag);
        let mut events: VecDeque<Event> = (0..15)
            .flat_map(|_| [Event::Stalled, Event::Deliver(b"  ".to_vec())])
            .collect();
        events.push_back(Event::Deliver(READY_LINE.to_vec()));

        let (verdict, stream) = run_scripted(&budget, events, clock.now(), fake_wait(&clock, None));

        assert_eq!(verdict, Ok(spent()), "a trickle cannot extend the budget");
        assert!(
            stream
                .unwritten_events()
                .contains(&Event::Deliver(READY_LINE.to_vec())),
            "the eventual ready frame was never consumed"
        );
        assert_eq!(
            stream.written(),
            STATUS_REQUEST.to_vec(),
            "the request went out before the reply stalled"
        );
    }

    #[test]
    fn fragments_that_arrive_without_waiting_are_still_bounded() {
        let base = Instant::now();
        let tick_ms = Rc::new(Cell::new(0u64));
        let reader = Rc::clone(&tick_ms);
        let now = move || {
            reader.set(reader.get() + 1);
            FakeClock::at(base, reader.get())
        };
        let flag = AtomicBool::new(false);
        let budget = ProbeBudget::new_at(Duration::from_millis(100), &flag, base);
        let events: VecDeque<Event> = (0..200)
            .map(|_| Event::Deliver(b" ".to_vec()))
            .chain([Event::Deliver(READY_LINE.to_vec())])
            .collect();

        let (verdict, _stream) = run_scripted(&budget, events, now, |_, _, _| Ok(true));

        assert_eq!(verdict, Ok(spent()), "flooding cannot skip the checks");
    }

    #[test]
    fn a_reply_of_exactly_the_cap_is_ready_and_one_more_byte_is_malformed() {
        let at_cap = ready_frame_of_len(MAX_RESPONSE_BYTES);
        assert_eq!(at_cap.len(), MAX_RESPONSE_BYTES);
        let flag = AtomicBool::new(false);

        let clock = FakeClock::new();
        let budget = clock.budget(DEADLINE, &flag);
        let events: VecDeque<Event> = [
            Event::Deliver(at_cap[..MAX_RESPONSE_BYTES / 2].to_vec()),
            Event::Deliver(at_cap[MAX_RESPONSE_BYTES / 2..].to_vec()),
            Event::Deliver(b"\nGARBAGE AFTER THE FRAME\n".to_vec()),
        ]
        .into();
        let (verdict, _stream) =
            run_scripted(&budget, events, clock.now(), fake_wait(&clock, None));
        assert_eq!(
            verdict,
            Ok(Readiness::Ready {
                state: "serving".to_string()
            }),
            "an at-cap frame plus newline is valid, trailing bytes ignored"
        );

        let clock = FakeClock::new();
        let budget = clock.budget(DEADLINE, &flag);
        let mut events: VecDeque<Event> = [Event::Deliver(at_cap)].into();
        events.push_back(Event::Deliver(b"x".to_vec()));
        events.push_back(Event::Deliver(b"\n".to_vec()));
        events.push_back(Event::Deliver(READY_LINE.to_vec()));
        let (verdict, stream) = run_scripted(&budget, events, clock.now(), fake_wait(&clock, None));
        assert!(matches!(
            verdict,
            Ok(Readiness::NotReady(NotReady::Malformed { .. }))
        ));
        assert!(
            stream
                .unwritten_events()
                .contains(&Event::Deliver(b"\n".to_vec())),
            "the peer is not drained past the refusal"
        );
    }

    #[test]
    fn an_oversized_frame_is_refused_without_its_terminator() {
        let clock = FakeClock::new();
        let flag = AtomicBool::new(false);
        let budget = clock.budget(DEADLINE, &flag);
        let mut events: VecDeque<Event> =
            [Event::Deliver(vec![b' '; MAX_RESPONSE_BYTES + 1])].into();
        events.push_back(Event::Deliver(b"\n".to_vec()));
        events.push_back(Event::Deliver(READY_LINE.to_vec()));

        let (verdict, stream) = run_scripted(&budget, events, clock.now(), fake_wait(&clock, None));

        assert!(matches!(
            verdict,
            Ok(Readiness::NotReady(NotReady::Malformed { .. }))
        ));
        assert!(
            stream
                .unwritten_events()
                .contains(&Event::Deliver(b"\n".to_vec())),
            "nothing is drained past the cap"
        );
    }

    #[test]
    fn a_malformed_diagnostic_keeps_a_bounded_prefix() {
        let frame = ready_frame_of_len(MAX_RESPONSE_BYTES + 50);
        let clock = FakeClock::new();
        let flag = AtomicBool::new(false);
        let budget = clock.budget(DEADLINE, &flag);
        let events: VecDeque<Event> =
            [Event::Deliver(frame), Event::Deliver(b"\n".to_vec())].into();

        let (verdict, _stream) =
            run_scripted(&budget, events, clock.now(), fake_wait(&clock, None));

        match verdict {
            Ok(Readiness::NotReady(NotReady::Malformed { line })) => {
                assert!(
                    line.len() <= DIAGNOSTIC_BYTES,
                    "the diagnostic is truncated, not the whole oversized frame"
                );
            }
            other => panic!("an oversized frame is malformed, got {other:?}"),
        }
    }

    #[test]
    fn unterminated_and_silent_replies_are_timed_out_not_malformed() {
        let flag = AtomicBool::new(false);

        let clock = FakeClock::new();
        let budget = clock.budget(DEADLINE, &flag);
        let partial: Vec<u8> = READY_LINE[..READY_LINE.len() - 1].to_vec();
        let events: VecDeque<Event> = [Event::Deliver(partial), Event::Gone].into();
        let (verdict, _stream) =
            run_scripted(&budget, events, clock.now(), fake_wait(&clock, None));
        assert_eq!(verdict, Ok(spent()), "EOF before the newline is timed out");

        let clock = FakeClock::new();
        let budget = clock.budget(DEADLINE, &flag);
        let (verdict, _stream) = run_scripted(
            &budget,
            [Event::Gone].into(),
            clock.now(),
            fake_wait(&clock, None),
        );
        assert_eq!(verdict, Ok(spent()), "an empty EOF is timed out");

        let clock = FakeClock::new();
        let budget = clock.budget(Duration::from_millis(80), &flag);
        let partial: Vec<u8> = READY_LINE[..10].to_vec();
        let events: VecDeque<Event> = (0..5)
            .map(|_| Event::Stalled)
            .chain([Event::Deliver(partial)])
            .collect();
        let (verdict, _stream) =
            run_scripted(&budget, events, clock.now(), fake_wait(&clock, None));
        assert_eq!(
            verdict,
            Ok(spent()),
            "silence after a partial frame is timed out"
        );
    }

    #[test]
    fn terminated_garbage_is_malformed_whatever_the_bytes_were() {
        let flag = AtomicBool::new(false);

        for frame in [
            &b"\xff\xfe not utf-8\n"[..],
            &b"not json at all\n"[..],
            &b"{\"id\":0}\n"[..],
        ] {
            let clock = FakeClock::new();
            let budget = clock.budget(DEADLINE, &flag);
            let events: VecDeque<Event> = [Event::Deliver(frame.to_vec())].into();
            let (verdict, _stream) =
                run_scripted(&budget, events, clock.now(), fake_wait(&clock, None));
            assert!(
                matches!(verdict, Ok(Readiness::NotReady(NotReady::Malformed { .. }))),
                "{frame:?} without a status payload is malformed, got {verdict:?}"
            );
        }
    }

    #[test]
    fn a_verdict_that_finishes_after_the_allowance_is_refused() {
        let base = Instant::now();
        let tick_ms = Rc::new(Cell::new(0u64));
        let reader = Rc::clone(&tick_ms);
        let now = move || {
            reader.set(reader.get() + 1);
            FakeClock::at(base, reader.get())
        };
        let flag = AtomicBool::new(false);
        let budget = ProbeBudget::new_at(Duration::from_millis(86), &flag, base);
        let events: VecDeque<Event> = (0..40)
            .map(|_| Event::Deliver(b" ".to_vec()))
            .chain([Event::Deliver(READY_LINE.to_vec())])
            .collect();

        let (verdict, _stream) = run_scripted(&budget, events, now, |_, _, _| Ok(true));

        assert_eq!(
            verdict,
            Ok(spent()),
            "a classifier that finishes after expiry must not bless the frame ready"
        );
    }

    #[test]
    fn a_connect_that_outlasts_the_allowance_is_discarded_without_a_request() {
        let clock = FakeClock::new();
        let flag = AtomicBool::new(false);
        let budget = clock.budget(Duration::from_millis(100), &flag);
        let connect_clock = clock.clone();
        let stream = Scripted::new(VecDeque::new());
        let handles = stream.handles();
        let verdict = probe_with(
            Path::new("/unused.uds"),
            &budget,
            move |path| {
                connect_clock.set(120);
                let _ = path;
                Connected::Yes(stream)
            },
            clock.now(),
            fake_wait(&clock, None),
        );
        assert_eq!(
            verdict,
            Ok(spent()),
            "a late connect grants no fresh budget"
        );
        assert!(
            handles.written().is_empty(),
            "no request follows a connect that spent the allowance"
        );
        assert!(
            handles.closed.get(),
            "the refused stream is dropped, releasing its descriptor"
        );
    }

    #[test]
    fn a_zero_allowance_never_creates_a_socket() {
        let clock = FakeClock::new();
        let flag = AtomicBool::new(false);
        let budget = clock.budget(Duration::ZERO, &flag);
        let stream = Scripted::new(VecDeque::new());
        let handles = stream.handles();
        let verdict = probe_with(
            Path::new("/unused.uds"),
            &budget,
            |_| Connected::Yes(stream),
            clock.now(),
            fake_wait(&clock, None),
        );
        assert_eq!(verdict, Ok(spent()));
        assert!(
            handles.written().is_empty(),
            "a spent budget must not write a socket"
        );
    }

    #[test]
    fn cancellation_before_any_connect_reports_cancelled_not_timed_out() {
        let clock = FakeClock::new();
        let flag = AtomicBool::new(true);
        let budget = clock.budget(DEADLINE, &flag);
        let stream = Scripted::new(VecDeque::new());
        let handles = stream.handles();
        let verdict = probe_with(
            Path::new("/unused.uds"),
            &budget,
            |_| Connected::Yes(stream),
            clock.now(),
            fake_wait(&clock, None),
        );
        assert_eq!(verdict, Err(Cancelled));
        assert!(handles.written().is_empty());
    }

    #[test]
    fn cancellation_during_a_stalled_read_dominates_and_leaves_no_verdict() {
        let clock = FakeClock::new();
        let flag = AtomicBool::new(false);
        let budget = clock.budget(Duration::from_secs(30), &flag);
        let mut events: VecDeque<Event> = [Event::Deliver(b" ".to_vec())].into();
        for _ in 0..8 {
            events.push_back(Event::Stalled);
        }
        let (verdict, stream) = run_scripted(
            &budget,
            events,
            clock.now(),
            fake_wait(&clock, Some((&flag, 40))),
        );

        assert_eq!(verdict, Err(Cancelled));
        assert!(
            stream.closed.get(),
            "the descriptor is dropped on cancellation"
        );
        assert!(
            stream
                .unwritten_events()
                .iter()
                .all(|event| matches!(event, Event::Stalled)),
            "nothing further is read after cancellation"
        );
    }

    #[test]
    fn cancellation_wins_over_expiry_at_the_same_check() {
        let flag = AtomicBool::new(true);
        let budget = ProbeBudget::new(Duration::ZERO, &flag);
        assert_eq!(budget.remaining(), Err(ProbeStopped::Cancelled));
    }

    #[test]
    fn an_extreme_allowance_does_not_overflow_the_clock() {
        let flag = AtomicBool::new(false);
        let budget = ProbeBudget::new(Duration::MAX, &flag);
        assert!(budget.remaining().is_ok());
    }

    #[test]
    fn interrupted_reads_and_stalls_never_reset_the_clock() {
        let clock = FakeClock::new();
        let flag = AtomicBool::new(false);
        let budget = clock.budget(Duration::from_millis(100), &flag);
        let mut events: VecDeque<Event> = [Event::Interrupted].into();
        for _ in 0..5 {
            events.push_back(Event::Stalled);
        }
        events.push_back(Event::Deliver(READY_LINE.to_vec()));

        let (verdict, stream) = run_scripted(&budget, events, clock.now(), fake_wait(&clock, None));

        assert_eq!(verdict, Ok(spent()));
        assert_eq!(stream.written(), STATUS_REQUEST.to_vec());
        assert!(
            stream
                .unwritten_events()
                .contains(&Event::Deliver(READY_LINE.to_vec())),
            "the late frame was never consumed"
        );
    }

    fn ready_frame_of_len(total: usize) -> Vec<u8> {
        let head = br#"{"id":0,"result":{"ready":true,"state":"serving","p":""#;
        let tail = br#""}}"#;
        let pad = total - head.len() - tail.len();
        let mut frame = head.to_vec();
        frame.resize(head.len() + pad, b'x');
        frame.extend_from_slice(tail);
        frame
    }
}
