use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(OpenedLog {
            file: Box::new(file),
            existing: Vec::new(),
            created: false,
        })
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
}

impl SessionLog {
    /// Opens the log at `path` with [`OsLogStore`]. See [`SessionLog::open_with`].
    pub fn open(path: PathBuf) -> Result<SessionLog, SessionLogError> {
        SessionLog::open_with(path, &OsLogStore)
    }

    /// Opens the log at `path` through `store`, creating it and any missing parent directories.
    pub fn open_with(path: PathBuf, store: &dyn LogStore) -> Result<SessionLog, SessionLogError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| SessionLogError::Io {
                path: parent.to_path_buf(),
                step: LogStep::CreateDirectory,
                source,
            })?;
        }
        let opened = store
            .open_log(&path)
            .map_err(|source| SessionLogError::Io {
                path: path.clone(),
                step: LogStep::Open,
                source,
            })?;
        Ok(SessionLog {
            path,
            state: Mutex::new(LogState {
                file: opened.file,
                next_seq: 1,
            }),
        })
    }

    /// The path this log was opened at.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one event of `kind`. Fields of an object `payload` are added after the envelope
    /// and never replace `ts_ms`, `seq` or `kind`. Returns the event's sequence number.
    pub fn append(&self, kind: &str, payload: serde_json::Value) -> Result<u64, SessionLogError> {
        let mut state = self
            .state
            .lock()
            .expect("session log file lock is never poisoned while held");
        let seq = state.next_seq;
        state.next_seq += 1;
        let line = event_line(seq, kind, payload);
        let _ = state.file.write_all(line.as_bytes());
        let _ = state.file.flush();
        Ok(seq)
    }
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
    use super::*;

    const LOG: &str = "session.ndjson";

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
}
