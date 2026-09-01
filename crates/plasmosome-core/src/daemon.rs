use std::io::{BufReader, ErrorKind, Read};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::control::{Controller, serve_connection};
use crate::state::{ControllerState, InstanceName, InstanceNameError};

const ACCEPT_POLL: Duration = Duration::from_millis(25);
const READ_TIMEOUT: Duration = Duration::from_millis(50);
const WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const NO_LEDGER_YET: u64 = 0;

/// Everything `plasmosomed` needs to start: the path it answers on, and which
/// named instance it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub control_socket: PathBuf,
    pub name: InstanceName,
}

/// Why a config text is not a config.
#[derive(Debug)]
pub enum ConfigError {
    NotConfig(serde_json::Error),
    NotAnInstanceName(InstanceNameError),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotConfig(error) => write!(f, "cannot read the config as JSON: {error}"),
            ConfigError::NotAnInstanceName(error) => write!(f, "`name` is unusable: {error}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::NotConfig(error) => Some(error),
            ConfigError::NotAnInstanceName(error) => Some(error),
        }
    }
}

/// Reads a config out of JSON text, or says which part of it is not a config.
///
/// `control_socket` and `name` are both required, and a key the daemon does
/// not know is refused rather than ignored, so a misspelled setting stops the
/// daemon instead of silently not applying. `name` is parsed into an
/// `InstanceName`, so a path-shaped name never reaches the socket layer.
pub fn parse_config(text: &str) -> Result<DaemonConfig, ConfigError> {
    let written: Written = serde_json::from_str(text).map_err(ConfigError::NotConfig)?;
    let name = InstanceName::parse(&written.name).map_err(ConfigError::NotAnInstanceName)?;
    Ok(DaemonConfig {
        control_socket: written.control_socket,
        name,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    #[serde(deserialize_with = "control_socket")]
    control_socket: PathBuf,
    #[serde(deserialize_with = "name")]
    name: String,
}

fn control_socket<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
    PathBuf::deserialize(deserializer)
        .map_err(|error| D::Error::custom(format!("`control_socket` is unusable: {error}")))
}

fn name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    String::deserialize(deserializer)
        .map_err(|error| D::Error::custom(format!("`name` is unusable: {error}")))
}

/// Why the daemon could not start, or could not keep serving.
#[derive(Debug)]
pub enum DaemonError {
    Bind {
        path: PathBuf,
        source: std::io::Error,
    },
    Listener(std::io::Error),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::Bind { path, source } => write!(
                f,
                "cannot bind the control socket {}: {source}",
                path.display()
            ),
            DaemonError::Listener(source) => write!(
                f,
                "the control socket stopped accepting connections: {source}"
            ),
        }
    }
}

impl std::error::Error for DaemonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DaemonError::Bind { source, .. } => Some(source),
            DaemonError::Listener(source) => Some(source),
        }
    }
}

/// What the accept loop does with an error the listener returned.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    Poll,
    Retry,
    Fail,
}

fn accept_outcome(error: &std::io::Error) -> Next {
    match error.kind() {
        ErrorKind::WouldBlock => Next::Poll,
        ErrorKind::Interrupted => Next::Retry,
        _ => Next::Fail,
    }
}

struct BoundSocket {
    path: PathBuf,
}

