use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(test)]
pub(crate) mod fault;

/// Why a session log operation failed.
#[derive(Debug)]
pub enum SessionLogError {
    /// An IO call failed. `path` is the file or directory it was made on and `step` names the
    /// call. After an append fails this way the line may still be on disk in whole or in part.
    Io {
        path: PathBuf,
        step: LogStep,
        source: std::io::Error,
    },
    /// The log's bytes are not a sequence of complete, valid events. `line` is the 1-based
    /// physical line holding the first fault. The file was left as it was.
    Malformed {
        path: PathBuf,
        line: usize,
        fault: LogFault,
    },
    /// An earlier failure stopped this log from accepting appends. Open a new `SessionLog` on the
    /// same path to validate the file and continue.
    Poisoned { path: PathBuf },
}

/// The IO call a [`SessionLogError::Io`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStep {
    CreateDirectory,
    SyncDirectory,
    Open,
    Write,
    Flush,
    SyncFile,
    Read,
}

/// What is wrong with the line a [`SessionLogError::Malformed`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFault {
    /// The file does not end with LF, so its last line may be torn.
    MissingNewline,
    NotUtf8,
    NotJson,
    NotAnObject,
    /// The object lacks an unsigned integer `seq` or a string `kind`.
    MissingEnvelope,
    /// The `seq` is not greater than the line before it.
    SequenceNotIncreasing,
    /// The last `seq` is `u64::MAX`, so no later event can be numbered.
    SequenceExhausted,
}

impl std::fmt::Display for LogStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LogStep::CreateDirectory => "create directory",
            LogStep::SyncDirectory => "sync directory",
            LogStep::Open => "open",
            LogStep::Write => "write",
            LogStep::Flush => "flush",
            LogStep::SyncFile => "sync file",
            LogStep::Read => "read",
        })
    }
}

impl std::fmt::Display for LogFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LogFault::MissingNewline => "no final newline",
            LogFault::NotUtf8 => "not UTF-8",
            LogFault::NotJson => "not JSON",
            LogFault::NotAnObject => "not a JSON object",
            LogFault::MissingEnvelope => "no unsigned seq or string kind",
            LogFault::SequenceNotIncreasing => "seq does not increase",
            LogFault::SequenceExhausted => "seq cannot advance past u64::MAX",
        })
    }
}

impl std::fmt::Display for SessionLogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionLogError::Io { path, step, source } => {
                write!(f, "session log {}: {step} failed: {source}", path.display())
            }
            SessionLogError::Malformed { path, line, fault } => {
                write!(f, "session log {} line {line}: {fault}", path.display())
            }
            SessionLogError::Poisoned { path } => write!(
                f,
                "session log {} refuses appends after an earlier failure; reopen it",
                path.display()
            ),
        }
    }
}

impl std::error::Error for SessionLogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SessionLogError::Io { source, .. } => Some(source),
            SessionLogError::Malformed { .. } | SessionLogError::Poisoned { .. } => None,
        }
    }
}

/// Where a session log's bytes and directories live. Production uses [`OsLogStore`].
pub trait LogStore {
    /// Creates one directory whose parent already exists. Fails if `path` exists.
    fn create_dir(&self, path: &Path) -> std::io::Result<()>;
    /// Makes the entries of the existing directory at `path` durable.
    fn sync_dir(&self, path: &Path) -> std::io::Result<()>;
    /// Opens `path` for reading and appending without following a final symlink, creating the
    /// file if it is missing. Refuses anything that is not a regular file.
    fn open_log(&self, path: &Path) -> std::io::Result<OpenedLog>;
}

/// An open log file. Each method has the meaning of the `std` call of the same name.
pub trait LogFile: Send {
    /// Writes all of `bytes` at the end of the file, or fails having written any prefix.
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()>;
    /// Pushes buffered bytes to the operating system. This is not durability.
    fn flush(&mut self) -> std::io::Result<()>;
    /// Returns only once the file's data and metadata are durable.
    fn sync_all(&mut self) -> std::io::Result<()>;
}

/// What [`LogStore::open_log`] found.
pub struct OpenedLog {
    /// The open file, positioned to append.
    pub file: Box<dyn LogFile>,
    /// Every byte the file held when it was opened.
    pub existing: Vec<u8>,
    /// Whether this call created the file.
    pub created: bool,
}

/// The production [`LogStore`] over `std::fs`.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsLogStore;

impl LogStore for OsLogStore {
    fn create_dir(&self, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir(path)
    }

