use crate::brokers::{BrokerSet, BrokerSpec, SpawnFailed};
use crate::control;
use crate::control::ListenerFailed;
use crate::exec::{ExecCommand, ExecError};
use crate::readiness::ControlSocketProbe;
use crate::vmm::{SpawnError, VmmChild};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const DEFAULT_STATUS_DEADLINE: Duration = Duration::from_millis(500);

/// One broker the daemon spawns and then asks for readiness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerConfig {
    pub name: String,
    pub control_socket: PathBuf,
    pub command: Vec<String>,
}

/// Everything the daemon needs to start: where it answers, how long one
/// readiness call may take, and the brokers it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub control_socket: PathBuf,
    pub status_deadline: Duration,
    pub brokers: Vec<BrokerConfig>,
}

/// Why a config text is not a config.
#[derive(Debug)]
pub enum ConfigError {
    NotJson(serde_json::Error),
    MissingKey { key: String },
    UnknownKey { key: String },
    WrongType { key: String, wanted: &'static str },
    EmptyValue { key: String },
    ZeroDeadline,
    DuplicateBrokerName { name: String },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotJson(error) => write!(f, "config is not JSON: {error}"),
            ConfigError::MissingKey { key } => write!(f, "config is missing `{key}`"),
            ConfigError::UnknownKey { key } => write!(f, "config carries an unknown key `{key}`"),
            ConfigError::WrongType { key, wanted } => write!(f, "`{key}` must be {wanted}"),
            ConfigError::EmptyValue { key } => write!(f, "`{key}` must not be empty"),
            ConfigError::ZeroDeadline => {
                write!(f, "`status_deadline_ms` must be greater than zero")
            }
            ConfigError::DuplicateBrokerName { name } => {
                write!(f, "two brokers are named `{name}`")
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::NotJson(error) => Some(error),
            _ => None,
        }
    }
}

/// Why the daemon could not start.
#[derive(Debug)]
pub enum DaemonError {
    Bind {
        path: PathBuf,
        source: std::io::Error,
    },
    BrokerCommand {
        broker: String,
        source: ExecError,
    },
    Spawn(SpawnFailed),
    Listener(ListenerFailed),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::Bind { path, source } => {
                write!(
                    f,
                    "cannot bind the control socket {}: {source}",
                    path.display()
                )
            }
            DaemonError::BrokerCommand { broker, source } => {
                write!(f, "broker `{broker}` has no runnable command: {source}")
            }
            DaemonError::Spawn(failure) => write!(f, "{failure}"),
            DaemonError::Listener(ListenerFailed(source)) => {
                write!(
                    f,
                    "the control socket stopped accepting connections: {source}"
                )
            }
        }
    }
}

impl std::error::Error for DaemonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DaemonError::Bind { source, .. } => Some(source),
            DaemonError::BrokerCommand { source, .. } => Some(source),
            DaemonError::Spawn(failure) => Some(failure),
            DaemonError::Listener(ListenerFailed(source)) => Some(source),
        }
    }
}

/// Reads a config out of JSON text, or says which part of it is not a config.
///
/// `control_socket` is required. `status_deadline_ms` defaults to 500 and may
/// not be zero. `brokers` defaults to none. Keys the daemon does not know are
/// refused rather than ignored, at the top level and inside each broker, so a
/// misspelled setting stops the daemon instead of silently not applying.
pub fn parse_config(text: &str) -> Result<DaemonConfig, ConfigError> {
    let value: Value = serde_json::from_str(text).map_err(ConfigError::NotJson)?;
    let fields = object_at(&value, "config")?;
    refuse_unknown(
        fields,
        &["control_socket", "status_deadline_ms", "brokers"],
        "",
    )?;
    let control_socket = PathBuf::from(required_string(fields, "control_socket", "")?);
    let status_deadline = match fields.get("status_deadline_ms") {
        None => DEFAULT_STATUS_DEADLINE,
        Some(value) => {
            let millis = value.as_u64().ok_or(ConfigError::WrongType {
                key: "status_deadline_ms".to_string(),
                wanted: "a whole number of milliseconds",
            })?;
            if millis == 0 {
                return Err(ConfigError::ZeroDeadline);
            }
            Duration::from_millis(millis)
        }
    };
    let brokers = match fields.get("brokers") {
        None => Vec::new(),
        Some(Value::Array(entries)) => parse_brokers(entries)?,
        Some(_) => {
            return Err(ConfigError::WrongType {
                key: "brokers".to_string(),
                wanted: "an array",
            });
        }
    };
    Ok(DaemonConfig {
        control_socket,
        status_deadline,
        brokers,
    })
}

