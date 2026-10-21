mod support;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use plasmosome_cli::json::Json;
use plasmosome_cli::{
    COMMANDS, ClientCode, Clock, Context, Deadline, Exit, SystemClock, Unreplied, exchange, handle,
};
use plasmosome_core::private_socket::effective_uid;
use serde_json::{Value, json};
use support::{Answer, FakeDaemon, PATIENCE, private_root};

#[cfg(target_os = "macos")]
const MAX_PATH: usize = 103;
#[cfg(not(target_os = "macos"))]
const MAX_PATH: usize = 107;

const EPIPE: i32 = 32;

const STATUS: &str = "{\"id\":1,\"method\":\"plasmosome.status\",\"params\":{}}\n";

struct Ran {
    exit: Exit,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    took: Duration,
}

fn line(socket: &Path, rest: &[&str]) -> Vec<OsString> {
    let mut line: Vec<OsString> = vec!["status".into(), "--socket".into(), socket.into()];
    line.extend(rest.iter().map(OsString::from));
    line
}

fn run_on(line: Vec<OsString>, clock: &'static dyn Clock) -> Ran {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let context = Context {
            version: "0.0.0",
            commands: COMMANDS,
            trusted_uid: effective_uid(),
            clock,
        };
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let started = Instant::now();
        let exit = handle(&line, &context, &mut stdout, &mut stderr);
        let _ = sender.send(Ran {
            exit,
            stdout,
            stderr,
            took: started.elapsed(),
        });
    });
    receiver
        .recv_timeout(PATIENCE)
        .unwrap_or_else(|_| panic!("plasmosome status did not return within {PATIENCE:?}"))
}

fn run(line: Vec<OsString>) -> Ran {
    run_on(line, &SystemClock)
}

fn one_line(bytes: &[u8]) -> Value {
    let text = std::str::from_utf8(bytes).expect("the output is UTF-8");
    assert!(
        text.ends_with('\n') && text.matches('\n').count() == 1,
        "the output is not one line: {text:?}"
    );
    serde_json::from_str(text).expect("the output is one JSON line")
}

fn failure(ran: &Ran) -> Value {
    assert!(ran.stdout.is_empty(), "a failure wrote to stdout");
    one_line(&ran.stderr)
}

fn reply(value: Value) -> Answer {
    Answer::Reply(format!("{value}\n").into_bytes())
}

