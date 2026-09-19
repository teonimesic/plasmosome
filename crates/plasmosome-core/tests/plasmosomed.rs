use plasmosome_core::MAX_LINE_BYTES;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const STATUS_REQUEST: &str = r#"{"id":1,"method":"plasmosome.status","params":{}}"#;
const PATIENCE: Duration = Duration::from_secs(10);

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
                    "plasmosomed exits within {budget:?} of the signal"
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

fn plasmosomed(arguments: &[&Path]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_plasmosomed"));
    command.args(arguments).stderr(Stdio::piped());
    command
}

fn start(config: &Path) -> Daemon {
    Daemon {
        child: plasmosomed(&[config])
            .spawn()
            .expect("plasmosomed starts as a process"),
    }
}

fn output_within(mut command: Command, budget: Duration) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .spawn()
        .expect("plasmosomed starts as a process");
    let deadline = Instant::now() + budget;
    while child
        .try_wait()
        .expect("the daemon's state is readable")
        .is_none()
    {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "plasmosomed exits within {budget:?} rather than serving; a start that should \
                 have been refused otherwise runs forever, and a test that only inspects the \
                 exit status waits for it forever instead of failing"
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
    child
        .wait_with_output()
        .expect("the daemon's output is readable")
}

fn write_config(path: &Path, body: &Value) {
    std::fs::write(
        path,
        serde_json::to_string(body).expect("the config is JSON"),
    )
    .expect("the config is written");
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
            "plasmosomed is addressable on {} within five seconds",
            socket.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn send(client: &BufReader<UnixStream>, bytes: &[u8]) {
    let mut stream = client.get_ref().try_clone().expect("clone for writing");
    stream
        .write_all(bytes)
        .expect("the request reaches plasmosomed");
    stream.write_all(b"\n").expect("the request line ends");
    stream.flush().expect("the request is flushed");
}

fn read_reply(client: &mut BufReader<UnixStream>) -> Value {
    let mut reply = String::new();
    let read = client.read_line(&mut reply).expect("plasmosomed answers");
    assert_ne!(
        read, 0,
        "plasmosomed answered rather than closing the socket"
    );
    serde_json::from_str(&reply)
        .unwrap_or_else(|error| panic!("plasmosomed answers JSON, got {reply:?}: {error}"))
}

fn ask(client: &mut BufReader<UnixStream>, line: &str) -> Value {
    send(client, line.as_bytes());
    read_reply(client)
}

fn padded_status(total: usize) -> (String, String) {
    let framing = r#"{"id":"","method":"plasmosome.status","params":{}}"#;
    let id = "x".repeat(total - framing.len());
    let line = format!(r#"{{"id":"{id}","method":"plasmosome.status","params":{{}}}}"#);
    assert_eq!(
        line.len(),
        total,
        "the padded line is exactly the length the test asked for"
    );
    (line, id)
}

fn one_instance(control: &Path, name: &str) -> Value {
    json!({"control_socket": control, "name": name})
}

#[test]
fn the_envelope_edges_hold_on_the_wire() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");
    let control = directory.path().join("control.uds");
    let config = directory.path().join("config.json");
    write_config(&config, &one_instance(&control, "work"));

    let _daemon = start(&config);
    let mut client = addressable(&control);

    let unparseable = ask(&mut client, "not json at all");
    assert_eq!(
        unparseable.pointer("/error/code").and_then(Value::as_i64),
        Some(-32700),
        "{unparseable}"
    );
    assert_eq!(unparseable.get("id"), Some(&Value::Null), "{unparseable}");
    assert_eq!(
        ask(&mut client, STATUS_REQUEST).pointer("/result/name"),
        Some(&json!("work")),
        "the conversation continues after a line that is not JSON"
    );

    let mut not_text: Vec<u8> =
        br#"{"id":1,"method":"plasmosome.status","params":{"pad":""#.to_vec();
    not_text.push(0xFF);
    not_text.extend_from_slice(br#""}}"#);
    send(&client, &not_text);
    let refused = read_reply(&mut client);
    assert_eq!(
        refused.pointer("/error/code").and_then(Value::as_i64),
        Some(-32700),
        "a line that is not UTF-8 is not JSON: {refused}"
    );
    assert_eq!(
        ask(&mut client, STATUS_REQUEST).pointer("/result/name"),
        Some(&json!("work")),
        "the conversation continues after a line that is not UTF-8"
    );

    let (at_cap, id) = padded_status(MAX_LINE_BYTES);
    let served = ask(&mut client, &at_cap);
    assert_eq!(
        served.pointer("/result/cells"),
        Some(&json!([])),
        "a line of exactly the cap is served: {served}",
    );
    assert_eq!(
        served.get("id"),
        Some(&Value::String(id)),
        "the id of a line at the cap comes back verbatim"
    );

    let unknown = ask(
        &mut client,
        r#"{"id":[1,{"deep":true}],"method":"plasmosome.nope","params":{}}"#,
    );
    assert_eq!(
        unknown.pointer("/error/code").and_then(Value::as_i64),
        Some(-32601),
        "{unknown}"
    );
    assert_eq!(
        unknown.get("id"),
        Some(&json!([1, {"deep": true}])),
        "any JSON id is echoed verbatim: {unknown}"
    );
    drop(client);

    let mut over_cap = addressable(&control);
    let (too_long, _) = padded_status(MAX_LINE_BYTES + 1);
    send(&over_cap, too_long.as_bytes());
    let refusal = read_reply(&mut over_cap);
    assert_eq!(
        refusal.pointer("/error/code").and_then(Value::as_i64),
        Some(-32600),
        "one byte past the cap is refused: {refusal}"
    );
    assert_eq!(refusal.get("id"), Some(&Value::Null), "{refusal}");
    let mut after = String::new();
    assert_eq!(
        over_cap.read_line(&mut after).ok(),
        Some(0),
        "the connection closes after an over-long line, got {after:?}"
    );
}

#[test]
fn plasmosomed_serves_status_and_dies_cleanly_on_sigterm() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");
    let control = directory.path().join("control.uds");
    let config = directory.path().join("config.json");
    write_config(&config, &one_instance(&control, "work"));

    let mut daemon = start(&config);
    let mut client = addressable(&control);
    let reply = ask(&mut client, STATUS_REQUEST);
    assert_eq!(
        reply.pointer("/result/name"),
        Some(&json!("work")),
        "{reply}"
    );
    assert_eq!(
        reply.pointer("/result/cells"),
        Some(&json!([])),
        "a daemon that has started no cell has none to report: {reply}"
    );
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
}

#[test]
fn plasmosomed_exits_nonzero_naming_the_failure() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");

    let no_arguments = output_within(plasmosomed(&[]), PATIENCE);
    assert_eq!(
        no_arguments.status.code(),
        Some(2),
        "plasmosomed needs a config"
    );
    assert!(
        String::from_utf8_lossy(&no_arguments.stderr).contains("usage"),
        "plasmosomed says how it is called"
    );

    let absent = directory.path().join("absent.json");
    let unreadable = output_within(plasmosomed(&[&absent]), PATIENCE);
    assert_eq!(
        unreadable.status.code(),
        Some(2),
        "an unreadable config is refused"
    );
    assert!(
        String::from_utf8_lossy(&unreadable.stderr).contains("absent.json"),
        "the refusal names the config it could not read"
    );

    let malformed = directory.path().join("malformed.json");
    std::fs::write(&malformed, b"{not json").expect("the test writes a malformed config");
    let invalid = output_within(plasmosomed(&[&malformed]), PATIENCE);
    assert_eq!(
        invalid.status.code(),
        Some(2),
        "a config that is not JSON is refused"
    );

    let control = directory.path().join("taken.uds");
    std::fs::write(&control, b"someone else's socket").expect("the test occupies the path");
    let occupied = directory.path().join("occupied.json");
    write_config(&occupied, &one_instance(&control, "work"));
    let refused = output_within(plasmosomed(&[&occupied]), PATIENCE);
    assert_eq!(
        refused.status.code(),
        Some(1),
        "a control socket path already in use refuses the start"
    );
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("taken.uds"),
        "the refusal names the path it could not bind, got: {}",
        String::from_utf8_lossy(&refused.stderr)
    );

    let live = directory.path().join("live.uds");
    let holder = UnixListener::bind(&live).expect("the test binds a live control socket");
    let held = directory.path().join("live.json");
    write_config(&held, &one_instance(&live, "work"));
    let refused = output_within(plasmosomed(&[&held]), PATIENCE);
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

    let link = directory.path().join("link.uds");
    std::os::unix::fs::symlink("nowhere.uds", &link)
        .expect("the test plants a dangling leaf symlink");
    let linked = directory.path().join("link.json");
    write_config(&linked, &one_instance(&link, "work"));
    let refused = output_within(plasmosomed(&[&linked]), PATIENCE);
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