impl Drop for BoundSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Runs the controller daemon until `shutdown` is set, then tears it down.
///
/// Binds the control socket first, so a start that cannot bind has done
/// nothing else. A path already there is refused whether it is a live daemon's
/// socket or a stale file: the daemon never unlinks a path it did not create,
/// because unlinking a live daemon's socket is how a half-alive controller is
/// manufactured. Returning — cleanly, with an error raised after the bind, or
/// through an unwinding panic — removes the socket path.
///
/// Connections are taken one at a time and each is answered in request order.
/// The flag is checked between accepts and between reads, and both halves of a
/// connection are bounded by a timeout, so neither an idle client nor one that
/// never reads its replies holds the daemon open past shutdown. The instance
/// starts with no cells and at ledger generation zero; nothing here adds one.
///
/// A handler panic is not caught: the connection loop answers `-32603` and
/// resumes the unwind, which passes through this function and removes the
/// socket path on the way out. **What no destructor can cover is `SIGKILL` of
/// the daemon itself** — a killed process runs none, so the socket path stays.
/// That residue is observed rather than prevented.
pub fn run(config: DaemonConfig, shutdown: &AtomicBool) -> Result<(), DaemonError> {
    let listener =
        UnixListener::bind(&config.control_socket).map_err(|source| DaemonError::Bind {
            path: config.control_socket.clone(),
            source,
        })?;
    let _bound = BoundSocket {
        path: config.control_socket,
    };
    listener
        .set_nonblocking(true)
        .map_err(DaemonError::Listener)?;

    let mut controller = Controller::new(config.name, ControllerState::default(), NO_LEDGER_YET);
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => converse(stream, shutdown, &mut controller),
            Err(error) => match accept_outcome(&error) {
                Next::Poll => std::thread::sleep(ACCEPT_POLL),
                Next::Retry => {}
                Next::Fail => return Err(DaemonError::Listener(error)),
            },
        }
    }
    Ok(())
}

fn converse(stream: UnixStream, shutdown: &AtomicBool, controller: &mut Controller) {
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(READ_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        return;
    }
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let reader = BufReader::new(FlaggedReads { stream, shutdown });
    let _ = serve_connection(reader, writer, controller);
}

struct FlaggedReads<'a> {
    stream: UnixStream,
    shutdown: &'a AtomicBool,
}

impl Read for FlaggedReads<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                return Err(std::io::Error::other("the daemon was asked to stop"));
            }
            match self.stream.read(buffer) {
                Err(error) if waiting(&error) => continue,
                outcome => return outcome,
            }
        }
    }
}