fn socket_in(root: &Path) -> PathBuf {
    root.join("control.uds")
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

#[test]
fn the_result_and_a_kernel_error_pass_through() {
    let root = private_root();
    let socket = socket_in(root.path());
    let result = "{\"id\":1,\"result\":{\"name\":\"work\",\"extra\":{\"nested\":[1,2]},\"big\":18446744073709551616}}\n";
    let error =
        json!({"code":101,"message":"m","target":"plasmosome x","fix":"f","field":"network.hosts"});
    let fake = FakeDaemon::start(
        &socket,
        vec![
            Answer::Reply(result.as_bytes().to_vec()),
            reply(json!({"id":1,"error":error})),
            reply(json!({"id":1,"result":{"name":"x"}})),
        ],
    );

    let ran = run(line(&socket, &[]));
    assert_eq!(ran.exit, Exit::Done);
    assert!(ran.stderr.is_empty());
    let stdout = String::from_utf8(ran.stdout.clone()).expect("stdout is UTF-8");
    assert!(
        stdout.contains(
            "\"result\":{\"name\":\"work\",\"extra\":{\"nested\":[1,2]},\"big\":18446744073709551616}"
        ),
        "{stdout}"
    );
    let envelope = one_line(&ran.stdout);
    assert_eq!(envelope["command"], "plasmosome.status");
    assert_eq!(envelope["instance"], json!({"socket": text(&socket)}));

    let ran = run(line(&socket, &[]));
    assert_eq!(ran.exit, Exit::Refused);
    let envelope = failure(&ran);
    let mut relayed = error.clone();
    relayed["domain"] = json!("kernel");
    assert_eq!(envelope["error"], relayed);
    assert_eq!(envelope["instance"], json!({"socket": text(&socket)}));

    let ran = run(line(&socket, &["--kernel", "x"]));
    assert_eq!(ran.exit, Exit::Done);
    let instance = &one_line(&ran.stdout)["instance"];
    assert_eq!(*instance, json!({"name": "x", "socket": text(&socket)}));

    let named = "{\"id\":1,\"method\":\"plasmosome.status\",\"params\":{\"name\":\"x\"}}\n";
    assert_eq!(
        fake.ended(3),
        [STATUS.as_bytes(), STATUS.as_bytes(), named.as_bytes()]
    );
    assert_eq!(fake.connections(), 3);
}

#[test]
fn a_bad_reply_exits_70() {
    let root = private_root();
    let socket = socket_in(root.path());
    let replies = [
        "not json",
        "[]",
        "{\"id\":1,\"result\":{},\"error\":{\"code\":1,\"message\":\"m\"}}",
        "{\"id\":1}",
        "{\"id\":2,\"result\":{}}",
        "{\"id\":1,\"result\":[]}",
        "{\"id\":1,\"error\":\"e\"}",
        "{\"id\":1,\"error\":{\"code\":\"101\",\"message\":\"m\"}}",
        "{\"id\":1,\"error\":{\"code\":101}}",
        "{\"id\":1,\"error\":{\"code\":101,\"message\":\"m\",\"domain\":\"x\"}}",
        "{\"id\":1,\"result\":{},\"result\":{}}",
    ];
    let answers = replies
        .iter()
        .map(|reply| Answer::Reply(format!("{reply}\n").into_bytes()))
        .collect();
    let _fake = FakeDaemon::start(&socket, answers);
    for reply in replies {
        let ran = run(line(&socket, &[]));
        assert_eq!(ran.exit, Exit::Internal, "{reply}");
        let envelope = failure(&ran);
        assert_eq!(envelope["error"]["code"], "bad_reply", "{reply}");
        assert!(envelope["error"]["detail"].is_string(), "{reply}");
        assert_eq!(envelope["instance"]["socket"], text(&socket));
    }
}

#[test]
fn an_endless_line_is_refused_early() {
    let root = private_root();
    let socket = socket_in(root.path());
    let fake = FakeDaemon::start(&socket, vec![Answer::Endless]);
    let ran = run(line(&socket, &["--deadline-ms", "60000"]));
    assert_eq!(ran.exit, Exit::Refused);
    let error = &failure(&ran)["error"];
    assert_eq!(error["code"], "reply_too_large");
    assert_eq!(error["max_bytes"], 16_777_216);
    assert!(ran.took < Duration::from_secs(10), "took {:?}", ran.took);
    assert_eq!(fake.ended(1), [STATUS.as_bytes()]);
}

#[test]
fn closed_and_silent_daemons() {
    let root = private_root();
    let socket = socket_in(root.path());
    let fake = FakeDaemon::start(&socket, vec![Answer::Close, Answer::Silent]);

    let closed = run(line(&socket, &[]));
    assert_eq!(closed.exit, Exit::Unavailable);
    let error = &failure(&closed)["error"];
    assert_eq!(
        (&error["code"], &error["cause"]),
        (&json!("no_reply"), &json!("closed"))
    );

    let silent = run(line(&socket, &["--deadline-ms", "300"]));
    assert_eq!(silent.exit, Exit::Unavailable);
    let error = &failure(&silent)["error"];
    assert_eq!(
        (&error["code"], &error["cause"]),
        (&json!("no_reply"), &json!("deadline"))
    );
    assert!(
        silent.took >= Duration::from_millis(300) && silent.took < Duration::from_secs(2),
        "took {:?}",
        silent.took
    );

    assert_eq!(fake.ended(2), [STATUS.as_bytes(), STATUS.as_bytes()]);
    assert_eq!(fake.connections(), 2);
}

struct ScriptedClock<F> {
    start: Instant,
    offsets_ms: &'static [u64],
    reads: AtomicUsize,
    at_reading: F,
}

impl<F: Fn(usize) + Sync> Clock for ScriptedClock<F> {
    fn now(&self) -> Instant {
        let reading = self.reads.fetch_add(1, Ordering::Relaxed);
        (self.at_reading)(reading);
        let offset = self.offsets_ms[reading.min(self.offsets_ms.len() - 1)];
        self.start + Duration::from_millis(offset)
    }
}

const FIRST_WRITE: usize = 1;

const SECOND_CALL: usize = 2;

fn clock_with(
    offsets_ms: &'static [u64],
    at_reading: impl Fn(usize) + Sync + 'static,
) -> &'static dyn Clock {
    Box::leak(Box::new(ScriptedClock {
        start: Instant::now(),
        offsets_ms,
        reads: AtomicUsize::new(0),
        at_reading,
    }))
}

