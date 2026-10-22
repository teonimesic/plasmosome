use std::io::{BufReader, ErrorKind, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::de::value::MapAccessDeserializer;
use serde::de::{Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use serde_json::value::RawValue;

mod cell_runtime;

pub use cell_runtime::{CellRuntime, CellRuntimeError};

use crate::control::{Controller, Handler, serve_connection};
use crate::private_socket::{
    Accepted, PrivateDir, PrivateListener, PrivateSocketError, effective_uid,
};
use crate::runtime::recipe::{HostLimitsError, PlatformLimits};
use crate::runtime::strict_json;
use crate::state::{ControllerState, InstanceName, InstanceNameError};

const ACCEPT_POLL: Duration = Duration::from_millis(25);
const READ_TIMEOUT: Duration = Duration::from_millis(50);
const WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const NO_LEDGER_YET: u64 = 0;

/// Everything `plasmosomed` needs to start: the path it answers on, which
/// named instance it is, where that instance's durable state lives, how long
/// startup recovery may take, where trusted registry imports live, if
/// anywhere, and how to start a cell's supervisor, if it may. Only
/// `parse_config` checks the rules below; a value built by hand carries no
/// such promise.
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
    /// Spec 023 §1's `cell_runtime`. `None` means `cell.new` refuses; startup,
    /// recovery, `cell.kill` and every observation still work.
    pub cell_runtime: Option<CellRuntime>,
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
    CellRuntime(CellRuntimeError),
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
            ConfigError::CellRuntime(error) => write!(f, "`cell_runtime` is refused: {error}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::NotConfig(error) => Some(error),
            ConfigError::NotAnInstanceName(error) => Some(error),
            ConfigError::CellRuntime(error) => Some(error),
            ConfigError::NotAbsolute { .. } | ConfigError::ZeroRecoveryDeadline => None,
        }
    }
}

/// Reads a config out of JSON text, or says which part of it is not a config.
/// `control_socket`, `name`, `instance_root` and `recovery_deadline_ms` are
/// required, and `registry_root` and `cell_runtime` are optional.
/// `instance_root` and `registry_root` must be absolute paths, and
/// `recovery_deadline_ms` a whole number of milliseconds, at least 1. An
/// explicit `null` is not an omission: it is refused like any other value of
/// the wrong type. A key the daemon does not know, or one given twice, is
/// refused rather than ignored, so a misspelled setting stops the daemon
/// instead of silently not applying; inside `cell_runtime` a key given twice
/// is refused at any depth. `name` is parsed into an `InstanceName`, so a
/// path-shaped name never reaches the socket layer. The text must be one JSON
/// object: an array is refused rather than read by position.
///
/// A `cell_runtime` is checked as spec 023 §1 says, against this host's
/// limits, which are read only when the key is present. With it,
/// `instance_root` must also be a canonical path, so that the longest cell
/// ID's three socket paths can be measured, the template completed with that
/// cell's paths, and the largest config record encoded. A `cell_runtime` that
/// passes cannot make any later `cell.new` refuse its recipe or its record,
/// while the host reports the same limits.
/// Nothing on disk is read or created, the Artifacts included: `cell.new`
/// verifies those with [`crate::artifact::verify_artifact`].
pub fn parse_config(text: &str) -> Result<DaemonConfig, ConfigError> {
    read_config(text, PlatformLimits::host)
}

fn read_config(
    text: &str,
    limits: impl FnOnce() -> Result<PlatformLimits, HostLimitsError>,
) -> Result<DaemonConfig, ConfigError> {
    let written = one_object(text).map_err(ConfigError::NotConfig)?;
    let name = InstanceName::parse(&written.name).map_err(ConfigError::NotAnInstanceName)?;
    let instance_root = absolute("instance_root", written.instance_root)?;
    if written.recovery_deadline_ms == 0 {
        return Err(ConfigError::ZeroRecoveryDeadline);
    }
    let registry_root = written
        .registry_root
        .map(|path| absolute("registry_root", path))
        .transpose()?;
    let cell_runtime = written
        .cell_runtime
        .map(|value| cell_runtime::read(&value, &instance_root, limits))
        .transpose()
        .map_err(ConfigError::CellRuntime)?;
    Ok(DaemonConfig {
        control_socket: written.control_socket,
        name,
        instance_root,
        recovery_deadline: Duration::from_millis(written.recovery_deadline_ms),
        registry_root,
        cell_runtime,
    })
}