#[test]
fn plasmosomed_leaves_a_replacement_settled_at_its_socket_path_alone() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");
    let control = directory.path().join("control.uds");
    let config = directory.path().join("config.json");
    write_config(&config, &one_instance(&control, "work"));

    let mut daemon = start(&config);
    let mut client = addressable(&control);
    assert_eq!(
        ask(&mut client, STATUS_REQUEST).pointer("/result/name"),
        Some(&json!("work")),
        "the daemon answers before its pathname is exchanged"
    );
    drop(client);

    let original = std::fs::symlink_metadata(&control).expect("the bound socket is on disk");
    let saved = directory.path().join("saved.uds");
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
fn plasmosomed_leaves_a_symlink_settled_at_its_socket_path_alone() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");
    let control = directory.path().join("control.uds");
    let config = directory.path().join("config.json");
    write_config(&config, &one_instance(&control, "work"));

    let mut daemon = start(&config);
    let mut client = addressable(&control);
    assert_eq!(
        ask(&mut client, STATUS_REQUEST).pointer("/result/name"),
        Some(&json!("work")),
        "the daemon answers before its pathname is exchanged"
    );
    drop(client);

    let saved = directory.path().join("saved.uds");
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
fn plasmosomed_leaves_a_dangling_symlink_settled_at_its_socket_path_alone() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");
    let control = directory.path().join("control.uds");
    let config = directory.path().join("config.json");
    write_config(&config, &one_instance(&control, "work"));

    let mut daemon = start(&config);
    let mut client = addressable(&control);
    assert_eq!(
        ask(&mut client, STATUS_REQUEST).pointer("/result/name"),
        Some(&json!("work")),
        "the daemon answers before its pathname is exchanged"
    );
    drop(client);

    let saved = directory.path().join("saved.uds");
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
        !directory.path().join("nowhere.uds").exists(),
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
