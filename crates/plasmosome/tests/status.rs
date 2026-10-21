use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use plasmosome_core::daemon;
use serde_json::{Value, json};
use tempfile::TempDir;

const BINARY: &str = env!("CARGO_BIN_EXE_plasmosome");

const PATIENCE: Duration = Duration::from_secs(10);

const EPIPE: i64 = 32;

#[cfg(target_os = "macos")]
fn base() -> PathBuf {
    let base = std::env::temp_dir();
    base.canonicalize()
        .unwrap_or_else(|error| panic!("{} does not resolve: {error}", base.display()))
}

#[cfg(not(target_os = "macos"))]
fn base() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|base| !base.is_empty())
        .map_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")), PathBuf::from)
}

fn private_dir(base: &Path) -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("pb")
        .tempdir_in(base)
        .unwrap_or_else(|error| panic!("no directory under {}: {error}", base.display()));
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).expect("0700");
    dir
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

fn run(args: &[&str], configure: impl FnOnce(&mut Command)) -> Output {
    let mut command = Command::new(BINARY);
    command.args(args).stdin(Stdio::null());
    configure(&mut command);
    command.output().expect("the plasmosome binary runs")
}

fn one_line(bytes: &[u8]) -> Value {
    let text = std::str::from_utf8(bytes).expect("the output is UTF-8");
    assert!(
        text.ends_with('\n') && text.matches('\n').count() == 1,
        "the output is not one line: {text:?}"
    );
    serde_json::from_str(text).expect("the output is one JSON line")
}

fn failure(output: &Output, code: i32) -> Value {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    one_line(&output.stderr)
}

struct Daemon {
    stop: &'static AtomicBool,
    thread: Option<JoinHandle<()>>,
}