fn absolute(key: &'static str, path: PathBuf) -> Result<PathBuf, ConfigError> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(ConfigError::NotAbsolute { key, path })
    }
}

fn one_object(text: &str) -> Result<Written, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let written = (&mut deserializer).deserialize_map(ObjectOnly)?;
    deserializer.end()?;
    Ok(written)
}

struct ObjectOnly;

impl<'de> Visitor<'de> for ObjectOnly {
    type Value = Written;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Written, A::Error> {
        Written::deserialize(MapAccessDeserializer::new(map))
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
    #[serde(default, deserialize_with = "cell_runtime")]
    cell_runtime: Option<Value>,
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

fn cell_runtime<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    let raw = unusable::<D, _>("cell_runtime", Box::<RawValue>::deserialize(deserializer))?;
    strict_json::parse_value(raw.get().as_bytes())
        .map(Some)
        .map_err(|error| D::Error::custom(format!("`cell_runtime` is unusable: {error}")))
}

/// Why the daemon could not start, or could not keep serving. `Boundary` is the private-socket
/// boundary's refusal of the control socket path, or a failed bind under it: the daemon served
/// nothing.
#[derive(Debug)]
pub enum DaemonError {
    Boundary(PrivateSocketError),
    Listener(std::io::Error),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::Boundary(refusal) => write!(f, "{refusal}"),
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
            DaemonError::Boundary(refusal) => Some(refusal),
            DaemonError::Listener(source) => Some(source),
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

fn split_socket_path(path: &Path) -> Result<(&Path, &str), PrivateSocketError> {
    let normal = path
        .as_os_str()
        .as_bytes()
        .strip_prefix(b"/")
        .is_some_and(|relative| {
            relative
                .split(|byte| *byte == b'/')
                .all(|component| !matches!(component, b"" | b"." | b".."))
        });
    let (true, Some(parent), Some(name)) = (normal, path.parent(), path.file_name()) else {
        return Err(PrivateSocketError::BadPath {
            path: path.to_path_buf(),
        });
    };
    let name = name.to_str().ok_or_else(|| PrivateSocketError::BadName {
        name: name.to_string_lossy().into_owned(),
    })?;
    Ok((parent, name))
}

/// Runs the controller daemon until `shutdown` is set, then tears it down.
/// Binds the control socket named by `config` first, so a start that cannot
/// bind has done nothing else. The path is held to spec 001 §4.1's boundary
/// for private sockets: it must be absolute with no empty, `.` or `..`
/// component and pass through no symlink. Its ancestors must be owned by root
/// or the effective UID, and neither group, other nor an ACL entry may let
/// another user replace them. Its directory must be owned by the effective
/// UID, with no group or other permission and no ACL. A path that fails is
/// refused as `DaemonError::Boundary`, naming the failing component, and
/// nothing is bound. A path already there is refused too, whether it holds a live
/// daemon's socket or a stale file; the daemon never unlinks a path it did not
/// create, so clearing a stale path is the caller's job. The socket is set to
/// mode 0600 before it listens, whatever the umask. A connection whose kernel
/// peer UID is not the effective UID is closed before a byte is read or
/// written, and the daemon goes on to the next one.
/// Returning — cleanly, on an error raised after the bind, or through an
/// unwinding panic — removes the socket only while the entry at that name in
/// the directory opened at start is still the socket this daemon bound, by
/// device and inode, without following a symlink. A replacement a caller
/// settles at the name in the meantime — a regular file or a symlink — is
/// left exactly as it was, and so is the original socket once renamed
/// elsewhere. The check and the unlink are two steps, not one atomic one: a
/// writer with authority over the directory can still substitute a victim
/// between them, so this is ownership-correct cleanup under a coordinated
/// namespace, not race-free protection. A `SIGKILL` runs no destructor, so
/// callers must not read the path's absence as the daemon being gone.
/// Requests on a connection are answered in order, and `shutdown` is honored
/// against a client that stops reading its replies. The instance starts with
/// no cells at ledger generation zero. A handler panic answers `-32603` and
/// then unwinds out of this function.
pub fn run(config: DaemonConfig, shutdown: &AtomicBool) -> Result<(), DaemonError> {
    let (parent, name) =
        split_socket_path(&config.control_socket).map_err(DaemonError::Boundary)?;
    let directory = PrivateDir::open(parent).map_err(DaemonError::Boundary)?;
    let listener =
        PrivateListener::bind(directory, name, effective_uid()).map_err(DaemonError::Boundary)?;

    let mut controller = Controller::new(config.name, ControllerState::default(), NO_LEDGER_YET);
    serve(listener, shutdown, &mut controller)
}

fn serve(
    listener: PrivateListener,
    shutdown: &AtomicBool,
    handler: &mut impl Handler,
) -> Result<(), DaemonError> {
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok(Some(Accepted::Trusted(stream))) => converse(stream, shutdown, handler),
            Ok(Some(Accepted::Refused(_))) => {}
            Ok(None) => std::thread::sleep(ACCEPT_POLL),
            Err(error) => match accept_outcome(&error) {
                Next::Poll => std::thread::sleep(ACCEPT_POLL),
                Next::Retry => {}
                Next::Fail => return Err(DaemonError::Listener(error)),
            },
        }
    }
    Ok(())
}