fn clock_reading(offsets_ms: &'static [u64]) -> &'static dyn Clock {
    clock_with(offsets_ms, |_| {})
}

fn peer_at(socket: &Path, act: impl FnOnce(UnixStream) + Send + 'static) {
    let listener = UnixListener::bind(socket).expect("a socket");
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
        .expect("the socket is 0600");
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("a connection");
        act(stream);
    });
}

fn read_request(stream: &mut UnixStream) {
    let mut seen = Vec::new();
    let mut buffer = [0_u8; 4096];
    while !seen.contains(&b'\n') {
        let count = stream.read(&mut buffer).expect("the request arrives");
        assert!(count > 0, "the client closed before its newline");
        seen.extend_from_slice(&buffer[..count]);
    }
}

fn signal() -> (Mutex<mpsc::Sender<()>>, mpsc::Receiver<()>) {
    let (sender, receiver) = mpsc::channel();
    (Mutex::new(sender), receiver)
}

fn raise(sender: &Mutex<mpsc::Sender<()>>) {
    let _ = sender
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .send(());
}

fn await_signal(receiver: &Mutex<mpsc::Receiver<()>>) {
    let _ = receiver
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .recv_timeout(PATIENCE);
}

#[test]
fn the_deadline_before_writing() {
    let root = private_root();
    let socket = socket_in(root.path());
    let fake = FakeDaemon::start(&socket, vec![Answer::Silent]);
    let ran = run_on(line(&socket, &[]), clock_reading(&[0, 3_600_000]));
    assert_eq!(ran.exit, Exit::Unavailable);
    let envelope = failure(&ran);
    assert_eq!(envelope["error"]["code"], "deadline");
    assert_eq!(envelope["error"]["deadline_ms"], 10_000);
    assert_eq!(envelope["instance"]["socket"], text(&socket));
    assert_eq!(fake.ended(1), [b""]);
}

fn no_reply(ran: &Ran) -> (Value, String) {
    assert_eq!(ran.exit, Exit::Unavailable);
    let error = &failure(ran)["error"];
    assert_eq!(error["code"], "no_reply", "{error}");
    let message = error["message"].as_str().expect("a message").to_string();
    (error["cause"].clone(), message)
}

#[test]
fn each_read_waits_only_the_time_left() {
    let root = private_root();
    let socket = socket_in(root.path());
    let _fake = FakeDaemon::start(&socket, vec![Answer::Silent]);
    let ran = run_on(
        line(&socket, &["--deadline-ms", "600"]),
        clock_reading(&[0, 0, 590]),
    );
    assert_eq!(no_reply(&ran).0, "deadline");
    assert!(
        ran.took < Duration::from_millis(300),
        "took {:?} with 10 ms left",
        ran.took
    );
}

fn held_peer(socket: &Path) -> mpsc::Sender<()> {
    let (release, released) = mpsc::channel::<()>();
    peer_at(socket, move |stream| {
        let _ = released.recv_timeout(PATIENCE);
        drop(stream);
    });
    release
}