    fn sync_dir(&self, path: &Path) -> std::io::Result<()> {
        std::fs::File::open(path)?.sync_all()
    }

    fn open_log(&self, path: &Path) -> std::io::Result<OpenedLog> {
        let options = |create: bool| {
            let mut options = std::fs::OpenOptions::new();
            options
                .read(true)
                .append(true)
                .create_new(create)
                .custom_flags(libc::O_NOFOLLOW);
            options
        };
        match options(true).open(path) {
            Ok(file) => Ok(OpenedLog {
                file: Box::new(file),
                existing: Vec::new(),
                created: true,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let file = options(false).open(path)?;
                if !file.metadata()?.is_file() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "the session log path is not a regular file",
                    ));
                }
                Ok(OpenedLog {
                    file: Box::new(file),
                    existing: Vec::new(),
                    created: false,
                })
            }
            Err(error) => Err(error),
        }
    }
}

impl LogFile for std::fs::File {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        std::io::Write::write_all(self, bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(self)
    }

    fn sync_all(&mut self) -> std::io::Result<()> {
        std::fs::File::sync_all(self)
    }
}

/// The append-only event log one instance's cells share. Each line is one JSON object whose
/// envelope is `ts_ms`, `seq` and `kind`.
pub struct SessionLog {
    path: PathBuf,
    state: Mutex<LogState>,
}

struct LogState {
    file: Box<dyn LogFile>,
    next_seq: u64,
    poisoned: bool,
}

impl SessionLog {
    /// Opens the log at `path` with [`OsLogStore`]. See [`SessionLog::open_with`].
    pub fn open(path: PathBuf) -> Result<SessionLog, SessionLogError> {
        SessionLog::open_with(path, &OsLogStore)
    }

    /// Opens the log at `path` through `store`, creating it and any missing parent directories.
    ///
    /// Each directory this creates, and the file if this creates it, has its containing directory
    /// synced. The nearest existing ancestor must be a directory; a symlink at `path` itself is
    /// refused. The file and its directory are synced before this returns, so reopening a log
    /// also makes durable whatever an earlier writer left unsynced. Any failed step is returned.
    pub fn open_with(path: PathBuf, store: &dyn LogStore) -> Result<SessionLog, SessionLogError> {
        let parent = parent_of(&path);
        create_parents(parent, store)?;
        let opened = store
            .open_log(&path)
            .map_err(io_error(&path, LogStep::Open))?;
        if opened.created {
            store
                .sync_dir(parent)
                .map_err(io_error(parent, LogStep::SyncDirectory))?;
        }
        let mut file = opened.file;
        file.sync_all()
            .map_err(io_error(&path, LogStep::SyncFile))?;
        store
            .sync_dir(parent)
            .map_err(io_error(parent, LogStep::SyncDirectory))?;
        Ok(SessionLog {
            path,
            state: Mutex::new(LogState {
                file,
                next_seq: 1,
                poisoned: false,
            }),
        })
    }

    /// The path this log was opened at.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one event of `kind`. Fields of an object `payload` are added after the envelope
    /// and never replace `ts_ms`, `seq` or `kind`.
    ///
    /// Returns the event's sequence number only after the whole LF-terminated line was written,
    /// flushed and synced with `sync_all`. The first write, flush or sync error poisons this log:
    /// that call returns [`SessionLogError::Io`] and every later append, from any thread, returns
    /// [`SessionLogError::Poisoned`]. A panic while appending poisons it the same way. An error
    /// does not mean the line is absent: it may be on disk whole or in part, so open a new
    /// `SessionLog` to validate the file before continuing.
    pub fn append(&self, kind: &str, payload: serde_json::Value) -> Result<u64, SessionLogError> {
        let poisoned = || SessionLogError::Poisoned {
            path: self.path.clone(),
        };
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if state.poisoned {
            return Err(poisoned());
        }
        let seq = state.next_seq;
        let line = event_line(seq, kind, payload);
        if let Err((step, source)) = write_durably(state.file.as_mut(), line.as_bytes()) {
            state.poisoned = true;
            return Err(SessionLogError::Io {
                path: self.path.clone(),
                step,
                source,
            });
        }
        state.next_seq = seq + 1;
        Ok(seq)
    }
}

fn io_error(path: &Path, step: LogStep) -> impl FnOnce(std::io::Error) -> SessionLogError {
    let path = path.to_path_buf();
    move |source| SessionLogError::Io { path, step, source }
}

fn parent_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