fn parse_brokers(entries: &[Value]) -> Result<Vec<BrokerConfig>, ConfigError> {
    let mut brokers: Vec<BrokerConfig> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let at = format!("brokers[{index}]");
        let fields = object_at(entry, &at)?;
        refuse_unknown(fields, &["name", "control_socket", "command"], &at)?;
        let name = required_string(fields, "name", &at)?;
        if name.is_empty() {
            return Err(ConfigError::EmptyValue {
                key: format!("{at}.name"),
            });
        }
        if brokers.iter().any(|held| held.name == name) {
            return Err(ConfigError::DuplicateBrokerName { name });
        }
        let control_socket = PathBuf::from(required_string(fields, "control_socket", &at)?);
        let command = required_words(fields, "command", &at)?;
        brokers.push(BrokerConfig {
            name,
            control_socket,
            command,
        });
    }
    Ok(brokers)
}

fn object_at<'a>(value: &'a Value, at: &str) -> Result<&'a Map<String, Value>, ConfigError> {
    value.as_object().ok_or_else(|| ConfigError::WrongType {
        key: at.to_string(),
        wanted: "an object",
    })
}

fn refuse_unknown(
    fields: &Map<String, Value>,
    known: &[&str],
    at: &str,
) -> Result<(), ConfigError> {
    match fields.keys().find(|key| !known.contains(&key.as_str())) {
        Some(key) => Err(ConfigError::UnknownKey {
            key: named(at, key),
        }),
        None => Ok(()),
    }
}

fn named(at: &str, key: &str) -> String {
    if at.is_empty() {
        key.to_string()
    } else {
        format!("{at}.{key}")
    }
}

fn required_string(
    fields: &Map<String, Value>,
    key: &str,
    at: &str,
) -> Result<String, ConfigError> {
    match fields.get(key) {
        None => Err(ConfigError::MissingKey {
            key: named(at, key),
        }),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(ConfigError::WrongType {
            key: named(at, key),
            wanted: "a string",
        }),
    }
}

fn required_words(
    fields: &Map<String, Value>,
    key: &str,
    at: &str,
) -> Result<Vec<String>, ConfigError> {
    let wrong_type = || ConfigError::WrongType {
        key: named(at, key),
        wanted: "an array of strings",
    };
    let Some(value) = fields.get(key) else {
        return Err(ConfigError::MissingKey {
            key: named(at, key),
        });
    };
    let Some(entries) = value.as_array() else {
        return Err(wrong_type());
    };
    let mut words = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(word) = entry.as_str() else {
            return Err(wrong_type());
        };
        words.push(word.to_string());
    }
    if words.is_empty() {
        return Err(ConfigError::EmptyValue {
            key: named(at, key),
        });
    }
    Ok(words)
}

struct BoundSocket {
    path: PathBuf,
}