#[test]
fn each_write_waits_only_the_time_left() {
    let root = private_root();
    let socket = socket_in(root.path());
    let release = held_peer(&socket);
    let name = name_for_a_line_of(MAX_REQUEST_BYTES);
    let calls: &'static Mutex<Vec<Instant>> = Box::leak(Box::new(Mutex::new(Vec::new())));
    let clock = clock_with(&[0, 590, 600], move |reading| {
        if reading >= FIRST_WRITE {
            calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Instant::now());
        }
    });
    let ran = run_on(
        line(&socket, &["--kernel", &name, "--deadline-ms", "600"]),
        clock,
    );
    let _ = release.send(());
    assert_eq!(no_reply(&ran).0, "deadline");
    let calls = calls.lock().unwrap_or_else(PoisonError::into_inner);
    let waited = calls[SECOND_CALL - FIRST_WRITE] - calls[0];
    assert!(
        waited < Duration::from_millis(300),
        "the first write waited {waited:?} with 10 ms left"
    );
}

#[test]
fn a_request_cut_off_midway_says_the_write_failed() {
    let root = private_root();
    let socket = socket_in(root.path());
    let name = name_for_a_line_of(MAX_REQUEST_BYTES);

    let release = held_peer(&socket);
    let ran = run_on(
        line(&socket, &["--kernel", &name, "--deadline-ms", "600"]),
        clock_reading(&[0, 590, 600]),
    );
    let _ = release.send(());
    let (cause, message) = no_reply(&ran);
    assert_eq!(cause, "deadline");
    assert!(
        message.starts_with("the deadline passed before the request was written whole"),
        "{message}"
    );

    let socket = root.path().join("closing.uds");
    let (close, closing) = signal();
    let (closed, gone) = signal();
    peer_at(&socket, move |stream| {
        let _ = closing.recv_timeout(PATIENCE);
        drop(stream);
        raise(&closed);
    });
    let gone = Mutex::new(gone);
    let clock = clock_with(&[0, 9_990], move |reading| {
        if reading == SECOND_CALL {
            raise(&close);
            await_signal(&gone);
        }
    });
    let ran = run_on(line(&socket, &["--kernel", &name]), clock);
    let (cause, message) = no_reply(&ran);
    assert_eq!(cause, "transport");
    assert!(
        message.starts_with("writing the request failed after part of it was sent"),
        "{message}"
    );
}