fn create_parents(parent: &Path, store: &dyn LogStore) -> Result<(), SessionLogError> {
    let mut missing = Vec::new();
    let mut cursor = parent;
    loop {
        match std::fs::symlink_metadata(cursor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let above = parent_of(cursor);
                if above == cursor {
                    return Err(io_error(cursor, LogStep::CreateDirectory)(error));
                }
                missing.push(cursor);
                cursor = above;
            }
            Err(error) => return Err(io_error(cursor, LogStep::CreateDirectory)(error)),
        }
    }
    let existing = std::fs::metadata(cursor).map_err(io_error(cursor, LogStep::CreateDirectory))?;
    if !existing.is_dir() {
        return Err(io_error(cursor, LogStep::CreateDirectory)(
            std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "an existing ancestor of the session log is not a directory",
            ),
        ));
    }
    for directory in missing.into_iter().rev() {
        store
            .create_dir(directory)
            .map_err(io_error(directory, LogStep::CreateDirectory))?;
        let above = parent_of(directory);
        store
            .sync_dir(above)
            .map_err(io_error(above, LogStep::SyncDirectory))?;
    }
    Ok(())
}

fn write_durably(file: &mut dyn LogFile, bytes: &[u8]) -> Result<(), (LogStep, std::io::Error)> {
    file.write_all(bytes)
        .map_err(|source| (LogStep::Write, source))?;
    file.flush().map_err(|source| (LogStep::Flush, source))?;
    file.sync_all()
        .map_err(|source| (LogStep::SyncFile, source))
}

fn event_line(seq: u64, kind: &str, payload: serde_json::Value) -> String {
    let mut event = serde_json::Map::new();
    event.insert(
        "ts_ms".to_string(),
        serde_json::Value::Number(system_millis().into()),
    );
    event.insert("seq".to_string(), serde_json::Value::Number(seq.into()));
    event.insert(
        "kind".to_string(),
        serde_json::Value::String(kind.to_string()),
    );
    if let serde_json::Value::Object(fields) = payload {
        for (key, value) in fields {
            event.entry(key).or_insert(value);
        }
    }
    let mut line = serde_json::Value::Object(event).to_string();
    line.push('\n');
    line
}

/// Reads every event in the log at `path`. A missing file has no events.
pub fn read_events(path: &Path) -> Result<Vec<serde_json::Value>, SessionLogError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(SessionLogError::Io {
                path: path.to_path_buf(),
                step: LogStep::Read,
                source,
            });
        }
    };
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// [`read_events`] filtered to the events whose `kind` is `kind`.
pub fn events_of_kind(path: &Path, kind: &str) -> Result<Vec<serde_json::Value>, SessionLogError> {
    Ok(read_events(path)?
        .into_iter()
        .filter(|event| event.get("kind").and_then(serde_json::Value::as_str) == Some(kind))
        .collect())
}