impl Drop for BoundSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Runs the daemon until `shutdown` is set, then tears it down.
///
/// Binds the control socket first, so a start that cannot bind has forked
/// nothing. An address already in use is refused whether it is a live daemon's
/// socket or a stale file: the daemon never unlinks a path it did not create,
/// because unlinking a live daemon's socket is how a half-alive supervisor is
/// manufactured. Every broker command is resolved before the first fork, so a
/// command that cannot run also leaves no children.
///
/// Returning — whether cleanly or with an error raised after the bind — kills
/// and reaps every broker and removes the socket path.
///
/// **What this cannot cover is `SIGKILL` of the daemon itself.** Brokers are
/// their own session leaders and do not die with their parent, and a killed
/// process runs no destructor, so the brokers keep running and the socket path
/// stays. That residue is observed rather than prevented, by the
/// `membrane.residue.snapshot` verb spec 001 §4 reserves for it.
pub fn run(config: DaemonConfig, shutdown: &AtomicBool) -> Result<(), DaemonError> {
    let listener =
        UnixListener::bind(&config.control_socket).map_err(|source| DaemonError::Bind {
            path: config.control_socket.clone(),
            source,
        })?;
    let _bound = BoundSocket {
        path: config.control_socket,
    };

    let deadline = config.status_deadline;
    let mut commands: HashMap<String, ExecCommand> = HashMap::with_capacity(config.brokers.len());
    let mut specs = Vec::with_capacity(config.brokers.len());
    for broker in config.brokers {
        let BrokerConfig {
            name,
            control_socket,
            command,
        } = broker;
        let built = ExecCommand::new(command).map_err(|source| DaemonError::BrokerCommand {
            broker: name.clone(),
            source,
        })?;
        commands.insert(name.clone(), built);
        specs.push(BrokerSpec {
            name,
            control_socket,
        });
    }

    let set = BrokerSet::spawn(
        specs,
        |spec: &BrokerSpec| match commands.remove(&spec.name) {
            Some(command) => VmmChild::spawn(command),
            None => Err(SpawnError::ForkFailed(std::io::Error::other(format!(
                "no command was built for broker `{}`",
                spec.name
            )))),
        },
        ControlSocketProbe,
    )
    .map_err(DaemonError::Spawn)?;

    control::serve(listener, shutdown, || set.status(deadline)).map_err(DaemonError::Listener)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    const STATUS_REQUEST: &str = r#"{"id":1,"method":"membrane.status","params":{}}"#;
    const PATIENCE: Duration = Duration::from_secs(10);
    const READY: &str = r#"{"id":0,"result":{"ready":true,"state":"serving"}}"#;
    const STARTING: &str = r#"{"id":0,"result":{"ready":false,"state":"starting"}}"#;

    fn sleeping_broker(pidfile: &Path) -> Vec<String> {
        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("echo $$ > {}; exec sleep 300", pidfile.display()),
        ]
    }

    fn scripted_broker(socket: PathBuf, answers: Vec<&'static str>) {
        let listener = UnixListener::bind(&socket).expect("the test broker binds its socket");
        thread::spawn(move || {
            for answer in answers {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
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

    fn ask(client: &mut BufReader<UnixStream>) -> Value {
        let mut stream = client.get_ref().try_clone().expect("clone for writing");
        writeln!(stream, "{STATUS_REQUEST}").expect("the request reaches the daemon");
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

    fn is_reaped(pid: i32) -> bool {
        let mut status: libc::c_int = 0;
        let observed = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        observed == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD)
    }

    #[test]
    fn a_full_config_parses_and_each_malformed_config_is_refused_by_name() {
        let parsed = parse_config(
            r#"{"control_socket": "/tmp/c.uds", "status_deadline_ms": 250,
                "brokers": [{"name": "egressd", "control_socket": "/tmp/b0.uds",
                             "command": ["sleep", "300"]}]}"#,
        )
        .expect("a full config parses");
        assert_eq!(
            parsed,
            DaemonConfig {
                control_socket: PathBuf::from("/tmp/c.uds"),
                status_deadline: Duration::from_millis(250),
                brokers: vec![BrokerConfig {
                    name: "egressd".to_string(),
                    control_socket: PathBuf::from("/tmp/b0.uds"),
                    command: vec!["sleep".to_string(), "300".to_string()],
                }],
            }
        );

        let bare = parse_config(r#"{"control_socket": "/tmp/c.uds"}"#)
            .expect("the two optional keys may be left out");
        assert_eq!(bare.status_deadline, DEFAULT_STATUS_DEADLINE);
        assert!(bare.brokers.is_empty());

        for (text, offender) in [
            (r#"{}"#, "control_socket"),
            (r#"{"control_socket": 7}"#, "control_socket"),
            (
                r#"{"control_socket": "/tmp/c.uds", "deadline_ms": 5}"#,
                "deadline_ms",
            ),
            (
                r#"{"control_socket": "/tmp/c.uds", "status_deadline_ms": 0}"#,
                "status_deadline_ms",
            ),
            (
                r#"{"control_socket": "/tmp/c.uds", "brokers": [{"name": "a",
                    "control_socket": "/tmp/b.uds", "command": ["sleep"], "extra": 1}]}"#,
                "brokers[0].extra",
            ),
            (
                r#"{"control_socket": "/tmp/c.uds", "brokers": [{"name": "",
                    "control_socket": "/tmp/b.uds", "command": ["sleep"]}]}"#,
                "brokers[0].name",
            ),
            (
                r#"{"control_socket": "/tmp/c.uds", "brokers": [{"name": "a",
                    "control_socket": "/tmp/b.uds", "command": []}]}"#,
                "brokers[0].command",
            ),
            (
                r#"{"control_socket": "/tmp/c.uds", "brokers": [{"name": "a",
                    "control_socket": "/tmp/b.uds", "command": "sleep"}]}"#,
                "brokers[0].command",
            ),
            (
                r#"{"control_socket": "/tmp/c.uds", "brokers": [
                    {"name": "a", "control_socket": "/tmp/b0.uds", "command": ["sleep"]},
                    {"name": "a", "control_socket": "/tmp/b1.uds", "command": ["sleep"]}]}"#,
                "a",
            ),
            (r#"not json"#, "JSON"),
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
    fn the_daemon_answers_status_with_its_brokers_readiness() {
        let dir = tempfile::tempdir().unwrap();
        let control = dir.path().join("c.uds");
        let broker_socket = dir.path().join("b0.uds");
        let pidfile = dir.path().join("b0.pid");
        scripted_broker(broker_socket.clone(), vec![READY, STARTING]);

        let daemon = start(DaemonConfig {
            control_socket: control.clone(),
            status_deadline: Duration::from_millis(500),
            brokers: vec![BrokerConfig {
                name: "egressd".to_string(),
                control_socket: broker_socket,
                command: sleeping_broker(&pidfile),
            }],
        });

        let mut client = addressable(&control);
        assert_eq!(
            ask(&mut client).get("result"),
            Some(&json!({"ready": true, "state": "serving"})),
            "the set is ready while its one broker answers ready"
        );
        assert_eq!(
            ask(&mut client).get("result"),
            Some(
                &json!({"ready": false, "state": "not_serving", "broker": "egressd",
                         "reason": "reported", "broker_state": "starting"})
            ),
            "the second call asks again rather than repeating the first answer"
        );

        let pid = recorded_pid(&pidfile);
        drop(client);
        daemon.stop().expect("a clean shutdown is not an error");

        assert!(
            is_reaped(pid),
            "the broker was killed and reaped, not orphaned"
        );
        assert!(
            !control.exists(),
            "the control socket path is removed on shutdown"
        );
    }

    #[test]
    fn a_daemon_with_no_brokers_answers_empty_and_never_ready() {
        let dir = tempfile::tempdir().unwrap();
        let control = dir.path().join("c.uds");
        let daemon = start(DaemonConfig {
            control_socket: control.clone(),
            status_deadline: Duration::from_millis(500),
            brokers: Vec::new(),
        });

        let mut client = addressable(&control);
        assert_eq!(
            ask(&mut client).get("result"),
            Some(&json!({"ready": false, "state": "empty"})),
            "a cell with nothing enforcing is never ready"
        );
        drop(client);
        daemon.stop().expect("a clean shutdown is not an error");
    }

    #[test]
    fn an_existing_control_socket_path_is_refused_before_any_fork() {
        let dir = tempfile::tempdir().unwrap();
        let control = dir.path().join("c.uds");
        std::fs::write(&control, b"someone else's socket").unwrap();
        let pidfile = dir.path().join("b0.pid");

        let shutdown = AtomicBool::new(false);
        let refusal = run(
            DaemonConfig {
                control_socket: control.clone(),
                status_deadline: Duration::from_millis(500),
                brokers: vec![BrokerConfig {
                    name: "egressd".to_string(),
                    control_socket: dir.path().join("b0.uds"),
                    command: sleeping_broker(&pidfile),
                }],
            },
            &shutdown,
        )
        .expect_err("an occupied control socket path refuses the start");

        match &refusal {
            DaemonError::Bind { path, .. } => assert_eq!(path, &control),
            other => panic!("an occupied path is a bind failure, got {other:?}"),
        }
        assert!(
            !pidfile.exists(),
            "run returned before spawning, so no broker was ever forked"
        );
        assert!(
            control.exists(),
            "the daemon does not unlink a path it did not bind"
        );
    }

    #[test]
    fn a_broker_command_that_cannot_resolve_refuses_startup_and_removes_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let control = dir.path().join("c.uds");
        let shutdown = AtomicBool::new(false);
        let refusal = run(
            DaemonConfig {
                control_socket: control.clone(),
                status_deadline: Duration::from_millis(500),
                brokers: vec![BrokerConfig {
                    name: "egressd".to_string(),
                    control_socket: dir.path().join("b0.uds"),
                    command: vec!["plasmosome-no-such-program".to_string()],
                }],
            },
            &shutdown,
        )
        .expect_err("a command that cannot resolve refuses the start");

        match &refusal {
            DaemonError::BrokerCommand { broker, .. } => assert_eq!(broker, "egressd"),
            other => panic!("an unresolvable command is a command failure, got {other:?}"),
        }
        assert!(
            refusal.to_string().contains("plasmosome-no-such-program"),
            "the refusal names the program, got: {refusal}"
        );
        assert!(!control.exists(), "the socket path is removed on a refusal");
    }

    #[test]
    fn a_spawn_refusal_reaps_the_spawned_and_removes_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let control = dir.path().join("c.uds");
        let shared = dir.path().join("shared.uds");
        let pidfile = dir.path().join("b0.pid");
        let shutdown = AtomicBool::new(false);

        let refusal = run(
            DaemonConfig {
                control_socket: control.clone(),
                status_deadline: Duration::from_millis(500),
                brokers: vec![
                    BrokerConfig {
                        name: "egressd".to_string(),
                        control_socket: shared.clone(),
                        command: sleeping_broker(&pidfile),
                    },
                    BrokerConfig {
                        name: "dnsd".to_string(),
                        control_socket: shared,
                        command: vec!["sleep".to_string(), "300".to_string()],
                    },
                ],
            },
            &shutdown,
        )
        .expect_err("two brokers may not share one control socket");

        match &refusal {
            DaemonError::Spawn(failure) => assert_eq!(failure.broker, "dnsd"),
            other => panic!("a shared socket is a spawn failure, got {other:?}"),
        }
        thread::sleep(Duration::from_secs(1));
        assert!(
            !pidfile.exists(),
            "the broker that was spawned was killed before it could run. A broker left running \
             records its pid within 50ms, measured; this waits twenty times that, so under load \
             the assertion fails rather than passing early. The reap itself is observed by \
             ECHILD at the brokers layer, which is the only place the pid is knowable — here \
             the kill lands before the child finishes exec'ing, so no pid is ever recorded"
        );
        assert!(!control.exists(), "the socket path is removed on a refusal");
    }
}
