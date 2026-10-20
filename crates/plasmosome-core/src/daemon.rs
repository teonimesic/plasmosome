use std::io::{BufReader, ErrorKind, Read};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
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

/// Everything `plasmosomed` needs to start: the path it answers on, which
/// named instance it is, where that instance's durable state lives, how long
/// startup recovery may take, and where trusted registry imports live, if
/// anywhere. Only `parse_config` checks the rules below; a value built by hand
/// carries no such promise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub control_socket: PathBuf,
    pub name: InstanceName,
    /// Absolute, and never derived from `control_socket`. A caller opens it
    /// with [`crate::instance::InstanceRoot::open`]; this build does not.
    pub instance_root: PathBuf,
    /// Never zero. This build does not recover cells, so nothing waits on it
    /// yet.
    pub recovery_deadline: Duration,
    /// Absolute when present. `None` means purely local operation: no other
    /// path, `HOME` or request supplies a default.
    pub registry_root: Option<PathBuf>,
}

/// Why a config text is not a config.
#[derive(Debug)]
pub enum ConfigError {
    NotConfig(serde_json::Error),
    NotAnInstanceName(InstanceNameError),
    /// `key` is `instance_root` or `registry_root`, and `path` is what the
    /// config wrote for it.
    NotAbsolute {
        key: &'static str,
        path: PathBuf,
    },
    ZeroRecoveryDeadline,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotConfig(error) => write!(f, "cannot read the config as JSON: {error}"),
            ConfigError::NotAnInstanceName(error) => write!(f, "`name` is unusable: {error}"),
            ConfigError::NotAbsolute { key, path } => {
                write!(f, "`{key}` must be an absolute path, got {path:?}")
            }
            ConfigError::ZeroRecoveryDeadline => {
                write!(f, "`recovery_deadline_ms` must be at least 1")
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::NotConfig(error) => Some(error),
            ConfigError::NotAnInstanceName(error) => Some(error),
            ConfigError::NotAbsolute { .. } | ConfigError::ZeroRecoveryDeadline => None,
        }
    }
}

/// Reads a config out of JSON text, or says which part of it is not a config.
/// `control_socket`, `name`, `instance_root` and `recovery_deadline_ms` are
/// required, and `registry_root` is optional. `instance_root` and
/// `registry_root` must be absolute paths, and `recovery_deadline_ms` a whole
/// number of milliseconds, at least 1. An explicit `null` is not an omission:
/// it is refused like any other value of the wrong type. A key the daemon does
/// not know, or one given twice, is refused rather than ignored, so a
/// misspelled setting stops the daemon instead of silently not applying.
/// `name` is parsed into an `InstanceName`, so a path-shaped name never
/// reaches the socket layer. Nothing on disk is read or created.
pub fn parse_config(text: &str) -> Result<DaemonConfig, ConfigError> {
    let written: Written = serde_json::from_str(text).map_err(ConfigError::NotConfig)?;
    let name = InstanceName::parse(&written.name).map_err(ConfigError::NotAnInstanceName)?;
    let instance_root = absolute("instance_root", written.instance_root)?;
    if written.recovery_deadline_ms == 0 {
        return Err(ConfigError::ZeroRecoveryDeadline);
    }
    let registry_root = written
        .registry_root
        .map(|path| absolute("registry_root", path))
        .transpose()?;
    Ok(DaemonConfig {
        control_socket: written.control_socket,
        name,
        instance_root,
        recovery_deadline: Duration::from_millis(written.recovery_deadline_ms),
        registry_root,
    })
}

fn absolute(key: &'static str, path: PathBuf) -> Result<PathBuf, ConfigError> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(ConfigError::NotAbsolute { key, path })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    #[serde(deserialize_with = "control_socket")]
    control_socket: PathBuf,
    #[serde(deserialize_with = "name")]
    name: String,
    #[serde(deserialize_with = "instance_root")]
    instance_root: PathBuf,
    #[serde(deserialize_with = "recovery_deadline_ms")]
    recovery_deadline_ms: u64,
    #[serde(default, deserialize_with = "registry_root")]
    registry_root: Option<PathBuf>,
}

fn unusable<'de, D: Deserializer<'de>, T>(
    key: &str,
    outcome: Result<T, D::Error>,
) -> Result<T, D::Error> {
    outcome.map_err(|error| D::Error::custom(format!("`{key}` is unusable: {error}")))
}

fn control_socket<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
    unusable::<D, _>("control_socket", PathBuf::deserialize(deserializer))
}