fn converse(stream: UnixStream, shutdown: &AtomicBool, handler: &mut impl Handler) {
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
    let _ = serve_connection(reader, writer, handler);
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
    use crate::control::Handler;
    use crate::nofollow::fixtures::root_bases;
    use crate::private_socket::{PrivateDir, PrivateListener, PrivateSocketError, effective_uid};
    use crate::protocol::WireError;
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, ErrorKind, Write};
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    const STATUS_REQUEST: &str = r#"{"id":1,"method":"plasmosome.status","params":{}}"#;
    const PATIENCE: Duration = Duration::from_secs(10);

    #[cfg(target_os = "macos")]
    const SUN_PATH_MAX: usize = 103;
    #[cfg(target_os = "linux")]
    const SUN_PATH_MAX: usize = 107;

    const NAME_ROOM: usize = 16;

    fn private_root() -> tempfile::TempDir {
        let mut refusals = Vec::new();
        for base in root_bases() {
            let root = match tempfile::Builder::new().prefix("pd").tempdir_in(&base) {
                Ok(root) => root,
                Err(error) => {
                    refusals.push(format!("{}: {error}", base.display()));
                    continue;
                }
            };
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
                .expect("the test root is made private");
            let length = root.path().as_os_str().len();
            match PrivateDir::open(root.path()) {
                Err(refusal) => refusals.push(format!("{}: {refusal}", base.display())),
                Ok(_) if length + 1 + NAME_ROOM > SUN_PATH_MAX => refusals.push(format!(
                    "{}: a {length}-byte root leaves fewer than {NAME_ROOM} bytes for a socket name",
                    base.display()
                )),
                Ok(_) => return root,
            }
        }
        panic!(
            "no base can hold a private test root; on macOS the base is the canonical per-user \
             temp directory, on Linux XDG_RUNTIME_DIR, else the test binary's directory:\n{}",
            refusals.join("\n")
        );
    }

    fn config(directory: &Path, control_socket: &Path, name: &str) -> DaemonConfig {
        DaemonConfig {
            control_socket: control_socket.to_path_buf(),
            name: InstanceName::parse(name).expect("the test names a valid instance"),
            instance_root: directory.join("instance"),
            recovery_deadline: Duration::from_secs(5),
            registry_root: None,
            cell_runtime: None,
        }
    }

    const FULL_CONFIG: [(&str, &str); 5] = [
        ("control_socket", r#""/tmp/c.uds""#),
        ("name", r#""work""#),
        ("instance_root", r#""/srv/roots/r7""#),
        ("recovery_deadline_ms", "4321"),
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

    fn spawned(
        work: impl FnOnce(&AtomicBool) -> Result<(), DaemonError> + Send + 'static,
    ) -> Running {
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let (done, finished) = mpsc::channel();
        let handle = thread::spawn(move || {
            let outcome = work(&flag);
            let _ = done.send(());
            outcome
        });
        Running {
            shutdown,
            finished,
            handle: Some(handle),
        }
    }

    fn start(config: DaemonConfig) -> Running {
        spawned(move |flag| run(config, flag))
    }

    fn serving(listener: PrivateListener, name: &str) -> Running {
        let name = InstanceName::parse(name).expect("the test names a valid instance");
        spawned(move |flag| {
            let mut controller = Controller::new(name, ControllerState::default(), NO_LEDGER_YET);
            serve(listener, flag, &mut controller)
        })
    }

    struct Panicking;

    impl Handler for Panicking {
        fn handle(
            &mut self,
            _method: &str,
            _params: &serde_json::Map<String, Value>,
        ) -> Result<Value, WireError> {
            panic!("the handler fails while answering");
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
                "the daemon is addressable within five seconds"
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
                instance_root: PathBuf::from("/srv/roots/r7"),
                recovery_deadline: Duration::from_millis(4321),
                registry_root: Some(PathBuf::from("/srv/imports")),
                cell_runtime: None,
            }
        );
        assert_eq!(
            parse_config(&format!("{}\n", written(&FULL_CONFIG)))
                .expect("a config file's final newline is whitespace after the object"),
            parsed
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
            (adding("cell_runtime", "null"), "`cell_runtime`"),
            (adding("cell_runtime", "{}"), "`cell_runtime"),
            (
                adding("instance_root", r#""/srv/other""#),
                "`instance_root`",
            ),
            (
                r#"["/tmp/c.uds", "work", "/srv/imports", 4321, "/srv/roots/r7"]"#.to_string(),
                "sequence, expected a JSON object",
            ),
            ("[]".to_string(), "sequence, expected a JSON object"),
            (
                r#""work""#.to_string(),
                r#"string "work", expected a JSON object"#,
            ),
            ("7".to_string(), "integer `7`, expected a JSON object"),
            ("null".to_string(), "null, expected a JSON object"),
            (
                format!("{} {{}}", written(&FULL_CONFIG)),
                "trailing characters",
            ),
        ]);
    }

    #[test]
    fn a_shape_refusal_names_its_key_and_path_and_has_no_underlying_cause() {
        use std::error::Error as _;

        for (key, text) in [
            ("instance_root", replacing("instance_root", r#""cells""#)),
            ("registry_root", replacing("registry_root", r#""cells""#)),
        ] {
            let refusal = parse_config(&text).expect_err("a relative path is refused");
            assert!(
                matches!(
                    &refusal,
                    ConfigError::NotAbsolute { key: named, path }
                        if *named == key && path == Path::new("cells")
                ),
                "the refusal carries the key and the path as written: {refusal:?}"
            );
            assert_eq!(
                refusal.to_string(),
                format!("`{key}` must be an absolute path, got \"cells\""),
                "the operator reads the key and the path as written"
            );
            assert!(refusal.source().is_none(), "{refusal:?}");
        }

        let zero = parse_config(&replacing("recovery_deadline_ms", "0"))
            .expect_err("a zero deadline is refused");
        assert!(
            matches!(zero, ConfigError::ZeroRecoveryDeadline),
            "{zero:?}"
        );
        assert!(zero.source().is_none(), "{zero:?}");

        let not_json = parse_config("not json").expect_err("text that is not JSON is refused");
        assert!(
            not_json
                .source()
                .and_then(|source| source.downcast_ref::<serde_json::Error>())
                .is_some(),
            "a JSON refusal keeps the parser's error: {not_json:?}"
        );
        let bad_name = parse_config(&replacing("name", r#""a/b""#))
            .expect_err("a path-shaped name is refused");
        assert!(
            bad_name
                .source()
                .and_then(|source| source.downcast_ref::<InstanceNameError>())
                .is_some(),
            "a name refusal keeps the name's error: {bad_name:?}"
        );
    }

    #[test]
    fn the_deadline_admits_every_positive_whole_number_of_milliseconds() {
        for (written_ms, deadline) in [
            ("1", Duration::from_millis(1)),
            ("18446744073709551615", Duration::from_millis(u64::MAX)),
        ] {
            let parsed = parse_config(&replacing("recovery_deadline_ms", written_ms))
                .unwrap_or_else(|error| panic!("{written_ms} ms is a valid deadline: {error}"));
            assert_eq!(
                parsed.recovery_deadline, deadline,
                "{written_ms} ms is carried exactly, with no floor above 1 and no cap"
            );
        }
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
        assert_eq!(here.instance_root, PathBuf::from("/srv/roots/r7"));
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
    fn the_daemon_serves_without_creating_its_instance_root() {
        let directory = private_root();
        let control = directory.path().join("control.uds");
        let configured = config(directory.path(), &control, "work");
        let root = configured.instance_root.clone();
        assert!(!root.exists(), "the configured root starts missing");
        let daemon = start(configured);

        let mut client = addressable(&control);
        assert_eq!(
            ask(&mut client, STATUS_REQUEST).pointer("/result/ready"),
            Some(&json!(true)),
            "this build serves status whether or not its instance root exists"
        );
        drop(client);
        daemon.stop().expect("a clean shutdown is not an error");
        assert!(
            !root.exists(),
            "the daemon never creates its instance root: the operator supplies it"
        );
    }

    #[test]
    fn the_daemon_answers_status_with_an_empty_cell_registry() {
        let directory = private_root();
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
        let directory = private_root();
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
        let directory = private_root();
        let control = directory.path().join("control.uds");
        std::fs::write(&control, b"someone else's socket").expect("the test occupies the path");

        let refusal = refuses(config(directory.path(), &control, "work"));

        match &refusal {
            DaemonError::Boundary(PrivateSocketError::AddressInUse { path }) => {
                assert_eq!(path, &control)
            }
            other => panic!("an occupied path is refused as in use, got {other:?}"),
        }
        assert!(
            control.exists(),
            "the daemon never unlinks a path it did not bind: a live daemon's socket removed \
             out from under it is how a half-alive controller is manufactured"
        );
    }

    #[test]
    fn shutdown_stops_serve_even_with_an_idle_connection_open() {
        let directory = private_root();
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
        let directory = private_root();
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
        let directory = private_root();
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
        let directory = private_root();
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
    fn another_uid_gets_no_reply_and_the_loop_goes_on() {
        let root = private_root();
        let euid = effective_uid();
        let foreign = PrivateListener::bind(
            PrivateDir::open(root.path()).expect("the test root is private"),
            "foreign.uds",
            euid.wrapping_add(1),
        )
        .expect("a listener trusting another UID binds");
        let clients: Vec<UnixStream> = (0..2)
            .map(|_| {
                let mut client = UnixStream::connect(&foreign.entry().path)
                    .expect("the listener queues the connection");
                client
                    .set_read_timeout(Some(PATIENCE))
                    .expect("the test client bounds its own reads");
                writeln!(client, "{STATUS_REQUEST}")
                    .expect("the request is queued before the daemon accepts");
                client
            })
            .collect();
        let daemon = serving(foreign, "work");

        for (attempt, mut client) in ["first", "second"].into_iter().zip(clients) {
            let mut reply = Vec::new();
            let end = client.read_to_end(&mut reply);
            assert!(
                reply.is_empty(),
                "the {attempt} connection from an untrusted UID got a reply: {:?}",
                String::from_utf8_lossy(&reply)
            );
            #[cfg(target_os = "macos")]
            assert_eq!(
                end.expect("macOS ends a refused connection with end of file"),
                0,
                "the {attempt} untrusted connection is closed"
            );
            #[cfg(target_os = "linux")]
            assert_eq!(
                end.expect_err("the daemon closed the connection with its request still queued")
                    .kind(),
                ErrorKind::ConnectionReset,
                "the {attempt} untrusted connection is reset: its request was still queued when \
                 the daemon closed it"
            );
        }
        assert!(
            daemon
                .finished
                .recv_timeout(Duration::from_millis(200))
                .is_err(),
            "the loop is still serving after it refused two peers"
        );
        daemon
            .stop()
            .expect("refusing a peer is not a listener failure: the loop keeps serving");

        let own = PrivateListener::bind(
            PrivateDir::open(root.path()).expect("the test root is private"),
            "own.uds",
            euid,
        )
        .expect("a listener trusting this UID binds");
        let path = own.entry().path.clone();
        let _daemon = serving(own, "work");
        let mut client = addressable(&path);
        assert_eq!(
            ask(&mut client, STATUS_REQUEST).pointer("/result/name"),
            Some(&json!("work")),
            "a loop serving this UID answers it"
        );
    }

    #[test]
    fn an_unwinding_panic_removes_the_socket_and_leaves_a_settled_replacement_alone() {
        let root = private_root();
        let own = |name: &str| {
            PrivateListener::bind(
                PrivateDir::open(root.path()).expect("the test root is private"),
                name,
                effective_uid(),
            )
            .expect("a listener trusting this UID binds")
        };
        let unwinding =
            |listener: PrivateListener| spawned(move |flag| serve(listener, flag, &mut Panicking));
        let unwound = |mut serving: Running| {
            assert_eq!(
                serving.finished.recv_timeout(PATIENCE),
                Err(mpsc::RecvTimeoutError::Disconnected),
                "the panic unwinds out of the accept loop within ten seconds"
            );
            assert!(
                serving
                    .handle
                    .take()
                    .expect("the loop thread is joined once")
                    .join()
                    .is_err(),
                "the loop thread ends in the handler's panic"
            );
        };

        let listener = own("control.uds");
        let control = listener.entry().path.clone();
        let serving = unwinding(listener);
        let mut client = addressable(&control);
        assert_eq!(
            ask(&mut client, STATUS_REQUEST)
                .pointer("/error/code")
                .and_then(Value::as_i64),
            Some(-32603),
            "a handler panic is answered before it unwinds"
        );
        unwound(serving);
        assert!(
            std::fs::symlink_metadata(&control).is_err(),
            "the socket is removed on the unwinding route out"
        );

        let listener = own("settled.uds");
        let settled = listener.entry().path.clone();
        let saved = root.path().join("saved.uds");
        let serving = unwinding(listener);
        let mut client = addressable(&settled);
        std::fs::rename(&settled, &saved).expect("the socket is renamed away");
        std::fs::write(&settled, b"a settled replacement").expect("the sentinel is written");
        let _ = ask(&mut client, STATUS_REQUEST);
        unwound(serving);
        assert_eq!(
            std::fs::read(&settled).expect("the replacement survives the unwind"),
            b"a settled replacement",
            "an unwinding teardown leaves a replacement at the name alone"
        );
        assert!(
            std::fs::symlink_metadata(&saved)
                .expect("the renamed original survives")
                .file_type()
                .is_socket(),
            "the renamed original stays at its new name"
        );
    }

    #[test]
    fn a_control_socket_path_that_is_not_absolute_and_normal_is_refused_as_written() {
        for written in [
            "",
            "control.uds",
            "relative/control.uds",
            "/",
            "/a/b/",
            "/a//b",
            "/a/./b",
            "/a/../b",
            "/a/b/.",
            "/a/b/..",
        ] {
            assert_eq!(
                split_socket_path(Path::new(written)),
                Err(PrivateSocketError::BadPath {
                    path: PathBuf::from(written)
                }),
                "{written:?} is refused before any directory is opened"
            );
        }
        assert_eq!(
            split_socket_path(Path::new("/a/b/control.uds")),
            Ok((Path::new("/a/b"), "control.uds"))
        );
        assert_eq!(
            split_socket_path(Path::new("/control.uds")),
            Ok((Path::new("/"), "control.uds"))
        );

        use std::os::unix::ffi::OsStrExt;
        let raw = Path::new(std::ffi::OsStr::from_bytes(b"/a/\xffcontrol.uds"));
        assert_eq!(
            split_socket_path(raw),
            Err(PrivateSocketError::BadName {
                name: "\u{fffd}control.uds".to_string()
            }),
            "a socket name that is not UTF-8 cannot be bound by name"
        );
    }

    #[test]
    fn a_boundary_refusal_reads_as_the_refusal_itself() {
        use std::error::Error as _;

        let refusal = PrivateSocketError::SymlinkInPath {
            path: PathBuf::from("/var"),
        };
        let error = DaemonError::Boundary(refusal.clone());
        assert_eq!(
            error.to_string(),
            refusal.to_string(),
            "the operator reads the failing component and the rule it broke"
        );
        assert_eq!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<PrivateSocketError>()),
            Some(&refusal),
            "the refusal is kept as the cause"
        );
    }

    #[test]
    fn a_listener_failure_reads_as_one_and_keeps_its_cause() {
        use std::error::Error as _;

        let error = DaemonError::Listener(std::io::Error::from_raw_os_error(libc::EMFILE));
        assert!(
            error
                .to_string()
                .starts_with("the control socket stopped accepting connections: "),
            "{error}"
        );
        assert_eq!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<std::io::Error>())
                .and_then(std::io::Error::raw_os_error),
            Some(libc::EMFILE),
            "the accept failure is kept as the cause"
        );
    }
}
