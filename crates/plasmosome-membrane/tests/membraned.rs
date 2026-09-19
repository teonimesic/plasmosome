use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[path = "support/fixture.rs"]
mod fixture;

const STATUS_REQUEST: &str = r#"{"id":1,"method":"membrane.status","params":{}}"#;
const READY: &str = r#"{"id":0,"result":{"ready":true,"state":"serving"}}"#;
const STARTING: &str = r#"{"id":0,"result":{"ready":false,"state":"starting"}}"#;
const PATIENCE: Duration = Duration::from_secs(10);

fn broker_answering(socket: PathBuf, answer: &'static str) {
    let listener = UnixListener::bind(&socket).expect("the test broker binds its control socket");
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let Ok(reading) = stream.try_clone() else {
                return;
            };
            let mut line = String::new();
            let _ = BufReader::new(reading).read_line(&mut line);
            let _ = writeln!(stream, "{answer}");
            let _ = stream.flush();
        }
    });
}

struct Daemon {
    child: Child,
}

impl Daemon {
    fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    fn signal(&self, signal: libc::c_int) {
        assert_eq!(
            unsafe { libc::kill(self.pid(), signal) },
            0,
            "the daemon is still running and can be signalled"
        );
    }

    fn wait_for_exit(&mut self, budget: Duration) -> ExitStatus {
        let deadline = Instant::now() + budget;
        loop {
            match self
                .child
                .try_wait()
                .expect("the daemon's state is readable")
            {
                Some(status) => return status,
                None => assert!(
                    Instant::now() < deadline,
                    "membraned exits within {budget:?} of the signal"
                ),
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        unsafe { libc::kill(self.pid(), libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn membraned(arguments: &[&Path]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_membraned"));
    command.args(arguments).stderr(Stdio::piped());
    command
}

fn start(config: &Path) -> Daemon {
    Daemon {
        child: membraned(&[config])
            .spawn()
            .expect("membraned starts as a process"),
    }
}

fn write_config(path: &Path, body: &Value) {
    std::fs::write(path, serde_json::to_string(body).unwrap()).expect("the config is written");
}

fn addressable(socket: &Path) -> BufReader<UnixStream> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(stream) = UnixStream::connect(socket) {
            stream
                .set_read_timeout(Some(PATIENCE))
                .expect("the test client bounds its own reads");
            return BufReader::new(stream);
        }
        assert!(
            Instant::now() < deadline,
            "membraned is addressable on {} within five seconds",
            socket.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn ask(client: &mut BufReader<UnixStream>) -> Value {
    let mut stream = client.get_ref().try_clone().expect("clone for writing");
    writeln!(stream, "{STATUS_REQUEST}").expect("the request reaches membraned");
    stream.flush().expect("the request is flushed");
    let mut reply = String::new();
    let read = client.read_line(&mut reply).expect("membraned answers");
    assert_ne!(read, 0, "membraned answered rather than closing the socket");
    serde_json::from_str(&reply)
        .unwrap_or_else(|error| panic!("membraned answers JSON, got {reply:?}: {error}"))
}

fn recorded_pid(pidfile: &Path) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(pidfile)
            && let Ok(pid) = text.trim().parse::<i32>()
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "the broker recorded its pid in {}",
            pidfile.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn is_gone(pid: i32) -> bool {
    let probed = unsafe { libc::kill(pid, 0) };
    probed == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

fn one_broker(dir: &Path, socket: &Path, pidfile: &Path) -> Value {
    json!({
        "control_socket": dir.join("c.uds"),
        "brokers": [{
            "name": "egressd",
            "control_socket": socket,
            "command": ["sh", "-c", format!("echo $$ > {}; exec sleep 300", pidfile.display())],
        }],
    })
}

struct NamedWorker {
    liveness: File,
    control: PathBuf,
    finished: bool,
}

impl NamedWorker {
    fn new(liveness: &Path, control: &Path) -> Self {
        for path in [liveness, control] {
            let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
                .expect("the FIFO path has no zero byte");
            assert_eq!(
                unsafe { libc::mkfifo(name.as_ptr(), 0o600) },
                0,
                "the private worker FIFO is created"
            );
        }
        let liveness = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(liveness)
            .expect("the parent opens its worker liveness FIFO");
        Self {
            liveness,
            control: control.to_owned(),
            finished: false,
        }
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + PATIENCE;
        let mut byte = [0u8; 1];
        loop {
            match self.liveness.read(&mut byte) {
                Ok(1) => return,
                Ok(0) => {}
                Ok(_) => unreachable!("the readiness read is one byte"),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => panic!("worker readiness is readable: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "the broker leader announced its live worker"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_eof(&mut self) {
        let deadline = Instant::now() + PATIENCE;
        let mut byte = [0u8; 1];
        loop {
            match self.liveness.read(&mut byte) {
                Ok(0) => {
                    self.finished = true;
                    return;
                }
                Ok(_) => panic!("the worker writes only its readiness byte"),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => panic!("worker liveness is readable: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "membraned shutdown closes the managed worker's liveness FIFO"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn release(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&self.control)
            {
                Ok(mut control) => {
                    let _ = control.write_all(b"x");
                    return;
                }
                Err(error)
                    if matches!(error.raw_os_error(), Some(libc::ENXIO) | Some(libc::ENOENT)) =>
                {
                    if Instant::now() >= deadline {
                        return;
                    }
                }
                Err(_) => return,
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for NamedWorker {
    fn drop(&mut self) {
        if !self.finished {
            self.release();
        }
    }
}

fn leader_first_broker(dir: &Path, socket: &Path, liveness: &Path, control: &Path) -> Value {
    json!({
        "control_socket": dir.join("c.uds"),
        "brokers": [{
            "name": "egressd",
            "control_socket": socket,
            "command": [
                fixture::supervision_fixture().display().to_string(),
                "named-exit",
                liveness,
                control,
                "unused"
            ],
        }],
    })
}

#[test]
fn membraned_shutdown_cleans_a_worker_whose_broker_leader_exited_first() {
    let dir = tempfile::tempdir().unwrap();
    let control_socket = dir.path().join("c.uds");
    let broker_socket = dir.path().join("b0.uds");
    let liveness = dir.path().join("worker.live");
    let worker_control = dir.path().join("worker.control");
    let config = dir.path().join("config.json");
    let mut worker = NamedWorker::new(&liveness, &worker_control);
    write_config(
        &config,
        &leader_first_broker(dir.path(), &broker_socket, &liveness, &worker_control),
    );

    let mut daemon = start(&config);
    let client = addressable(&control_socket);
    worker.wait_ready();
    drop(client);

    daemon.signal(libc::SIGTERM);
    let status = daemon.wait_for_exit(PATIENCE);
    assert_eq!(status.code(), Some(0), "SIGTERM completes daemon teardown");
    worker.wait_eof();
    assert!(
        !control_socket.exists(),
        "the control socket path is removed after worker cleanup"
    );
}

#[test]
fn membraned_serves_ready_and_dies_cleanly_on_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("c.uds");
    let broker_socket = dir.path().join("b0.uds");
    let pidfile = dir.path().join("b0.pid");
    let config = dir.path().join("config.json");
    broker_answering(broker_socket.clone(), READY);
    write_config(&config, &one_broker(dir.path(), &broker_socket, &pidfile));

    let mut daemon = start(&config);
    let mut client = addressable(&control);
    assert_eq!(
        ask(&mut client).get("result"),
        Some(&json!({"ready": true, "state": "serving"})),
        "membraned relays its broker set's readiness onto the wire"
    );
    let broker = recorded_pid(&pidfile);
    drop(client);

    daemon.signal(libc::SIGTERM);
    let status = daemon.wait_for_exit(PATIENCE);
    assert_eq!(
        status.code(),
        Some(0),
        "a signalled shutdown is a clean one"
    );
    assert!(
        !control.exists(),
        "the control socket path is removed on shutdown"
    );
    assert!(
        is_gone(broker),
        "the broker died with the membrane that owned it"
    );
}

#[test]
fn membraned_reports_the_broker_that_is_not_serving() {
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("c.uds");
    let broker_socket = dir.path().join("b0.uds");
    let pidfile = dir.path().join("b0.pid");
    let config = dir.path().join("config.json");
    broker_answering(broker_socket.clone(), STARTING);
    write_config(&config, &one_broker(dir.path(), &broker_socket, &pidfile));

    let _daemon = start(&config);
    let mut client = addressable(&control);
    assert_eq!(
        ask(&mut client).get("result"),
        Some(
            &json!({"ready": false, "state": "not_serving", "broker": "egressd",
                     "reason": "reported", "broker_state": "starting"})
        ),
        "a broker that answers but is not serving holds the set back by name"
    );
}

#[test]
fn membraned_exits_nonzero_naming_the_failure() {
    let dir = tempfile::tempdir().unwrap();

    let no_arguments = membraned(&[]).output().expect("membraned runs");
    assert_eq!(
        no_arguments.status.code(),
        Some(2),
        "membraned needs a config"
    );
    assert!(
        String::from_utf8_lossy(&no_arguments.stderr).contains("usage"),
        "membraned says how it is called"
    );

    let absent = dir.path().join("absent.json");
    let unreadable = membraned(&[&absent]).output().expect("membraned runs");
    assert_eq!(
        unreadable.status.code(),
        Some(2),
        "an unreadable config is refused"
    );
    assert!(
        String::from_utf8_lossy(&unreadable.stderr).contains("absent.json"),
        "the refusal names the config it could not read"
    );

    let malformed = dir.path().join("malformed.json");
    std::fs::write(&malformed, b"{not json").unwrap();
    let invalid = membraned(&[&malformed]).output().expect("membraned runs");
    assert_eq!(
        invalid.status.code(),
        Some(2),
        "a config that is not JSON is refused"
    );

    let control = dir.path().join("taken.uds");
    std::fs::write(&control, b"someone else's socket").unwrap();
    let config = dir.path().join("occupied.json");
    write_config(&config, &json!({"control_socket": control}));
    let occupied = membraned(&[&config]).output().expect("membraned runs");
    assert_eq!(
        occupied.status.code(),
        Some(1),
        "a control socket path already in use refuses the start"
    );
    assert!(
        String::from_utf8_lossy(&occupied.stderr).contains("taken.uds"),
        "the refusal names the path it could not bind, got: {}",
        String::from_utf8_lossy(&occupied.stderr)
    );

    let live = dir.path().join("live.uds");
    let holder = UnixListener::bind(&live).expect("the test binds a live control socket");
    let held = dir.path().join("live.json");
    write_config(&held, &json!({"control_socket": live}));
    let refused = membraned(&[&held]).output().expect("membraned runs");
    assert_eq!(
        refused.status.code(),
        Some(1),
        "a live daemon's listening socket refuses the start"
    );
    assert!(
        UnixStream::connect(&live).is_ok(),
        "the listening socket that refused the start is still addressable"
    );
    drop(holder);

    let link = dir.path().join("link.uds");
    std::os::unix::fs::symlink("nowhere.uds", &link)
        .expect("the test plants a dangling leaf symlink");
    let linked = dir.path().join("link.json");
    write_config(&linked, &json!({"control_socket": link}));
    let refused = membraned(&[&linked]).output().expect("membraned runs");
    assert_eq!(
        refused.status.code(),
        Some(1),
        "a control path holding a symlink, not a socket, refuses the start"
    );
    let planted = std::fs::symlink_metadata(&link).expect("the symlink survives the refusal");
    assert!(
        planted.file_type().is_symlink()
            && std::fs::read_link(&link).expect("the link target is readable")
                == Path::new("nowhere.uds"),
        "the refusal preserves the planted symlink and its exact target"
    );
}

/// A broker that answers every probe connection the way a stalled real broker
/// does: it reads the request, signals the rendezvous once it has one, then
/// trickles `spaces` blanks one `drip` apart before its terminator — or closes
/// without one. Each connection gets the same script, so retries see a
/// consistent peer, and every connection is finished when the daemon lets go
/// of its end because the writes then fail.
fn trickling_broker(
    socket: PathBuf,
    spaces: usize,
    drip: Duration,
    terminator: Option<&'static str>,
) -> mpsc::Receiver<()> {
    let listener = UnixListener::bind(&socket).expect("the test broker binds its control socket");
    let (rendezvous, received) = mpsc::channel();
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let Ok(reading) = stream.try_clone() else {
                return;
            };
            let mut line = String::new();
            let _ = BufReader::new(reading).read_line(&mut line);
            if line.is_empty() {
                return;
            }
            let _ = rendezvous.send(());
            for _ in 0..spaces {
                if !drip.is_zero() {
                    thread::sleep(drip);
                }
                if stream.write_all(b" ").is_err() {
                    return;
                }
            }
            if let Some(ready_line) = terminator
                && (stream.write_all(ready_line.as_bytes()).is_err()
                    || stream.write_all(b"\n").is_err())
            {
                return;
            }
            let _ = stream.flush();
        }
    });
    received
}

#[test]
fn membraned_times_out_a_trickling_broker_instead_of_accepting_a_late_ready() {
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("c.uds");
    let broker_socket = dir.path().join("b0.uds");
    let pidfile = dir.path().join("b0.pid");
    let config = dir.path().join("config.json");
    trickling_broker(
        broker_socket.clone(),
        30,
        Duration::from_millis(20),
        Some(READY),
    );
    write_config(
        &config,
        &json!({
            "control_socket": control,
            "status_deadline_ms": 100u64,
            "brokers": [{
                "name": "egressd",
                "control_socket": broker_socket,
                "command": ["sh", "-c", format!("echo $$ > {}; exec sleep 300", pidfile.display())],
            }],
        }),
    );

    let _daemon = start(&config);
    let mut client = addressable(&control);
    let started = Instant::now();
    let answer = ask(&mut client);
    let elapsed = started.elapsed();
    eprintln!("trickle probe answer after {elapsed:?}: {answer}");
    assert_eq!(
        answer.get("result"),
        Some(
            &json!({"ready": false, "state": "not_serving", "broker": "egressd",
                      "reason": "timed_out"})
        ),
        "a broker trickling past the deadline is timed out, never the late ready it would have given"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "the query ended near its 100ms budget, not after the trickle: {elapsed:?}"
    );
}

#[test]
fn membraned_refuses_an_oversized_broker_reply_as_malformed() {
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("c.uds");
    let broker_socket = dir.path().join("b0.uds");
    let pidfile = dir.path().join("b0.pid");
    let config = dir.path().join("config.json");
    trickling_broker(
        broker_socket.clone(),
        1_048_577,
        Duration::ZERO,
        Some(READY),
    );
    write_config(
        &config,
        &json!({
            "control_socket": control,
            "status_deadline_ms": 5_000u64,
            "brokers": [{
                "name": "egressd",
                "control_socket": broker_socket,
                "command": ["sh", "-c", format!("echo $$ > {}; exec sleep 300", pidfile.display())],
            }],
        }),
    );

    let _daemon = start(&config);
    let mut client = addressable(&control);
    let started = Instant::now();
    let answer = ask(&mut client);
    let elapsed = started.elapsed();
    eprintln!("oversize probe answer after {elapsed:?}: {answer}");
    assert_eq!(
        answer.get("result"),
        Some(
            &json!({"ready": false, "state": "not_serving", "broker": "egressd",
                      "reason": "malformed"})
        ),
        "a reply with more than the cap of bytes before its newline is refused as malformed, \
         whatever terminated it later"
    );
}

#[test]
fn membraned_shuts_down_during_an_active_trickling_probe() {
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("c.uds");
    let broker_socket = dir.path().join("b0.uds");
    let pidfile = dir.path().join("b0.pid");
    let config = dir.path().join("config.json");
    // A long trickle: the allowance (10s) sits well above it, so only real
    // cancellation — not the trickle finishing — can end this probe promptly.
    let trickled = trickling_broker(
        broker_socket.clone(),
        400,
        Duration::from_millis(20),
        Some(READY),
    );
    write_config(
        &config,
        &json!({
            "control_socket": control,
            "status_deadline_ms": 10_000u64,
            "brokers": [{
                "name": "egressd",
                "control_socket": broker_socket,
                "command": ["sh", "-c", format!("echo $$ > {}; exec sleep 300", pidfile.display())],
            }],
        }),
    );

    let mut daemon = start(&config);
    let mut client = addressable(&control);
    let mut writer = client.get_ref().try_clone().expect("clone for writing");
    writeln!(writer, "{STATUS_REQUEST}").expect("the status request reaches membraned");
    writer.flush().expect("the request is flushed");
    let broker = recorded_pid(&pidfile);
    trickled
        .recv_timeout(PATIENCE)
        .expect("the broker saw the probe's request, so a probe is in flight");

    let started = Instant::now();
    daemon.signal(libc::SIGTERM);
    // The watchdog bounds signal-to-EOF, so a daemon that waits out the
    // trickle instead of cancelling the probe fails here rather than late.
    client
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("the test bounds its own read");
    let mut reply = String::new();
    let read = client
        .read_line(&mut reply)
        .expect("control EOF arrives within the two-second watchdog");
    assert_eq!(
        read, 0,
        "the conversation closes with no fabricated answer, got {reply:?}"
    );
    let status = daemon.wait_for_exit(Duration::from_secs(2));
    let elapsed = started.elapsed();
    eprintln!("shutdown during probe: exit={status:?} after {elapsed:?}");
    assert_eq!(
        status.code(),
        Some(0),
        "the daemon exits cleanly without waiting out the 10s allowance"
    );
    assert!(
        !control.exists(),
        "teardown began: the control socket path was removed"
    );
    assert!(
        is_gone(broker),
        "the broker was reaped by the teardown the probe no longer delays"
    );
}

#[test]
fn membraned_leaves_a_replacement_settled_at_its_socket_path_alone() {
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("c.uds");
    let config = dir.path().join("config.json");
    write_config(&config, &json!({"control_socket": control}));

    let mut daemon = start(&config);
    let mut client = addressable(&control);
    assert_eq!(
        ask(&mut client).pointer("/result/state"),
        Some(&json!("empty")),
        "the daemon answers before its pathname is exchanged"
    );
    drop(client);

    let original = std::fs::symlink_metadata(&control).expect("the bound socket is on disk");
    let saved = dir.path().join("saved.uds");
    std::fs::rename(&control, &saved).expect("the original socket is renamed");

    let sentinel = b"a caller's regular file, settled after the first status answer";
    std::fs::write(&control, sentinel).expect("the sentinel is written");

    daemon.signal(libc::SIGTERM);
    let status = daemon.wait_for_exit(PATIENCE);
    assert_eq!(
        status.code(),
        Some(0),
        "a signalled shutdown is a clean one"
    );

    let left = std::fs::symlink_metadata(&control).expect("the replacement survives shutdown");
    assert!(
        left.file_type().is_file(),
        "shutdown removes the socket the daemon bound, not an entry a caller settled at the \
         pathname afterwards"
    );
    assert_eq!(
        std::fs::read(&control)
            .expect("the sentinel is readable")
            .as_slice(),
        sentinel.as_slice(),
        "the replacement's contents are preserved exactly"
    );
    let renamed = std::fs::symlink_metadata(&saved).expect("the renamed original survives");
    assert!(
        renamed.file_type().is_socket()
            && (renamed.dev(), renamed.ino()) == (original.dev(), original.ino()),
        "the original socket, renamed away by the caller, survives as the very socket the \
         daemon bound"
    );
}

#[test]
fn membraned_leaves_a_symlink_settled_at_its_socket_path_alone() {
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("c.uds");
    let config = dir.path().join("config.json");
    write_config(&config, &json!({"control_socket": control}));

    let mut daemon = start(&config);
    let mut client = addressable(&control);
    assert_eq!(
        ask(&mut client).pointer("/result/state"),
        Some(&json!("empty")),
        "the daemon answers before its pathname is exchanged"
    );
    drop(client);

    let saved = dir.path().join("saved.uds");
    std::fs::rename(&control, &saved).expect("the original socket is renamed");
    std::os::unix::fs::symlink("saved.uds", &control).expect("the test settles a leaf symlink");

    daemon.signal(libc::SIGTERM);
    let status = daemon.wait_for_exit(PATIENCE);
    assert_eq!(
        status.code(),
        Some(0),
        "a signalled shutdown is a clean one"
    );

    let left = std::fs::symlink_metadata(&control).expect("the symlink survives shutdown");
    assert!(
        left.file_type().is_symlink(),
        "the settled symlink is preserved: the teardown check never follows the leaf entry"
    );
    assert_eq!(
        std::fs::read_link(&control).expect("the link target is readable"),
        PathBuf::from("saved.uds"),
        "the exact link target is preserved"
    );
    assert!(
        std::fs::symlink_metadata(&saved)
            .expect("the renamed original survives")
            .file_type()
            .is_socket(),
        "the symlink's target — the original socket — is not removed through the link"
    );
}

#[test]
fn membraned_leaves_a_dangling_symlink_settled_at_its_socket_path_alone() {
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("c.uds");
    let config = dir.path().join("config.json");
    write_config(&config, &json!({"control_socket": control}));

    let mut daemon = start(&config);
    let mut client = addressable(&control);
    assert_eq!(
        ask(&mut client).pointer("/result/state"),
        Some(&json!("empty")),
        "the daemon answers before its pathname is exchanged"
    );
    drop(client);

    let saved = dir.path().join("saved.uds");
    std::fs::rename(&control, &saved).expect("the original socket is renamed");
    std::os::unix::fs::symlink("nowhere.uds", &control)
        .expect("the test settles a dangling leaf symlink");

    daemon.signal(libc::SIGTERM);
    let status = daemon.wait_for_exit(PATIENCE);
    assert_eq!(
        status.code(),
        Some(0),
        "a signalled shutdown is a clean one"
    );

    let left = std::fs::symlink_metadata(&control).expect("the symlink survives shutdown");
    assert!(
        left.file_type().is_symlink()
            && std::fs::read_link(&control).expect("the link target is readable")
                == Path::new("nowhere.uds"),
        "a dangling leaf symlink is preserved exactly: a missing target is not a missing entry"
    );
    assert!(
        !dir.path().join("nowhere.uds").exists(),
        "the dangling target stays absent"
    );
    assert!(
        std::fs::symlink_metadata(&saved)
            .expect("the renamed original survives")
            .file_type()
            .is_socket(),
        "the original socket stays allocated under the caller's new name"
    );
}