fn name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    unusable::<D, _>("name", String::deserialize(deserializer))
}

fn instance_root<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
    unusable::<D, _>("instance_root", PathBuf::deserialize(deserializer))
}

fn recovery_deadline_ms<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    u64::deserialize(deserializer).map_err(|error| {
        D::Error::custom(format!(
            "`recovery_deadline_ms` must be a whole number of milliseconds: {error}"
        ))
    })
}

fn registry_root<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<PathBuf>, D::Error> {
    unusable::<D, _>("registry_root", PathBuf::deserialize(deserializer)).map(Some)
}

/// Why the daemon could not start, or could not keep serving.
#[derive(Debug)]
pub enum DaemonError {
    Bind {
        path: PathBuf,
        source: std::io::Error,
    },
    Listener(std::io::Error),
    SocketIdentity {
        path: PathBuf,
        source: std::io::Error,
    },
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
            DaemonError::SocketIdentity { path, source } => write!(
                f,
                "cannot inspect the control socket {}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for DaemonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DaemonError::Bind { source, .. } => Some(source),
            DaemonError::Listener(source) => Some(source),
            DaemonError::SocketIdentity { source, .. } => Some(source),
        }
    }
}

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

#[derive(Debug)]
struct BoundSocket {
    path: PathBuf,
    dev: u64,
    ino: u64,
}
impl BoundSocket {
    fn capture(path: PathBuf) -> Result<Self, DaemonError> {
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|source| DaemonError::SocketIdentity {
                path: path.clone(),
                source,
            })?;
        if !metadata.file_type().is_socket() {
            return Err(DaemonError::SocketIdentity {
                path,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "the entry at the pathname is not a socket",
                ),
            });
        }
        Ok(Self {
            path,
            dev: metadata.dev(),
            ino: metadata.ino(),
        })
    }
}