fn waiting(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, ErrorKind, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    const STATUS_REQUEST: &str = r#"{"id":1,"method":"plasmosome.status","params":{}}"#;
    const PATIENCE: Duration = Duration::from_secs(10);

    fn config(control_socket: &Path, name: &str) -> DaemonConfig {
        DaemonConfig {
            control_socket: control_socket.to_path_buf(),
            name: InstanceName::parse(name).expect("the test names a valid instance"),
        }
    }

    struct Running {
        shutdown: Arc<AtomicBool>,
        finished: mpsc::Receiver<()>,
        handle: Option<thread::JoinHandle<Result<(), DaemonError>>>,
    }

    impl Running {
        fn stop(mut self) -> Result<(), DaemonError> {
            self.shutdown.store(true, Ordering::Relaxed);
            self.finished
                .recv_timeout(PATIENCE)
                .expect("run returns once the shutdown flag is set");
            self.handle
                .take()
                .expect("the daemon thread is joined once")
                .join()
                .expect("the daemon thread does not panic")
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn start(config: DaemonConfig) -> Running {
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let (done, finished) = mpsc::channel();
        let handle = thread::spawn(move || {
            let outcome = run(config, &flag);
            let _ = done.send(());
            outcome
        });
        Running {
            shutdown,
            finished,
            handle: Some(handle),
        }
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
                "the daemon is addressable on {} within five seconds",
                socket.display()
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn ask(client: &mut BufReader<UnixStream>, line: &str) -> Value {
        let mut stream = client.get_ref().try_clone().expect("clone for writing");
        writeln!(stream, "{line}").expect("the request reaches the daemon");
        stream.flush().expect("the request is flushed");
        let mut reply = String::new();
        let read = client.read_line(&mut reply).expect("the daemon answers");
        assert_ne!(
            read, 0,
            "the daemon answered rather than closing the socket"
        );
        serde_json::from_str(&reply)
            .unwrap_or_else(|error| panic!("the daemon answers JSON, got {reply:?}: {error}"))
    }

    #[test]
    fn a_full_config_parses_and_each_malformed_config_is_refused_by_name() {
        let parsed = parse_config(r#"{"control_socket": "/tmp/c.uds", "name": "work"}"#)
            .expect("a full config parses");
        assert_eq!(
            parsed,
            DaemonConfig {
                control_socket: PathBuf::from("/tmp/c.uds"),
                name: InstanceName::parse("work").expect("`work` is an instance name"),
            }
        );

        for (text, offender) in [
            (r#"not json"#, "JSON"),
            (r#"{"name": "work"}"#, "control_socket"),
            (r#"{"control_socket": "/tmp/c.uds"}"#, "name"),
            (r#"{"control_socket": 7, "name": "work"}"#, "control_socket"),
            (r#"{"control_socket": "/tmp/c.uds", "name": 7}"#, "name"),
            (
                r#"{"control_socket": "/tmp/c.uds", "name": "work", "socket": "/tmp/x.uds"}"#,
                "socket",
            ),
            (r#"{"control_socket": "/tmp/c.uds", "name": ""}"#, "empty"),
            (r#"{"control_socket": "/tmp/c.uds", "name": "a/b"}"#, "a/b"),
            (r#"{"control_socket": "/tmp/c.uds", "name": ".."}"#, ".."),
        ] {
            let refusal = parse_config(text)
                .err()
                .unwrap_or_else(|| panic!("{text} is refused"));
            assert!(
                refusal.to_string().contains(offender),
                "the refusal of {text} names {offender}, got: {refusal}"
            );
        }
    }

    #[test]
    fn the_daemon_answers_status_with_an_empty_cell_registry() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");
        let control = directory.path().join("control.uds");
        let daemon = start(config(&control, "work"));

        let mut client = addressable(&control);
        let reply = ask(&mut client, STATUS_REQUEST);
        assert_eq!(
            reply.get("error"),
            None,
            "a served status is not an error: {reply}"
        );
        let result = reply
            .get("result")
            .unwrap_or_else(|| panic!("status answered with no result: {reply}"));
        assert_eq!(
            result.get("name").and_then(Value::as_str),
            Some("work"),
            "the daemon answers for the instance its config named: {result}"
        );
        assert_eq!(
            result.get("state").and_then(Value::as_str),
            Some("running"),
            "{result}"
        );
        assert_eq!(
            result.get("ready").and_then(Value::as_bool),
            Some(true),
            "{result}"
        );
        assert_eq!(
            result
                .pointer("/controller/ledger_generation")
                .and_then(Value::as_u64),
            Some(0),
            "a controller with no ledger yet is at generation zero: {result}"
        );
        assert_eq!(
            result.get("cells"),
            Some(&json!([])),
            "a daemon that has started no cell has none to report: {result}"
        );

        drop(client);
        daemon.stop().expect("a clean shutdown is not an error");
        assert!(
            !control.exists(),
            "the control socket path is removed on shutdown"
        );
    }

    #[test]
    fn a_status_for_a_name_this_daemon_is_not_is_refused_on_the_wire() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");
        let control = directory.path().join("control.uds");
        let _daemon = start(config(&control, "work"));

        let mut client = addressable(&control);
        let elsewhere = ask(
            &mut client,
            r#"{"id":1,"method":"plasmosome.status","params":{"name":"elsewhere"}}"#,
        );
        assert_eq!(
            elsewhere.pointer("/error/code").and_then(Value::as_i64),
            Some(101),
            "a daemon answers for the instance it is, never for one it is not: {elsewhere}"
        );
        assert_eq!(
            elsewhere.pointer("/error/target").and_then(Value::as_str),
            Some("plasmosome elsewhere"),
            "the refusal names the target that was asked for: {elsewhere}"
        );
        assert_eq!(elsewhere.get("result"), None, "{elsewhere}");

        let mistyped = ask(
            &mut client,
            r#"{"id":2,"method":"plasmosome.status","params":{"name":7}}"#,
        );
        assert_eq!(
            mistyped.pointer("/error/code").and_then(Value::as_i64),
            Some(-32602),
            "params that do not parse are invalid params: {mistyped}"
        );
    }

    #[test]
    fn an_existing_control_socket_path_is_refused_and_the_path_is_not_unlinked() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");
        let control = directory.path().join("control.uds");
        std::fs::write(&control, b"someone else's socket").expect("the test occupies the path");

        let shutdown = AtomicBool::new(false);
        let refusal = run(config(&control, "work"), &shutdown)
            .expect_err("an occupied control socket path refuses the start");

        match &refusal {
            DaemonError::Bind { path, .. } => assert_eq!(path, &control),
            other => panic!("an occupied path is a bind failure, got {other:?}"),
        }
        assert!(
            control.exists(),
            "the daemon never unlinks a path it did not bind: a live daemon's socket removed \
             out from under it is how a half-alive controller is manufactured"
        );
    }

    #[test]
    fn shutdown_stops_serve_even_with_an_idle_connection_open() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");
        let control = directory.path().join("control.uds");
        let daemon = start(config(&control, "work"));

        let mut client = addressable(&control);
        assert_eq!(
            ask(&mut client, STATUS_REQUEST).pointer("/result/ready"),
            Some(&json!(true)),
            "the daemon is inside the connection before the flag is set"
        );

        daemon
            .stop()
            .expect("run returns once the flag is set, with the client still connected");
    }

    #[test]
    fn shutdown_stops_serve_even_with_a_client_that_never_reads_its_replies() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");
        let control = directory.path().join("control.uds");
        let daemon = start(config(&control, "work"));

        let client = addressable(&control);
        let mut writer = client
            .get_ref()
            .try_clone()
            .expect("the test clones its own writing half");
        writer
            .set_write_timeout(Some(Duration::from_millis(200)))
            .expect("the test bounds its own writes so a wedged daemon cannot hang it");
        for _ in 0..20_000 {
            if writeln!(writer, "{STATUS_REQUEST}").is_err() {
                break;
            }
        }
        let _ = writer.flush();

        daemon.stop().expect(
            "run returns once the flag is set even against a client that never reads; a write \
             with no timeout blocks forever and turns the covered SIGTERM teardown into the \
             uncovered SIGKILL residue path",
        );
    }

    #[test]
    fn a_connection_survives_a_parse_error_and_a_second_connection_is_served() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");
        let control = directory.path().join("control.uds");
        let _daemon = start(config(&control, "work"));

        let mut client = addressable(&control);
        let refused = ask(&mut client, "not json at all");
        assert_eq!(
            refused.pointer("/error/code").and_then(Value::as_i64),
            Some(-32700),
            "{refused}"
        );
        assert_eq!(refused.get("id"), Some(&Value::Null), "{refused}");
        assert_eq!(
            ask(&mut client, STATUS_REQUEST).pointer("/result/name"),
            Some(&json!("work")),
            "the conversation continues after a parse error"
        );
        drop(client);

        let mut second = addressable(&control);
        assert_eq!(
            ask(&mut second, STATUS_REQUEST).pointer("/result/name"),
            Some(&json!("work")),
            "a second connection is served once the first closes"
        );
    }

    #[test]
    fn an_accept_error_is_classified_by_what_it_means_for_the_loop() {
        assert_eq!(
            accept_outcome(&std::io::Error::from(ErrorKind::WouldBlock)),
            Next::Poll,
            "a nonblocking listener with nothing pending is the idle case, not a failure",
        );
        assert_eq!(
            accept_outcome(&std::io::Error::from(ErrorKind::Interrupted)),
            Next::Retry,
            "a signal interrupting accept is retried, not a failure",
        );
        assert_eq!(
            accept_outcome(&std::io::Error::from(ErrorKind::Other)),
            Next::Fail,
            "any other accept error is a listener that stopped working",
        );
    }
}