impl Daemon {
    fn start(root: &Path, socket: &Path) -> Daemon {
        let config = json!({
            "control_socket": socket,
            "name": "work",
            "instance_root": root.join("instance"),
            "recovery_deadline_ms": 5000,
        });
        let config = daemon::parse_config(&config.to_string()).expect("a valid configuration");
        let stop: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        let thread = thread::spawn(move || {
            daemon::run(config, stop).expect("plasmosomed serves until it is stopped");
        });
        let until = Instant::now() + PATIENCE;
        while !socket.exists() {
            assert!(
                Instant::now() < until,
                "plasmosomed did not bind {socket:?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
        Daemon {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let until = Instant::now() + Duration::from_secs(2);
            while !thread.is_finished() && Instant::now() < until {
                thread::sleep(Duration::from_millis(5));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}

fn raw_status(socket: &Path) -> Value {
    let mut stream = UnixStream::connect(socket).expect("a raw connection");
    stream.set_read_timeout(Some(PATIENCE)).expect("a timeout");
    stream
        .write_all(b"{\"id\":1,\"method\":\"plasmosome.status\",\"params\":{}}\n")
        .expect("the raw request is written");
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .expect("the raw reply");
    serde_json::from_str::<Value>(&line).expect("a JSON reply")["result"].take()
}

fn without_uptime(mut result: Value) -> Value {
    let controller = result["controller"]
        .as_object_mut()
        .expect("the result has controller");
    assert!(
        controller
            .remove("uptime_ms")
            .is_some_and(|uptime| uptime.is_u64())
    );
    result
}

#[test]
fn status_against_the_real_daemon() {
    let root = private_dir(&base());
    let socket = root.path().join("control.uds");
    let _daemon = Daemon::start(root.path(), &socket);

    let output = run(&["status", "--socket", text(&socket)], |_| {});
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let mut envelope = one_line(&output.stdout);
    let mut keys: Vec<&String> = envelope.as_object().expect("an object").keys().collect();
    keys.sort();
    assert_eq!(keys, ["command", "instance", "result", "schema"]);
    assert_eq!(envelope["schema"], 1);
    assert_eq!(envelope["command"], "plasmosome.status");
    assert_eq!(envelope["instance"], json!({"socket": text(&socket)}));
    assert_eq!(
        without_uptime(envelope["result"].take()),
        without_uptime(raw_status(&socket))
    );

    let (reader, writer) = std::io::pipe().expect("a pipe");
    drop(reader);
    let closed = run(&["status", "--socket", text(&socket)], |command| {
        command.stdout(writer);
    });
    let envelope = failure(&closed, 1);
    assert_eq!(envelope["error"]["code"], "local_io");
    assert_eq!(envelope["error"]["errno"], EPIPE);
    assert_eq!(envelope["instance"]["socket"], text(&socket));
}

fn listener_at(socket: &Path) -> UnixListener {
    let listener = UnixListener::bind(socket).expect("a live socket");
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600)).expect("0600");
    listener.set_nonblocking(true).expect("nonblocking");
    listener
}

fn assert_never_reached(listener: &UnixListener, what: &str) {
    match listener.accept() {
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        other => panic!("{what} reached the socket: {other:?}"),
    }
}

fn unsafe_socket(socket: &Path) -> Value {
    let envelope = failure(&run(&["status", "--socket", text(socket)], |_| {}), 2);
    assert_eq!(envelope["error"]["code"], "unsafe_socket", "{envelope}");
    envelope["error"].clone()
}

#[test]
fn unsafe_paths_are_refused_before_connecting() {
    let live = private_dir(&base());
    let target = live.path().join("control.uds");
    let listener = listener_at(&target);
    let linked = private_dir(&base());
    let link = linked.path().join("control.uds");
    std::os::unix::fs::symlink(&target, &link).expect("a symbolic link");
    let error = unsafe_socket(&link);
    assert_eq!(
        (&error["rule"], &error["path"]),
        (&json!("not_socket"), &json!(text(&link)))
    );
    assert_never_reached(&listener, "a link at the socket entry");

    let root = private_dir(&base());
    let open = root.path().join("it's open");
    fs::create_dir(&open).expect("the directory");
    let socket = open.join("control.uds");
    let listener = listener_at(&socket);
    fs::set_permissions(&open, fs::Permissions::from_mode(0o750)).expect("0750");
    let error = unsafe_socket(&socket);
    assert_eq!(
        (&error["rule"], &error["path"]),
        (&json!("mode"), &json!(text(&open)))
    );
    let fix = error["fix"].as_str().expect("a fix");
    let fixed = Command::new("/bin/sh")
        .args(["-c", fix])
        .status()
        .expect("sh runs");
    assert!(fixed.success(), "{fix}");
    let mode = fs::metadata(&open)
        .expect("the directory")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700, "{fix}");
    assert_never_reached(&listener, "a 0750 directory");
}

#[cfg(target_os = "macos")]
#[test]
fn a_socket_under_private_tmp_is_refused() {
    let dir = private_dir(Path::new("/private/tmp"));
    let socket = dir.path().join("control.uds");
    let listener = listener_at(&socket);
    let error = unsafe_socket(&socket);
    assert_eq!(
        (&error["rule"], &error["path"]),
        (&json!("mode"), &json!("/private/tmp"))
    );
    assert_never_reached(&listener, "a socket under /private/tmp");
}

#[test]
fn usage_errors_through_the_binary() {
    let root = private_dir(&base());
    let socket = root.path().join("control.uds");
    let listener = listener_at(&socket);
    let p = text(&socket);
    let escape = char::from_u32(0x1b).expect("ESC is a code point");
    let escaped = format!("a{escape}b");
    let lossy = format!("a{}b", char::REPLACEMENT_CHARACTER);
    let rows: Vec<(Vec<OsString>, Option<&str>)> = vec![
        (os(&["frobnicate", "--socket", p]), Some("frobnicate")),
        (
            os(&["frobnicate", "--socket", p, "--help"]),
            Some("frobnicate"),
        ),
        (os(&["status", "--socket", p, "--frob"]), Some("--frob")),
        (
            os(&["status", "--socket", p, "--frob", "extra"]),
            Some("--frob"),
        ),
        (
            os(&["status", "--socket", p, "--kernel", "a", "--kernel", "b"]),
            Some("--kernel"),
        ),
        (
            os(&["status", "--socket", p, "--kernel", "../x"]),
            Some("../x"),
        ),
        (
            os(&["status", "--socket", p, "--kernel", "../x", "extra"]),
            Some("../x"),
        ),
        (
            os(&["status", "--socket", p, "--kernel", &escaped]),
            Some(&escaped),
        ),
        (
            os(&["status", "--socket", p, "--kernel", "--"]),
            Some("--kernel"),
        ),
        (
            os(&["status", "--socket", p, "--kernel=work"]),
            Some("--kernel=work"),
        ),
        (os(&["status", "--socket", p, "-"]), Some("-")),
        (os(&["status", "--socket", p, "-5"]), Some("-5")),
        (
            os(&["status", "--socket", "relative/control.uds"]),
            Some("relative/control.uds"),
        ),
        (
            os(&["status", "--socket", "/a//b/control.uds"]),
            Some("/a//b/control.uds"),
        ),
        (
            os(&["status", "--socket", p, "--deadline-ms", "0"]),
            Some("0"),
        ),
        (
            os(&["status", "--socket", p, "--deadline-ms", "3600001"]),
            Some("3600001"),
        ),
        (
            os(&["status", "--socket", p, "--json", "--human"]),
            Some("--human"),
        ),
        (
            os(&["status", "--socket", p, "--version"]),
            Some("--version"),
        ),
        (
            os(&["status", "--socket", p, "--trusted-uid", "0"]),
            Some("--trusted-uid"),
        ),
        (os(&["status", "--socket", p, "--", "x"]), Some("--")),
        (
            os(&["--frob", "--kernel", "../x", "status", "--socket", p]),
            Some("--frob"),
        ),
        (
            [
                os(&["status", "--socket", p]),
                vec![OsString::from_vec(vec![b'a', 0xff, b'b'])],
            ]
            .concat(),
            Some(&lossy),
        ),
    ];
    for (line, argument) in rows {
        let output = Command::new(BINARY)
            .args(&line)
            .stdin(Stdio::null())
            .output()
            .expect("the plasmosome binary runs");
        let envelope = failure(&output, 64);
        let mut keys: Vec<&String> = envelope.as_object().expect("an object").keys().collect();
        keys.sort();
        assert_eq!(keys, ["command", "error", "schema"], "{line:?}");
        assert_eq!(envelope["command"], "invalid", "{line:?}");
        assert_eq!(envelope["error"]["code"], "invalid_argument", "{line:?}");
        assert_eq!(
            envelope["error"].get("argument").and_then(Value::as_str),
            argument,
            "{line:?}"
        );
    }

    let human = run(
        &["status", "--socket", p, "--human", "--", "--human"],
        |_| {},
    );
    assert_eq!(human.status.code(), Some(64), "{human:?}");
    let shown = std::str::from_utf8(&human.stderr).expect("UTF-8");
    assert!(
        shown.ends_with("\n(client invalid_argument)\n"),
        "{shown:?}"
    );

    let help = run(&["status", "--socket", p, "--help"], |_| {});
    assert_eq!(help.status.code(), Some(0), "{help:?}");
    assert_never_reached(&listener, "a usage error or help");
}

fn os(line: &[&str]) -> Vec<OsString> {
    line.iter().map(OsString::from).collect()
}