impl Drop for BoundSocket {
    fn drop(&mut self) {
        if let Ok(metadata) = std::fs::symlink_metadata(&self.path)
            && metadata.file_type().is_socket()
            && metadata.dev() == self.dev
            && metadata.ino() == self.ino
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Runs the controller daemon until `shutdown` is set, then tears it down.
/// Binds the control socket named by `config` first, so a start that cannot
/// bind has done nothing else. A path already there is refused, whether it
/// holds a live daemon's socket or a stale file; the daemon never unlinks a
/// path it did not create, so clearing a stale path is the caller's job.
/// Right after binding, the daemon records the socket's device and inode; a
/// start that cannot capture that identity refuses without unlinking
/// anything, leaving any residue for the operator.
/// Returning — cleanly, on an error raised after the bind, or through an
/// unwinding panic — removes the socket path only when the entry still at
/// that name is the socket this daemon bound: a no-follow device-and-inode
/// match. A replacement a caller settles at the pathname in the meantime — a
/// regular file or a symlink — is left exactly as it was, and so is the
/// original socket once renamed elsewhere. The check and the unlink are two
/// steps, not one atomic one: a writer with authority over the directory can
/// still substitute a victim between them, so this is ownership-correct
/// cleanup under a coordinated namespace, not race-free protection. A
/// `SIGKILL` runs no destructor, so callers must not read the path's absence
/// as the daemon being gone.
/// Requests on a connection are answered in order, and `shutdown` is honored
/// against a client that stops reading its replies. The instance starts with
/// no cells at ledger generation zero. A handler panic answers `-32603` and
/// then unwinds out of this function.
pub fn run(config: DaemonConfig, shutdown: &AtomicBool) -> Result<(), DaemonError> {
    let listener =
        UnixListener::bind(&config.control_socket).map_err(|source| DaemonError::Bind {
            path: config.control_socket.clone(),
            source,
        })?;
    let _bound = BoundSocket::capture(config.control_socket.clone())?;
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

    fn config(directory: &Path, control_socket: &Path, name: &str) -> DaemonConfig {
        DaemonConfig {
            control_socket: control_socket.to_path_buf(),
            name: InstanceName::parse(name).expect("the test names a valid instance"),
            instance_root: directory.join("instance"),
            recovery_deadline: Duration::from_secs(5),
            registry_root: None,
        }
    }

    const FULL_CONFIG: [(&str, &str); 5] = [
        ("control_socket", r#""/tmp/c.uds""#),
        ("name", r#""work""#),
        ("instance_root", r#""/srv/instances/work""#),
        ("recovery_deadline_ms", "5000"),
        ("registry_root", r#""/srv/imports""#),
    ];

    fn written(pairs: &[(&str, &str)]) -> String {
        let body: Vec<String> = pairs
            .iter()
            .map(|(key, value)| format!("\"{key}\": {value}"))
            .collect();
        format!("{{{}}}", body.join(", "))
    }

    fn replacing(key: &str, value: &str) -> String {
        let pairs: Vec<(&str, &str)> = FULL_CONFIG
            .iter()
            .map(|&(name, old)| (name, if name == key { value } else { old }))
            .collect();
        written(&pairs)
    }

    fn omitting(key: &str) -> String {
        let pairs: Vec<(&str, &str)> = FULL_CONFIG
            .iter()
            .copied()
            .filter(|&(name, _)| name != key)
            .collect();
        written(&pairs)
    }

    fn adding(key: &str, value: &str) -> String {
        let mut pairs = FULL_CONFIG.to_vec();
        pairs.push((key, value));
        written(&pairs)
    }

    fn refused_naming(rows: Vec<(String, &str)>) {
        let failures: Vec<String> = rows
            .iter()
            .filter_map(|(text, offender)| match parse_config(text) {
                Ok(parsed) => Some(format!("{text} parsed as {parsed:?}")),
                Err(refusal) if !refusal.to_string().contains(offender) => Some(format!(
                    "the refusal of {text} does not name {offender}: {refusal}"
                )),
                Err(_) => None,
            })
            .collect();
        assert!(
            failures.is_empty(),
            "every malformed config is refused by name:\n{}",
            failures.join("\n")
        );
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
            let Some(handle) = self.handle.take() else {
                return;
            };
            if self.finished.recv_timeout(PATIENCE).is_ok() {
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

    fn refuses(config: DaemonConfig) -> DaemonError {
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let (done, finished) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(run(config, &flag));
        });
        let outcome = finished.recv_timeout(PATIENCE).expect(
            "run refuses within the deadline rather than serving; a daemon still running here \
             is one that took a path it should have refused, and asserting on the returned \
             error alone would wait for it forever instead of failing",
        );
        shutdown.store(true, Ordering::Relaxed);
        outcome.expect_err("the start is refused")
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
        let parsed = parse_config(&written(&FULL_CONFIG)).expect("a full config parses");
        assert_eq!(
            parsed,
            DaemonConfig {
                control_socket: PathBuf::from("/tmp/c.uds"),
                name: InstanceName::parse("work").expect("`work` is an instance name"),
                instance_root: PathBuf::from("/srv/instances/work"),
                recovery_deadline: Duration::from_millis(5000),
                registry_root: Some(PathBuf::from("/srv/imports")),
            }
        );

        refused_naming(vec![
            ("not json".to_string(), "JSON"),
            (omitting("control_socket"), "`control_socket`"),
            (omitting("name"), "`name`"),
            (omitting("instance_root"), "`instance_root`"),
            (omitting("recovery_deadline_ms"), "`recovery_deadline_ms`"),
            (replacing("control_socket", "7"), "`control_socket`"),
            (replacing("name", "7"), "`name`"),
            (adding("socket", r#""/tmp/x.uds""#), "`socket`"),
            (replacing("name", r#""""#), "empty"),
            (replacing("name", r#""a/b""#), "a/b"),
            (replacing("name", r#""..""#), ".."),
            (replacing("instance_root", r#""cells""#), "`instance_root`"),
            (replacing("instance_root", r#""""#), "`instance_root`"),
            (replacing("instance_root", "7"), "`instance_root`"),
            (replacing("instance_root", "null"), "`instance_root`"),
            (
                replacing("recovery_deadline_ms", "0"),
                "`recovery_deadline_ms`",
            ),
            (
                replacing("recovery_deadline_ms", "-1"),
                "`recovery_deadline_ms`",
            ),
            (
                replacing("recovery_deadline_ms", "1.5"),
                "`recovery_deadline_ms`",
            ),
            (
                replacing("recovery_deadline_ms", "1e3"),
                "`recovery_deadline_ms`",
            ),
            (
                replacing("recovery_deadline_ms", r#""5000""#),
                "`recovery_deadline_ms`",
            ),
            (
                replacing("recovery_deadline_ms", "null"),
                "`recovery_deadline_ms`",
            ),
            (
                replacing("registry_root", r#""imports""#),
                "`registry_root`",
            ),
            (replacing("registry_root", r#""""#), "`registry_root`"),
            (replacing("registry_root", "7"), "`registry_root`"),
            (replacing("registry_root", "null"), "`registry_root`"),
            (adding("registry", r#""/srv/imports""#), "`registry`"),
            (
                adding("instance_root", r#""/srv/other""#),
                "`instance_root`",
            ),
        ]);
    }

    #[test]
    fn registry_root_is_optional_and_omission_means_local_operation() {
        let local = parse_config(&omitting("registry_root"))
            .expect("a config without registry_root parses");
        assert_eq!(
            local.registry_root, None,
            "no registry root is supplied by HOME, the instance root or any other path"
        );

        let imports = parse_config(&written(&FULL_CONFIG)).expect("a full config parses");
        assert_eq!(
            imports.registry_root,
            Some(PathBuf::from("/srv/imports")),
            "a configured registry root is carried exactly as written"
        );
    }

    #[test]
    fn instance_root_is_never_derived_from_the_control_socket() {
        let here = parse_config(&replacing("control_socket", r#""/a/control.uds""#))
            .expect("the config parses");
        let there = parse_config(&replacing("control_socket", r#""/b/elsewhere.uds""#))
            .expect("the config parses");
        assert_eq!(here.instance_root, PathBuf::from("/srv/instances/work"));
        assert_eq!(
            there.instance_root, here.instance_root,
            "moving the control socket does not move the instance root"
        );

        let laid_out =
            omitting("instance_root").replace("/tmp/c.uds", "/x/instances/work/control.uds");
        let refusal = parse_config(&laid_out).expect_err(
            "a socket inside the usual instance layout does not stand in for a missing root",
        );
        assert!(
            refusal.to_string().contains("`instance_root`"),
            "the refusal names the missing key: {refusal}"
        );
    }

    #[test]
    fn the_daemon_answers_status_with_an_empty_cell_registry() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");
        let control = directory.path().join("control.uds");
        let daemon = start(config(directory.path(), &control, "work"));

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
        let _daemon = start(config(directory.path(), &control, "work"));

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

        let refusal = refuses(config(directory.path(), &control, "work"));

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
        let daemon = start(config(directory.path(), &control, "work"));

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
        let daemon = start(config(directory.path(), &control, "work"));

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
        let _daemon = start(config(directory.path(), &control, "work"));

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

    #[test]
    fn shutdown_leaves_a_later_socket_settled_at_the_pathname_alone() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");
        let control = directory.path().join("control.uds");
        let daemon = start(config(directory.path(), &control, "work"));

        let mut client = addressable(&control);
        assert_eq!(
            ask(&mut client, STATUS_REQUEST).pointer("/result/name"),
            Some(&json!("work")),
            "the daemon is serving before its pathname is exchanged"
        );
        drop(client);

        let original = std::fs::symlink_metadata(&control).expect("the bound socket is on disk");
        let saved = directory.path().join("saved.uds");
        std::fs::rename(&control, &saved).expect("the original socket is renamed");

        let replacement =
            UnixListener::bind(&control).expect("the test binds its own socket at the pathname");
        let settled = std::fs::symlink_metadata(&control).expect("the settled socket is on disk");
        assert_ne!(
            (settled.dev(), settled.ino()),
            (original.dev(), original.ino()),
            "the second bind is a different socket while the original stays allocated"
        );
        thread::spawn(move || {
            while let Ok((mut stream, _)) = replacement.accept() {
                let mut line = String::new();
                let _ = BufReader::new(&mut stream).read_line(&mut line);
                let _ = writeln!(stream, r#"{{"id":7,"result":{{"fixture":true}}}}"#);
                let _ = stream.flush();
            }
        });

        let mut bystander = addressable(&control);
        assert_eq!(
            ask(&mut bystander, STATUS_REQUEST).pointer("/result/fixture"),
            Some(&json!(true)),
            "the settled socket answers before the daemon shuts down"
        );
        drop(bystander);

        daemon.stop().expect("the daemon shuts down cleanly");

        let survivor =
            std::fs::symlink_metadata(&control).expect("the settled socket outlives shutdown");
        assert!(
            survivor.file_type().is_socket(),
            "teardown removes the socket the daemon bound, not a socket another owner later \
             settled at the pathname"
        );
        assert_eq!(
            (survivor.dev(), survivor.ino()),
            (settled.dev(), settled.ino()),
            "the survivor is the fixture's own socket entry"
        );
        let renamed = std::fs::symlink_metadata(&saved).expect("the renamed original is retained");
        assert!(
            renamed.file_type().is_socket()
                && (renamed.dev(), renamed.ino()) == (original.dev(), original.ino()),
            "the original socket stays allocated under the caller's new name"
        );
        let mut after = addressable(&control);
        assert_eq!(
            ask(&mut after, STATUS_REQUEST).pointer("/result/fixture"),
            Some(&json!(true)),
            "the settled socket still answers once the daemon is gone"
        );
    }

    #[test]
    fn an_unwinding_guard_removes_its_own_socket_and_leaves_a_settled_replacement_alone() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");

        let control = directory.path().join("control.uds");
        let bound = UnixListener::bind(&control).expect("the fixture binds a real socket");
        let guard = BoundSocket::capture(control.clone()).expect("the guard captures its socket");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = guard;
            let _still_bound = bound;
            panic!("the daemon unwinds through the guard");
        }));
        assert!(
            !control.exists(),
            "an unchanged socket the guard owns is removed even on an unwinding exit"
        );

        let control = directory.path().join("control.uds");
        let bound = UnixListener::bind(&control).expect("the fixture binds a real socket");
        let guard = BoundSocket::capture(control.clone()).expect("the guard captures its socket");
        let saved = directory.path().join("saved.uds");
        std::fs::rename(&control, &saved).expect("the original socket is renamed");
        std::fs::write(&control, b"a settled replacement").expect("the sentinel is written");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = guard;
            let _still_bound = bound;
            panic!("the daemon unwinds through the guard");
        }));
        let left = std::fs::symlink_metadata(&control).expect("the sentinel survives the unwind");
        assert!(
            left.file_type().is_file(),
            "a replacement settled after capture is not the guard's socket, so an unwinding \
             teardown leaves it standing"
        );
        assert!(
            std::fs::symlink_metadata(&saved)
                .expect("the renamed original is retained")
                .file_type()
                .is_socket(),
            "the renamed original socket stays allocated at its new name"
        );
    }

    #[test]
    fn capture_refuses_a_non_socket_or_missing_entry_and_leaves_it_alone() {
        use std::error::Error as _;

        let directory = tempfile::tempdir().expect("the test owns a temporary directory");

        let control = directory.path().join("control.uds");
        std::fs::write(&control, b"someone else's socket").expect("the test occupies the path");
        let refusal =
            BoundSocket::capture(control.clone()).expect_err("a regular file is not a socket");
        match &refusal {
            DaemonError::SocketIdentity { path, .. } => assert_eq!(path, &control),
            other => panic!("a non-socket entry is an identity failure, got {other:?}"),
        }
        let source = refusal
            .source()
            .and_then(|source| source.downcast_ref::<std::io::Error>())
            .expect("the identity refusal carries the actual I/O error");
        assert_eq!(
            source.kind(),
            std::io::ErrorKind::InvalidData,
            "a non-socket entry is invalid data, not a missing one: {source}"
        );
        assert_eq!(
            std::fs::read(&control)
                .expect("the occupied file is readable")
                .as_slice(),
            b"someone else's socket".as_slice(),
            "capture inspects the namespace, it never clears it: the entry is left for its owner"
        );

        let link = directory.path().join("link.uds");
        std::os::unix::fs::symlink("nowhere.uds", &link)
            .expect("the test plants a dangling leaf symlink");
        let refusal = BoundSocket::capture(link.clone())
            .expect_err("a leaf symlink is not a socket, even dangling");
        assert!(
            matches!(&refusal, DaemonError::SocketIdentity { path, .. } if path == &link),
            "the refusal names the symlink, not its target: {refusal:?}"
        );
        assert_eq!(
            std::fs::read_link(&link).expect("the symlink target is readable"),
            PathBuf::from("nowhere.uds"),
            "the no-follow inspection preserves the symlink and its exact target"
        );

        let missing = directory.path().join("absent.uds");
        let refusal = BoundSocket::capture(missing.clone())
            .expect_err("a missing path has no identity to capture");
        let source = refusal
            .source()
            .and_then(|source| source.downcast_ref::<std::io::Error>())
            .expect("the identity refusal carries the actual I/O error");
        assert_eq!(
            source.kind(),
            std::io::ErrorKind::NotFound,
            "a missing path is a not-found inspection error: {source}"
        );
        assert!(
            !missing.exists(),
            "a failed capture creates nothing and removes nothing"
        );
    }
}