#[test]
fn a_read_after_the_daemon_closed_still_takes_its_reply() {
    let root = private_root();
    let socket = socket_in(root.path());
    let (closed, gone) = signal();
    peer_at(&socket, move |mut stream| {
        read_request(&mut stream);
        stream
            .write_all(b"{\"id\":1,\"result\":{}}\n")
            .expect("the reply is written");
        drop(stream);
        raise(&closed);
    });
    let gone = Mutex::new(gone);
    let clock = clock_with(&[0], move |reading| {
        if reading == SECOND_CALL {
            await_signal(&gone);
        }
    });
    let ran = run_on(line(&socket, &[]), clock);
    assert_eq!(
        ran.exit,
        Exit::Done,
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert_eq!(one_line(&ran.stdout)["result"], json!({}));
}

#[cfg(target_os = "macos")]
#[test]
fn a_datagram_socket_is_a_transport_failure() {
    let root = private_root();
    let socket = socket_in(root.path());
    let _datagram = std::os::unix::net::UnixDatagram::bind(&socket).expect("a datagram socket");
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
        .expect("the datagram socket is 0600");
    let ran = run(line(&socket, &[]));
    assert_eq!(ran.exit, Exit::Unavailable);
    let error = &failure(&ran)["error"];
    assert_eq!(error["code"], "transport");
    assert_eq!(error["errno"], 41);
    assert_eq!(error["path"], text(&socket));
}

struct Closed;

impl Write for Closed {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from_raw_os_error(EPIPE))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_closed_stdout_is_local_io() {
    let root = private_root();
    let socket = socket_in(root.path());
    let _fake = FakeDaemon::start(&socket, vec![reply(json!({"id":1,"result":{}}))]);
    let context = Context {
        version: "0.0.0",
        commands: COMMANDS,
        trusted_uid: effective_uid(),
        clock: &SystemClock,
    };
    let mut stderr = Vec::new();
    let exit = handle(&line(&socket, &[]), &context, &mut Closed, &mut stderr);
    assert_eq!(exit, Exit::Unavailable);
    let envelope = one_line(&stderr);
    assert_eq!(envelope["command"], "plasmosome.status");
    assert_eq!(envelope["error"]["code"], "local_io");
    assert_eq!(envelope["error"]["errno"], EPIPE);
    assert_eq!(envelope["instance"]["socket"], text(&socket));
}

fn field<'a>(fields: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

fn number(value: Option<&Json>) -> Option<u32> {
    match value {
        Some(Json::Number(number)) => number.as_str().parse().ok(),
        _ => None,
    }
}

fn refused_peer(stream: UnixStream, trusted_uid: u32) -> (ClientCode, Option<u32>, Option<u32>) {
    let deadline = Deadline::new(&SystemClock, SystemClock.now(), 10_000);
    match exchange(
        stream,
        Path::new("/x/control.uds"),
        trusted_uid,
        STATUS,
        &deadline,
    ) {
        Err(Unreplied::NotSent(error)) => (
            error.code(),
            number(field(error.fields(), "uid")),
            number(field(error.fields(), "peer_uid")),
        ),
        Err(unreplied) => panic!("the request was sent: {unreplied:?}"),
        Ok(line) => panic!("the request was answered: {line:?}"),
    }
}

#[test]
fn a_foreign_peer_gets_no_byte() {
    let (ours, mut theirs) = UnixStream::pair().expect("a socket pair");
    theirs
        .set_read_timeout(Some(PATIENCE))
        .expect("a read timeout");
    let stranger = effective_uid() + 1;
    assert_eq!(
        refused_peer(ours, stranger),
        (
            ClientCode::PeerMismatch,
            Some(stranger),
            Some(effective_uid())
        )
    );
    let mut received = Vec::new();
    theirs
        .read_to_end(&mut received)
        .expect("the refused stream is closed");
    assert!(
        received.is_empty(),
        "the refused peer received {received:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_foreign_daemon_is_a_peer_mismatch() {
    let stream =
        UnixStream::connect("/private/var/run/mDNSResponder").expect("mDNSResponder listens");
    let (code, uid, peer_uid) = refused_peer(stream, effective_uid());
    assert_eq!(
        (code, uid),
        (ClientCode::PeerMismatch, Some(effective_uid()))
    );
    assert!(
        peer_uid.is_some_and(|peer| peer != effective_uid()),
        "{peer_uid:?}"
    );
}

#[test]
fn the_sun_path_limit() {
    let root = private_root();
    let room = MAX_PATH - root.path().as_os_str().len() - 1;
    let longest = root.path().join("s".repeat(room));
    assert_eq!(longest.as_os_str().len(), MAX_PATH);
    let fake = FakeDaemon::start(&longest, vec![reply(json!({"id":1,"result":{}}))]);
    assert_eq!(run(line(&longest, &[])).exit, Exit::Done);

    let over = root.path().join("s".repeat(room + 1));
    let ran = run(line(&over, &[]));
    assert_eq!(ran.exit, Exit::Refused);
    let envelope = failure(&ran);
    assert_eq!(
        envelope["error"],
        json!({
            "domain": "client",
            "code": "socket_path_too_long",
            "message": envelope["error"]["message"].clone(),
            "path": text(&over),
            "max_bytes": MAX_PATH,
        })
    );
    assert_eq!(envelope["instance"]["socket"], text(&over));
    assert_eq!(fake.connections(), 1);
}

#[test]
fn the_sun_path_limit_counts_bytes() {
    let root = private_root();
    let room = MAX_PATH + 1 - root.path().as_os_str().len() - 1;
    let wide = char::from_u32(0xe9).expect("a code point").to_string();
    let over = root
        .path()
        .join(format!("{}{}", "s".repeat(room % 2), wide.repeat(room / 2)));
    assert_eq!(over.as_os_str().len(), MAX_PATH + 1);
    assert!(text(&over).chars().count() <= MAX_PATH);
    let ran = run(line(&over, &[]));
    assert_eq!(ran.exit, Exit::Refused);
    let error = &failure(&ran)["error"];
    assert_eq!(
        (&error["code"], &error["max_bytes"]),
        (&json!("socket_path_too_long"), &json!(MAX_PATH))
    );
}

fn controls() -> String {
    [0x1b, 0x9b, 0x7f, 0x202e]
        .into_iter()
        .map(|code| char::from_u32(code).expect("a code point"))
        .collect()
}

#[test]
fn daemon_text_is_escaped() {
    let root = private_root();
    let socket = socket_in(root.path());
    let held = format!("a{}b", controls());
    let mut members = serde_json::Map::new();
    members.insert("s".to_string(), json!(held));
    members.insert(held.clone(), json!(1));
    let result = json!({"id":1,"result":members});
    let error = json!({"id":1,"error":{"code":101,"message":held,"fix":held,"target":held}});
    let _fake = FakeDaemon::start(
        &socket,
        vec![
            reply(result.clone()),
            reply(error.clone()),
            reply(result.clone()),
            reply(error.clone()),
        ],
    );
    let raw = |bytes: &[u8]| {
        let text = String::from_utf8(bytes.to_vec()).expect("the output is UTF-8");
        assert!(
            !text.chars().any(|c| controls().contains(c)),
            "a raw control reached the output: {text:?}"
        );
        text
    };

    let ran = run(line(&socket, &[]));
    raw(&ran.stdout);
    assert_eq!(one_line(&ran.stdout)["result"], result["result"]);
    let ran = run(line(&socket, &[]));
    raw(&ran.stderr);
    let mut relayed = error["error"].clone();
    relayed["domain"] = json!("kernel");
    assert_eq!(failure(&ran)["error"], relayed);

    for ran in [
        run(line(&socket, &["--human"])),
        run(line(&socket, &["--human"])),
    ] {
        let shown = raw(&[ran.stdout, ran.stderr].concat());
        for code in ["<U+001B>", "<U+009B>", "<U+007F>", "<U+202E>"] {
            assert!(shown.contains(code), "{code} is not shown in {shown:?}");
        }
    }
}

#[test]
fn status_needs_a_socket_in_this_build() {
    for line in [&["status"][..], &["status", "--kernel", "a"]] {
        let ran = run(line.iter().map(OsString::from).collect());
        assert_eq!(ran.exit, Exit::Usage, "{line:?}");
        let envelope = failure(&ran);
        assert_eq!(envelope["command"], "invalid");
        assert_eq!(envelope["error"]["code"], "invalid_argument", "{line:?}");
        assert!(envelope["error"].get("argument").is_none(), "{envelope}");
        assert!(envelope.get("instance").is_none(), "{envelope}");
        let message = envelope["error"]["message"].as_str().expect("a message");
        assert!(message.contains("--socket"), "{message}");
    }
}

#[test]
fn a_socket_nobody_listens_on_stays() {
    let root = private_root();
    let socket = socket_in(root.path());
    drop(UnixListener::bind(&socket).expect("a socket"));
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
        .expect("the socket is 0600");
    let inode = std::fs::symlink_metadata(&socket).expect("the entry").ino();
    let ran = run(line(&socket, &[]));
    assert_eq!(ran.exit, Exit::Unavailable);
    let error = &failure(&ran)["error"];
    assert_eq!(error["code"], "unreachable");
    assert_eq!(error["path"], text(&socket));
    assert!(error.get("fix").is_none(), "{error}");
    let after = std::fs::symlink_metadata(&socket).expect("the entry is left");
    assert_eq!(after.ino(), inode);

    let absent = root.path().join("absent.uds");
    let ran = run(line(&absent, &[]));
    assert_eq!(ran.exit, Exit::Unavailable);
    let error = &failure(&ran)["error"];
    assert_eq!(
        (&error["code"], &error["path"]),
        (&json!("not_running"), &json!(text(&absent)))
    );
}

const MAX_REQUEST_BYTES: usize = 1_048_576;

const QUOTES: usize = 1_000;

fn name_for_a_line_of(bytes: usize) -> String {
    let empty = "{\"id\":1,\"method\":\"plasmosome.status\",\"params\":{\"name\":\"\"}}".len();
    let plain = bytes - empty - 2 * QUOTES;
    format!("{}{}", "\"".repeat(QUOTES), "a".repeat(plain))
}

#[test]
fn a_request_line_of_the_bound_is_sent() {
    let root = private_root();
    let socket = socket_in(root.path());
    let fake = FakeDaemon::start(&socket, vec![reply(json!({"id":1,"result":{}}))]);
    let name = name_for_a_line_of(MAX_REQUEST_BYTES);
    let ran = run(line(&socket, &["--kernel", &name]));
    assert_eq!(
        ran.exit,
        Exit::Done,
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let sent = fake.ended(1).remove(0);
    assert_eq!(sent.len(), MAX_REQUEST_BYTES + 1);
    assert_eq!(sent.last(), Some(&b'\n'));
    let request: Value = serde_json::from_slice(&sent).expect("the request is JSON");
    assert_eq!(request["params"]["name"], name.as_str());
}

#[test]
fn a_request_line_over_the_bound_is_refused_before_connecting() {
    let root = private_root();
    let socket = socket_in(root.path());
    let listener = UnixListener::bind(&socket).expect("a socket");
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
        .expect("the socket is 0600");
    listener
        .set_nonblocking(true)
        .expect("the listener does not block");
    let name = name_for_a_line_of(MAX_REQUEST_BYTES + 1);
    assert!(name.len() < MAX_REQUEST_BYTES);
    let ran = run(line(&socket, &["--kernel", &name, "--deadline-ms", "2000"]));
    assert_eq!(ran.exit, Exit::Usage);
    let envelope = failure(&ran);
    assert_eq!(envelope["command"], "invalid");
    assert_eq!(envelope["error"]["code"], "invalid_argument");
    assert_eq!(envelope["error"]["argument"], name.as_str());
    assert_eq!(
        envelope["instance"],
        json!({"name": name, "socket": text(&socket)})
    );
    let accepted = listener.accept().map(|_| ());
    assert!(
        matches!(&accepted, Err(error) if error.kind() == io::ErrorKind::WouldBlock),
        "the listener saw {accepted:?}"
    );
}

fn send(stream: UnixStream, request: &str) -> Result<Vec<u8>, Unreplied> {
    let deadline = Deadline::new(&SystemClock, SystemClock.now(), 10_000);
    exchange(
        stream,
        Path::new("/x/control.uds"),
        effective_uid(),
        request,
        &deadline,
    )
}

#[test]
fn a_write_refused_before_any_byte_is_a_transport_failure() {
    let (ours, theirs) = UnixStream::pair().expect("a socket pair");
    drop(theirs);
    let Err(Unreplied::NotSent(error)) = send(ours, STATUS) else {
        panic!("a request to a closed peer was not refused unsent");
    };
    assert_eq!(error.code(), ClientCode::Transport);
    assert_eq!(number(field(error.fields(), "errno")), Some(EPIPE as u32));
    assert_eq!(
        field(error.fields(), "path"),
        Some(&Json::String("/x/control.uds".to_string()))
    );
}

#[test]
fn a_request_the_deadline_cuts_off_is_cut_off() {
    let (ours, mut theirs) = UnixStream::pair().expect("a socket pair");
    let (done, finished) = mpsc::channel::<()>();
    let reader = thread::spawn(move || {
        let mut first = [0_u8; 1];
        theirs
            .read_exact(&mut first)
            .expect("the first byte arrives");
        let _ = finished.recv_timeout(PATIENCE);
    });
    let request = format!("{}\n", "x".repeat(MAX_REQUEST_BYTES));
    let deadline = Deadline::new(&SystemClock, SystemClock.now(), 300);
    let outcome = exchange(
        ours,
        Path::new("/x/control.uds"),
        effective_uid(),
        &request,
        &deadline,
    );
    let _ = done.send(());
    reader.join().expect("the reader ends");
    assert!(
        matches!(
            &outcome,
            Err(Unreplied::CutOff(error))
                if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
        ),
        "{outcome:?}"
    );
}