fn system_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::fault::{Call, FaultLogStore, Step};
    use super::*;
    use serde_json::json;

    const LOG: &str = "session.ndjson";

    fn open_in(store: &FaultLogStore) -> SessionLog {
        SessionLog::open_with(store.root().join(LOG), store).unwrap()
    }

    fn open_then_fail(step: Step) -> (FaultLogStore, SessionLog) {
        let store = FaultLogStore::new();
        let log = open_in(&store);
        let nth = store.count(step) + 1;
        (store.fail(step, nth), log)
    }

    fn assert_io(result: Result<u64, SessionLogError>, expected: LogStep, at: &Path) {
        match result {
            Err(SessionLogError::Io { path, step, .. }) => {
                assert_eq!(step, expected);
                assert_eq!(path, at);
            }
            other => panic!("expected an Io error at {expected:?}, got {other:?}"),
        }
    }

    fn assert_poisoned(result: Result<u64, SessionLogError>) {
        assert!(
            matches!(result, Err(SessionLogError::Poisoned { .. })),
            "expected Poisoned, got {result:?}"
        );
    }

    fn assert_send_sync<T: Send + Sync>() {}

    fn open_error(path: PathBuf, store: &dyn LogStore) -> SessionLogError {
        match SessionLog::open_with(path, store) {
            Ok(_) => panic!("open succeeded"),
            Err(error) => error,
        }
    }

    fn assert_open_io(error: SessionLogError, expected: LogStep, at: &Path) {
        match error {
            SessionLogError::Io { path, step, .. } => {
                assert_eq!(step, expected);
                assert_eq!(path, at);
            }
            other => panic!("expected an Io error at {expected:?}, got {other:?}"),
        }
    }

    #[test]
    fn events_are_appended_with_monotonic_seq_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::open(dir.path().join(LOG)).unwrap();
        log.append("plugin_attach", serde_json::json!({ "id": "github-pr" }))
            .unwrap();
        log.append("tool_invoke", serde_json::json!({ "name": "pr.read" }))
            .unwrap();
        let events = read_events(&dir.path().join(LOG)).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["kind"], "plugin_attach");
        assert_eq!(events[0]["id"], "github-pr");
        assert_eq!(events[1]["kind"], "tool_invoke");
        let first = events[0]["seq"].as_u64().unwrap();
        let second = events[1]["seq"].as_u64().unwrap();
        assert_eq!(second, first + 1);
    }

    #[test]
    fn payload_fields_never_override_the_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::open(dir.path().join(LOG)).unwrap();
        log.append("turn", serde_json::json!({ "kind": "spoofed", "extra": 1 }))
            .unwrap();
        let events = read_events(&dir.path().join(LOG)).unwrap();
        assert_eq!(
            events[0]["kind"], "turn",
            "the envelope owns the kind field"
        );
        assert_eq!(events[0]["extra"], 1);
    }

    #[test]
    fn events_of_kind_filters_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::open(dir.path().join(LOG)).unwrap();
        log.append("a", serde_json::json!({})).unwrap();
        log.append("b", serde_json::json!({})).unwrap();
        log.append("a", serde_json::json!({})).unwrap();
        let a_events = events_of_kind(&dir.path().join(LOG), "a").unwrap();
        assert_eq!(a_events.len(), 2);
    }

    #[test]
    fn reading_a_missing_log_yields_no_events() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            read_events(&dir.path().join("missing.ndjson"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn append_returns_the_sequence_it_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::open(dir.path().join(LOG)).unwrap();
        assert_eq!(log.append("a", serde_json::json!({})).unwrap(), 1);
        assert_eq!(log.append("b", serde_json::json!({})).unwrap(), 2);
        let bytes = std::fs::read(dir.path().join(LOG)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.ends_with('\n'), "every line ends with LF: {text:?}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "exactly two lines: {text:?}");
        let seqs: Vec<u64> = lines
            .iter()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["seq"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        assert_eq!(seqs, vec![1, 2]);
    }

    fn a_failed_step_is_returned_and_poisons_the_log(fault: Step, expected: LogStep) {
        let (store, log) = open_then_fail(fault);
        let at = store.root().join(LOG);
        assert_io(log.append("force", json!({ "n": 1 })), expected, &at);
        assert_poisoned(log.append("force", json!({ "n": 2 })));
        assert_eq!(
            store.count(Step::Write),
            1,
            "a poisoned log writes nothing more: {:?}",
            store.calls()
        );
    }

    #[test]
    fn a_failed_write_is_returned_and_poisons_the_log() {
        a_failed_step_is_returned_and_poisons_the_log(Step::Write, LogStep::Write);
    }

    #[test]
    fn a_failed_flush_is_returned_and_poisons_the_log() {
        a_failed_step_is_returned_and_poisons_the_log(Step::Flush, LogStep::Flush);
    }

    #[test]
    fn a_failed_file_sync_is_returned_and_poisons_the_log() {
        a_failed_step_is_returned_and_poisons_the_log(Step::SyncFile, LogStep::SyncFile);
    }

    #[test]
    fn success_is_returned_only_after_write_flush_and_sync() {
        let store = FaultLogStore::new();
        let log = open_in(&store);
        let before = store.calls().len();
        assert_eq!(log.append("a", json!({})).unwrap(), 1);
        let written = std::fs::read(store.root().join(LOG)).unwrap().len();
        assert_eq!(
            store.calls()[before..],
            [Call::Write(written), Call::Flush, Call::SyncFile]
        );
    }

    #[test]
    fn one_failure_refuses_every_other_caller() {
        assert_send_sync::<SessionLog>();
        let (store, log) = open_then_fail(Step::Write);
        let log = &log;
        let (a_failed, a_has_failed) = std::sync::mpsc::channel();
        let (b_done, b_is_done) = std::sync::mpsc::channel();
        let (a_first, a_again, b) = std::thread::scope(|scope| {
            let a = scope.spawn(move || {
                let first = log.append("force", json!({ "cell": "a" }));
                a_failed.send(()).unwrap();
                b_is_done.recv().unwrap();
                (first, log.append("force", json!({ "cell": "a" })))
            });
            let b = scope.spawn(move || {
                a_has_failed.recv().unwrap();
                let result = log.append("force", json!({ "cell": "b" }));
                b_done.send(()).unwrap();
                result
            });
            let b = b.join().unwrap();
            let (first, again) = a.join().unwrap();
            (first, again, b)
        });
        assert_io(a_first, LogStep::Write, &store.root().join(LOG));
        assert_poisoned(b);
        assert_poisoned(a_again);
        assert_eq!(store.count(Step::Write), 1, "{:?}", store.calls());
    }

    #[test]
    fn a_panic_inside_append_poisons_the_log() {
        let store = FaultLogStore::new();
        let log = open_in(&store);
        let nth = store.count(Step::Write) + 1;
        let store = store.panic_on_write(nth);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            log.append("force", json!({ "n": 1 }))
        }));
        assert!(panicked.is_err(), "the injected write panics");
        assert_poisoned(log.append("force", json!({ "n": 2 })));
        assert_eq!(store.count(Step::Write), 1, "{:?}", store.calls());
    }

    #[test]
    fn open_creates_missing_parents_and_syncs_each_new_entry() {
        let store = FaultLogStore::new();
        let root = store.root().to_path_buf();
        let path = root.join("a").join("b").join(LOG);
        SessionLog::open_with(path.clone(), &store).unwrap();
        assert_eq!(
            store.calls(),
            [
                Call::CreateDir(root.join("a")),
                Call::SyncDir(root.clone()),
                Call::CreateDir(root.join("a").join("b")),
                Call::SyncDir(root.join("a")),
                Call::Open(path.clone()),
                Call::SyncDir(root.join("a").join("b")),
                Call::SyncFile,
                Call::SyncDir(root.join("a").join("b")),
            ]
        );
        assert!(path.is_file());
    }

    #[test]
    fn a_failed_parent_sync_refuses_open() {
        let synced = |root: &Path| {
            [
                root.to_path_buf(),
                root.join("a"),
                root.join("a").join("b"),
                root.join("a").join("b"),
            ]
        };
        for (index, _) in synced(Path::new("")).iter().enumerate() {
            let store = FaultLogStore::new().fail(Step::SyncDir, index + 1);
            let directory = synced(store.root())[index].clone();
            let error = open_error(store.root().join("a").join("b").join(LOG), &store);
            assert_open_io(error, LogStep::SyncDirectory, &directory);
        }
    }

    #[test]
    fn a_failed_create_open_or_file_sync_refuses_open() {
        let cases = [
            (Step::CreateDir, LogStep::CreateDirectory),
            (Step::Open, LogStep::Open),
            (Step::SyncFile, LogStep::SyncFile),
        ];
        for (fault, expected) in cases {
            let store = FaultLogStore::new().fail(fault, 1);
            let path = store.root().join("a").join(LOG);
            let at = match fault {
                Step::CreateDir => store.root().join("a"),
                _ => path.clone(),
            };
            assert_open_io(open_error(path, &store), expected, &at);
        }
    }

    #[test]
    fn open_refuses_a_parent_that_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"keep").unwrap();
        for path in [blocker.join(LOG), blocker.join("sub").join(LOG)] {
            match open_error(path.clone(), &OsLogStore) {
                SessionLogError::Io { step, source, .. } => {
                    assert_eq!(step, LogStep::CreateDirectory, "{path:?}");
                    assert_eq!(source.kind(), std::io::ErrorKind::NotADirectory, "{path:?}");
                }
                other => panic!("expected an Io error for {path:?}, got {other:?}"),
            }
        }
        assert_eq!(std::fs::read(&blocker).unwrap(), b"keep");
    }

    #[test]
    fn open_refuses_a_symlinked_log() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"victim\n").unwrap();
        let planted = dir.path().join(LOG);
        std::os::unix::fs::symlink(&target, &planted).unwrap();
        assert_open_io(
            open_error(planted.clone(), &OsLogStore),
            LogStep::Open,
            &planted,
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"victim\n");

        let missing = dir.path().join("missing");
        let dangling = dir.path().join("dangling.ndjson");
        std::os::unix::fs::symlink(&missing, &dangling).unwrap();
        assert_open_io(
            open_error(dangling.clone(), &OsLogStore),
            LogStep::Open,
            &dangling,
        );
        assert!(
            !missing.exists(),
            "a dangling symlink's target is never created"
        );
    }
}
