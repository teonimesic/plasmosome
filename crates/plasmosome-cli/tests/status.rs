mod support;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
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

struct LateClock {
    start: Instant,
    reads: AtomicUsize,
}

impl Clock for LateClock {
    fn now(&self) -> Instant {
        match self.reads.fetch_add(1, Ordering::Relaxed) {
            0 => self.start,
            _ => self.start + Duration::from_secs(3600),
        }
    }
}

#[test]
fn the_deadline_before_writing() {
    let root = private_root();
    let socket = socket_in(root.path());
    let fake = FakeDaemon::start(&socket, vec![Answer::Silent]);
    let clock = Box::leak(Box::new(LateClock {
        start: Instant::now(),
        reads: AtomicUsize::new(0),
    }));
    let ran = run_on(line(&socket, &[]), clock);
    assert_eq!(ran.exit, Exit::Unavailable);
    let envelope = failure(&ran);
    assert_eq!(envelope["error"]["code"], "deadline");
    assert_eq!(envelope["error"]["deadline_ms"], 10_000);
    assert_eq!(envelope["instance"]["socket"], text(&socket));
    assert_eq!(fake.ended(1), [b""]);
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
        Err(Unreplied::Unanswered(failure)) => panic!("the request was sent: {failure:?}"),
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
