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
    /// An earlier failure stopped this log from accepting appends and released its writer lock.
    /// One recovering owner opens a new `SessionLog` on the same path, which validates the file;
    /// every other holder of the poisoned value must use that new log rather than open its own,
    /// because a second open is refused with [`SessionLogError::Locked`].
    Poisoned { path: PathBuf },
    /// Another open `SessionLog`, in this process or another, holds the writer lock on the file
    /// at `path`. Nothing was read or written.
    Locked { path: PathBuf },
    /// An append's event line is `bytes` long without its LF, more than
    /// [`SessionLog::MAX_LINE_BYTES`]. Nothing was written and the log still accepts appends.
    EventTooLong { path: PathBuf, bytes: usize },
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
    /// The line is longer than [`SessionLog::MAX_LINE_BYTES`] before its LF, or has no LF and
    /// is already longer than that.
    LineTooLong,
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
        match self {
            LogFault::MissingNewline => f.write_str("no final newline"),
            LogFault::NotUtf8 => f.write_str("not UTF-8"),
            LogFault::NotJson => f.write_str("not JSON"),
            LogFault::NotAnObject => f.write_str("not a JSON object"),
            LogFault::MissingEnvelope => f.write_str("no unsigned seq or string kind"),
            LogFault::SequenceNotIncreasing => f.write_str("seq does not increase"),
            LogFault::SequenceExhausted => f.write_str("seq cannot advance past u64::MAX"),
            LogFault::LineTooLong => {
                write!(f, "longer than {} bytes", SessionLog::MAX_LINE_BYTES)
            }
        }
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
            SessionLogError::Locked { path } => write!(
                f,
                "session log {} is held by another writer",
                path.display()
            ),
            SessionLogError::EventTooLong { path, bytes } => write!(
                f,
                "session log {}: an event line of {bytes} bytes is longer than {}",
                path.display(),
                SessionLog::MAX_LINE_BYTES
            ),
        }
    }
}

impl std::error::Error for SessionLogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SessionLogError::Io { source, .. } => Some(source),
            SessionLogError::Malformed { .. }
            | SessionLogError::Poisoned { .. }
            | SessionLogError::Locked { .. }
            | SessionLogError::EventTooLong { .. } => None,
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
    /// file if it is missing. Refuses anything that is not a regular file, without blocking on
    /// it. Takes a nonblocking exclusive lock on the file, held for as long as the returned file
    /// is open; if another open file holds that lock, fails with `ErrorKind::WouldBlock`.
    fn open_log(&self, path: &Path) -> std::io::Result<Box<dyn LogFile>>;
}

/// An open log file, read from its start through [`std::io::Read`] and appended to at its end.
/// Each method has the meaning of the `std` call of the same name.
pub trait LogFile: std::io::Read + Send {
    /// Writes all of `bytes` at the end of the file, or fails having written any prefix.
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()>;
    /// Pushes buffered bytes to the operating system. This is not durability.
    fn flush(&mut self) -> std::io::Result<()>;
    /// Returns only once the file's data and metadata are durable.
    fn sync_all(&mut self) -> std::io::Result<()>;
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

    fn open_log(&self, path: &Path) -> std::io::Result<Box<dyn LogFile>> {
        let options = |create: bool| {
            let mut options = std::fs::OpenOptions::new();
            options.read(true).append(true).create_new(create);
            options
        };
        let file = match open_regular(&mut options(true), path) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                open_regular(&mut options(false), path)?
            }
            opened => opened?,
        };
        file.try_lock()?;
        Ok(Box::new(OsLogFile(file)))
    }
}

struct OsLogFile(std::fs::File);

impl Drop for OsLogFile {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl std::io::Read for OsLogFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(&mut self.0, buf)
    }
}

fn open_regular(options: &mut std::fs::OpenOptions, path: &Path) -> std::io::Result<std::fs::File> {
    let file = options
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the session log path is not a regular file",
        ));
    }
    let descriptor = std::os::fd::AsRawFd::as_raw_fd(&file);
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1
        || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags & !libc::O_NONBLOCK) } == -1
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

impl LogFile for OsLogFile {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        std::io::Write::write_all(&mut self.0, bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(&mut self.0)
    }

    fn sync_all(&mut self) -> std::io::Result<()> {
        self.0.sync_all()
    }
}

/// The append-only event log one instance's cells share through a single value of this type.
/// Each line is one JSON object whose envelope is `ts_ms`, `seq` and `kind`.
///
/// An open log holds an exclusive advisory lock (`flock`) on its file, so any other open of that
/// file, from this process or another, returns [`SessionLogError::Locked`]. The lock is released
/// when this value is dropped, or as soon as an append poisons it: before that append returns its
/// [`SessionLogError::Io`], before a panic inside it continues to unwind, and once it has written
/// `seq` `u64::MAX`. The lock is advisory: it stops other `SessionLog`s, not a process that
/// writes the file without asking for it.
///
/// The lock belongs to the open file, which a child process forked while the log is open shares.
/// Dropping or poisoning this value unlocks the file explicitly, so such a child does not keep
/// the lock. The unlock works from either side: a forked child that drops a `SessionLog` it
/// inherited releases the parent's lock, so a forked child must exec or exit without running the
/// parent's destructors.
pub struct SessionLog {
    path: PathBuf,
    state: Mutex<LogState>,
}

struct LogState {
    file: Option<Box<dyn LogFile>>,
    next_seq: u64,
}

impl SessionLog {
    /// The longest line, in bytes before its LF, that a log writes or accepts.
    pub const MAX_LINE_BYTES: usize = 1 << 20;

    /// Opens the log at `path` with [`OsLogStore`]. See [`SessionLog::open_with`].
    pub fn open(path: PathBuf) -> Result<SessionLog, SessionLogError> {
        SessionLog::open_with(path, &OsLogStore)
    }

    /// Opens the log at `path` through `store`, creating it and any missing parent directories.
    ///
    /// The nearest existing ancestor must be a directory, and symlinks among the ancestors are
    /// followed. A symlink at `path`, or anything there that is not a regular file, is refused.
    /// The open takes the writer lock described on [`SessionLog`], or returns
    /// [`SessionLogError::Locked`]. An existing log must pass the checks [`read_events`] makes,
    /// or this returns [`SessionLogError::Malformed`] and leaves the file as it was; a log whose
    /// last `seq` is `u64::MAX` is refused too. The check reads one line at a time and keeps only
    /// the last `seq`, so its memory grows with the longest line, not with the log.
    ///
    /// Before returning, every open syncs the file, then its directory and each ancestor of that
    /// directory up to the root, whether or not this call created them. A directory an earlier,
    /// interrupted open created is therefore made durable by the next open. An ancestor this
    /// process cannot open for reading fails the open at [`LogStep::SyncDirectory`]. Syncing the
    /// file again does not prove that a line written before a failed sync is on disk: on Linux a
    /// failed `fsync` can drop the unwritten pages, and the next sync then succeeds without them.
    /// After an append that returned an error, assert that event again rather than relying on
    /// the reopen. Any failed step is returned. The next append is numbered one more than the
    /// last line's `seq`, or 1.
    pub fn open_with(path: PathBuf, store: &dyn LogStore) -> Result<SessionLog, SessionLogError> {
        let parent = parent_of(&path);
        create_parents(parent, store)?;
        let mut file = store.open_log(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::WouldBlock {
                SessionLogError::Locked { path: path.clone() }
            } else {
                io_error(&path, LogStep::Open)(source)
            }
        })?;
        let next_seq = match scan_lines(&path, &mut file, |_| {})? {
            None => 1,
            Some((line, last)) => {
                last.checked_add(1)
                    .ok_or_else(|| SessionLogError::Malformed {
                        path: path.clone(),
                        line,
                        fault: LogFault::SequenceExhausted,
                    })?
            }
        };
        file.sync_all()
            .map_err(io_error(&path, LogStep::SyncFile))?;
        sync_up_from(parent, store)?;
        Ok(SessionLog {
            path,
            state: Mutex::new(LogState {
                file: Some(file),
                next_seq,
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
    /// [`SessionLogError::Poisoned`]. A panic while appending poisons it the same way, and so does
    /// writing `seq` `u64::MAX`, after which no event can be numbered. Poisoning releases the
    /// writer lock. An error does not mean the line is absent: it may be on disk whole or in part,
    /// so one recovering owner opens a new `SessionLog` to validate the file before continuing.
    ///
    /// The log's mutex is held across the write and the `sync_all`, so appends from every thread
    /// wait behind one full sync each. On macOS `sync_all` is `F_FULLFSYNC`, and the
    /// `session_log_append` bench measures about 7 to 10 ms per append on an idle machine, more
    /// under load: at most about 100 to 140 appends a second for a whole instance. A producer
    /// that needs a higher rate needs group commit or a path that does not sync.
    pub fn append(&self, kind: &str, payload: serde_json::Value) -> Result<u64, SessionLogError> {
        let poisoned = || SessionLogError::Poisoned {
            path: self.path.clone(),
        };
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        let seq = state.next_seq;
        let Some(file) = state.file.as_mut() else {
            return Err(poisoned());
        };
        let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            write_durably(file.as_mut(), event_line(seq, kind, payload).as_bytes())
        }));
        match written {
            Ok(Ok(())) => {}
            Ok(Err((step, source))) => {
                state.file = None;
                return Err(SessionLogError::Io {
                    path: self.path.clone(),
                    step,
                    source,
                });
            }
            Err(panic) => {
                state.file = None;
                std::panic::resume_unwind(panic);
            }
        }
        match seq.checked_add(1) {
            Some(next) => state.next_seq = next,
            None => state.file = None,
        }
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
    }
    Ok(())
}

fn sync_up_from(directory: &Path, store: &dyn LogStore) -> Result<(), SessionLogError> {
    let absolute =
        std::path::absolute(directory).map_err(io_error(directory, LogStep::SyncDirectory))?;
    for each in absolute.ancestors() {
        store
            .sync_dir(each)
            .map_err(io_error(each, LogStep::SyncDirectory))?;
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

/// Reads every event in the log at `path`, in file order.
///
/// When nothing exists at `path` the log has no events and this returns an empty list. The file
/// is opened the way [`SessionLog::open`] opens it, without following a symlink and without
/// blocking, so a symlink at `path`, dangling or not, and anything that is not a regular file,
/// such as a FIFO, is refused with [`SessionLogError::Io`] at [`LogStep::Read`].
///
/// The whole read is refused with [`SessionLogError::Malformed`], naming the first bad physical
/// line, when the file does not end with LF, or a line is not UTF-8, is not a JSON object, lacks
/// an unsigned integer `seq` or a string `kind`, or has a `seq` not greater than the line before.
/// No line is skipped and the file is never changed. The read takes no lock, so reading while a
/// writer appends can see its line half written and refuse it as a missing newline.
pub fn read_events(path: &Path) -> Result<Vec<serde_json::Value>, SessionLogError> {
    let mut file = match open_regular(std::fs::OpenOptions::new().read(true), path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io_error(path, LogStep::Read)(source)),
    };
    let mut events = Vec::new();
    scan_lines(path, &mut file, |event| events.push(event))?;
    Ok(events)
}

fn scan_lines(
    path: &Path,
    file: &mut dyn std::io::Read,
    mut keep: impl FnMut(serde_json::Value),
) -> Result<Option<(usize, u64)>, SessionLogError> {
    let malformed = |line, fault| SessionLogError::Malformed {
        path: path.to_path_buf(),
        line,
        fault,
    };
    let mut reader = std::io::BufReader::new(file);
    let mut bytes = Vec::new();
    let mut last: Option<(usize, u64)> = None;
    for line in 1.. {
        bytes.clear();
        let read = std::io::BufRead::read_until(&mut reader, b'\n', &mut bytes)
            .map_err(io_error(path, LogStep::Read))?;
        if read == 0 {
            break;
        }
        let Some(segment) = bytes.strip_suffix(b"\n") else {
            return Err(malformed(line, LogFault::MissingNewline));
        };
        let text = std::str::from_utf8(segment).map_err(|_| malformed(line, LogFault::NotUtf8))?;
        let event: serde_json::Value =
            serde_json::from_str(text).map_err(|_| malformed(line, LogFault::NotJson))?;
        let object = event
            .as_object()
            .ok_or_else(|| malformed(line, LogFault::NotAnObject))?;
        let seq = object.get("seq").and_then(serde_json::Value::as_u64);
        let kind = object.get("kind").and_then(serde_json::Value::as_str);
        let (Some(seq), Some(_)) = (seq, kind) else {
            return Err(malformed(line, LogFault::MissingEnvelope));
        };
        if last.is_some_and(|(_, last_seq)| seq <= last_seq) {
            return Err(malformed(line, LogFault::SequenceNotIncreasing));
        }
        last = Some((line, seq));
        keep(event);
    }
    Ok(last)
}

/// [`read_events`] filtered to the events whose `kind` is `kind`, with the same refusals.
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
        assert_io_error(result.expect_err("append succeeded"), expected, at);
    }

    fn assert_io_error(error: SessionLogError, expected: LogStep, at: &Path) {
        match error {
            SessionLogError::Io { path, step, .. } => {
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

    fn synced_up_from(directory: &Path) -> Vec<Call> {
        directory
            .ancestors()
            .map(|each| Call::SyncDir(each.to_path_buf()))
            .collect()
    }

    fn assert_locked(result: Result<SessionLog, SessionLogError>, at: &Path) {
        match result {
            Err(SessionLogError::Locked { path }) => assert_eq!(path, at),
            Err(other) => panic!("expected Locked, got {other:?}"),
            Ok(_) => panic!("a second writer opened {at:?}"),
        }
    }

    fn seqs_on_disk(path: &Path) -> Vec<u64> {
        read_events(path)
            .unwrap()
            .iter()
            .map(|event| event["seq"].as_u64().unwrap())
            .collect()
    }

    fn assert_malformed(
        error: SessionLogError,
        at: &Path,
        expected: (usize, LogFault),
        case: &str,
    ) {
        match error {
            SessionLogError::Malformed { path, line, fault } => {
                assert_eq!((line, fault), expected, "{case}");
                assert_eq!(path, at, "{case}");
            }
            other => panic!("{case}: expected Malformed {expected:?}, got {other:?}"),
        }
    }

    fn open_error(path: PathBuf, store: &dyn LogStore) -> SessionLogError {
        match SessionLog::open_with(path, store) {
            Ok(_) => panic!("open succeeded"),
            Err(error) => error,
        }
    }

    #[test]
    fn events_are_appended_with_monotonic_seq_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::open(dir.path().join(LOG)).unwrap();
        log.append("plugin_attach", json!({ "id": "github-pr" }))
            .unwrap();
        log.append("tool_invoke", json!({ "name": "pr.read" }))
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
        log.append("turn", json!({ "kind": "spoofed", "extra": 1 }))
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
        log.append("a", json!({})).unwrap();
        log.append("b", json!({})).unwrap();
        log.append("a", json!({})).unwrap();
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
        assert_eq!(log.append("a", json!({})).unwrap(), 1);
        assert_eq!(log.append("b", json!({})).unwrap(), 2);
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
        let mut expected = vec![
            Call::CreateDir(root.join("a")),
            Call::CreateDir(root.join("a").join("b")),
            Call::Open(path.clone()),
            Call::SyncFile,
        ];
        expected.extend(synced_up_from(&root.join("a").join("b")));
        assert_eq!(store.calls(), expected);
        assert!(path.is_file());
    }

    #[test]
    fn a_failed_parent_sync_refuses_open() {
        let positions = FaultLogStore::new().root().ancestors().count() + 2;
        for nth in 1..=positions {
            let store = FaultLogStore::new().fail(Step::SyncDir, nth);
            let synced = synced_up_from(&store.root().join("a").join("b"));
            let Call::SyncDir(directory) = &synced[nth - 1] else {
                unreachable!()
            };
            let error = open_error(store.root().join("a").join("b").join(LOG), &store);
            assert_io_error(error, LogStep::SyncDirectory, directory);
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
            assert_io_error(open_error(path, &store), expected, &at);
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
        assert_io_error(
            open_error(planted.clone(), &OsLogStore),
            LogStep::Open,
            &planted,
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"victim\n");

        let missing = dir.path().join("missing");
        let dangling = dir.path().join("dangling.ndjson");
        std::os::unix::fs::symlink(&missing, &dangling).unwrap();
        assert_io_error(
            open_error(dangling.clone(), &OsLogStore),
            LogStep::Open,
            &dangling,
        );
        assert!(
            !missing.exists(),
            "a dangling symlink's target is never created"
        );
    }

    #[test]
    fn reopen_continues_after_the_last_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG);
        let log = SessionLog::open(path.clone()).unwrap();
        assert_eq!(log.append("a", json!({})).unwrap(), 1);
        assert_eq!(log.append("b", json!({})).unwrap(), 2);
        drop(log);
        let log = SessionLog::open(path.clone()).unwrap();
        assert_eq!(log.path(), path);
        assert_eq!(log.append("c", json!({})).unwrap(), 3);
        let kinds: Vec<String> = read_events(&path)
            .unwrap()
            .iter()
            .map(|event| event["kind"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(kinds, ["a", "b", "c"]);
        assert_eq!(seqs_on_disk(&path), [1, 2, 3]);
    }

    #[test]
    fn a_failed_append_does_not_consume_a_sequence_number() {
        let store = FaultLogStore::new();
        let path = store.root().join(LOG);
        let log = open_in(&store);
        assert_eq!(log.append("a", json!({})).unwrap(), 1);
        let nth = store.count(Step::Write) + 1;
        let store = store.fail(Step::Write, nth);
        assert_io(log.append("b", json!({})), LogStep::Write, &path);
        drop(log);
        let log = SessionLog::open(path.clone()).unwrap();
        assert_eq!(log.append("b", json!({})).unwrap(), 2);
        assert_eq!(seqs_on_disk(&path), [1, 2]);
        drop(store);
    }

    #[test]
    fn reopen_refuses_a_malformed_log_without_changing_it() {
        let valid: &[u8] = b"{\"ts_ms\":1,\"seq\":1,\"kind\":\"a\"}\n";
        let cases: [(&str, &[u8], LogFault); 14] = [
            ("torn line", b"{\"seq\":2,\"kind", LogFault::MissingNewline),
            (
                "complete JSON without LF",
                b"{\"seq\":2,\"kind\":\"a\"}",
                LogFault::MissingNewline,
            ),
            ("invalid UTF-8", b"\xff\xfe\n", LogFault::NotUtf8),
            ("not JSON", b"not json\n", LogFault::NotJson),
            ("empty line", b"\n", LogFault::NotJson),
            ("array", b"[1]\n", LogFault::NotAnObject),
            ("no seq", b"{\"kind\":\"a\"}\n", LogFault::MissingEnvelope),
            (
                "seq as a string",
                b"{\"seq\":\"2\",\"kind\":\"a\"}\n",
                LogFault::MissingEnvelope,
            ),
            (
                "negative seq",
                b"{\"seq\":-2,\"kind\":\"a\"}\n",
                LogFault::MissingEnvelope,
            ),
            ("no kind", b"{\"seq\":2}\n", LogFault::MissingEnvelope),
            (
                "kind as a number",
                b"{\"seq\":2,\"kind\":7}\n",
                LogFault::MissingEnvelope,
            ),
            (
                "repeated seq",
                b"{\"seq\":1,\"kind\":\"a\"}\n",
                LogFault::SequenceNotIncreasing,
            ),
            (
                "decreasing seq",
                b"{\"seq\":0,\"kind\":\"a\"}\n",
                LogFault::SequenceNotIncreasing,
            ),
            (
                "bad line before a good one",
                b"not json\n{\"seq\":3,\"kind\":\"a\"}\n",
                LogFault::NotJson,
            ),
        ];
        for (case, suffix, fault) in cases {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(LOG);
            let bytes = [valid, suffix].concat();
            std::fs::write(&path, &bytes).unwrap();
            assert_malformed(
                open_error(path.clone(), &OsLogStore),
                &path,
                (2, fault),
                case,
            );
            match read_events(&path) {
                Err(error) => assert_malformed(error, &path, (2, fault), case),
                Ok(events) => panic!("{case}: read accepted {events:?}"),
            }
            assert_eq!(
                std::fs::read(&path).unwrap(),
                bytes,
                "{case}: bytes changed"
            );
        }
    }

    #[test]
    fn a_torn_write_leaves_a_log_that_reopen_refuses() {
        let store = FaultLogStore::new().tear(1, 5);
        let path = store.root().join(LOG);
        let log = open_in(&store);
        assert_io(log.append("force", json!({})), LogStep::Write, &path);
        assert_poisoned(log.append("force", json!({})));
        drop(log);
        let torn = std::fs::read(&path).unwrap();
        assert_eq!(torn.len(), 5);
        assert_malformed(
            open_error(path.clone(), &OsLogStore),
            &path,
            (1, LogFault::MissingNewline),
            "torn first line",
        );
        assert_eq!(std::fs::read(&path).unwrap(), torn);
    }

    #[test]
    fn a_complete_uncertain_append_may_be_asserted_again() {
        let (store, log) = open_then_fail(Step::SyncFile);
        let path = store.root().join(LOG);
        let assertion = json!({ "cell": "c1", "generation": 4, "operator": "op", "reason": "r" });
        assert_io(
            log.append("force", assertion.clone()),
            LogStep::SyncFile,
            &path,
        );
        drop(log);
        let log = SessionLog::open(path.clone()).unwrap();
        assert_eq!(log.append("force", assertion.clone()).unwrap(), 2);
        let forces = events_of_kind(&path, "force").unwrap();
        assert_eq!(forces.len(), 2, "{forces:?}");
        for (event, seq) in forces.iter().zip([1, 2]) {
            assert_eq!(event["seq"], seq);
            for field in ["cell", "generation", "operator", "reason"] {
                assert_eq!(event[field], assertion[field], "{field}");
            }
        }
    }

    #[test]
    fn read_events_refuses_a_torn_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG);
        std::fs::write(&path, b"{\"seq\":1,\"kind\":\"a\"}\n{\"seq\":2").unwrap();
        match read_events(&path) {
            Err(error) => assert_malformed(error, &path, (2, LogFault::MissingNewline), "torn"),
            Ok(events) => panic!("read accepted {events:?}"),
        }
    }

    #[test]
    fn open_refuses_a_log_whose_sequence_cannot_advance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG);
        let bytes = format!(
            "{{\"seq\":1,\"kind\":\"a\"}}\n{{\"seq\":{},\"kind\":\"a\"}}\n",
            u64::MAX
        );
        std::fs::write(&path, &bytes).unwrap();
        assert_malformed(
            open_error(path.clone(), &OsLogStore),
            &path,
            (2, LogFault::SequenceExhausted),
            "seq u64::MAX",
        );
        assert_eq!(seqs_on_disk(&path), [1, u64::MAX]);
        assert_eq!(std::fs::read(&path).unwrap(), bytes.as_bytes());
    }

    #[test]
    fn an_append_numbered_u64_max_is_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG);
        std::fs::write(
            &path,
            format!("{{\"seq\":{},\"kind\":\"a\"}}\n", u64::MAX - 1),
        )
        .unwrap();
        let log = SessionLog::open(path.clone()).unwrap();
        assert_eq!(log.append("b", json!({})).unwrap(), u64::MAX);
        assert_poisoned(log.append("c", json!({})));
        assert_eq!(seqs_on_disk(&path), [u64::MAX - 1, u64::MAX]);
        assert_malformed(
            open_error(path.clone(), &OsLogStore),
            &path,
            (2, LogFault::SequenceExhausted),
            "the exhausted writer released its lock",
        );
    }

    #[test]
    fn errors_name_the_path_and_the_step_or_line() {
        use std::error::Error;
        let path = PathBuf::from("/logs/session.ndjson");
        let io = SessionLogError::Io {
            path: path.clone(),
            step: LogStep::SyncFile,
            source: std::io::Error::other("disk gone"),
        };
        assert_eq!(
            io.to_string(),
            "session log /logs/session.ndjson: sync file failed: disk gone"
        );
        assert_eq!(io.source().unwrap().to_string(), "disk gone");
        let malformed = SessionLogError::Malformed {
            path: path.clone(),
            line: 7,
            fault: LogFault::MissingNewline,
        };
        assert_eq!(
            malformed.to_string(),
            "session log /logs/session.ndjson line 7: no final newline"
        );
        assert!(malformed.source().is_none());
        let poisoned = SessionLogError::Poisoned { path: path.clone() };
        assert_eq!(
            poisoned.to_string(),
            "session log /logs/session.ndjson refuses appends after an earlier failure; reopen it"
        );
        assert!(poisoned.source().is_none());
        let locked = SessionLogError::Locked { path };
        assert_eq!(
            locked.to_string(),
            "session log /logs/session.ndjson is held by another writer"
        );
        assert!(locked.source().is_none());
        let steps = [
            LogStep::CreateDirectory,
            LogStep::SyncDirectory,
            LogStep::Open,
            LogStep::Write,
            LogStep::Flush,
            LogStep::SyncFile,
            LogStep::Read,
        ]
        .map(|step| step.to_string());
        let faults = [
            LogFault::MissingNewline,
            LogFault::NotUtf8,
            LogFault::NotJson,
            LogFault::NotAnObject,
            LogFault::MissingEnvelope,
            LogFault::SequenceNotIncreasing,
            LogFault::SequenceExhausted,
        ]
        .map(|fault| fault.to_string());
        for names in [&steps[..], &faults[..]] {
            let distinct: std::collections::BTreeSet<&String> = names.iter().collect();
            assert_eq!(distinct.len(), names.len(), "{names:?}");
            assert!(names.iter().all(|name| !name.is_empty()), "{names:?}");
        }
    }

    #[test]
    fn os_refusals_are_returned_with_their_step() {
        let dir = tempfile::tempdir().unwrap();
        match read_events(dir.path()) {
            Err(error) => assert_io_error(error, LogStep::Read, dir.path()),
            Ok(events) => panic!("read a directory as {events:?}"),
        }
        let unnamable = dir.path().join("n".repeat(1024));
        assert_io_error(
            open_error(unnamable.clone(), &OsLogStore),
            LogStep::Open,
            &unnamable,
        );
    }

    #[test]
    fn a_payload_that_is_not_an_object_adds_no_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG);
        let log = SessionLog::open(path.clone()).unwrap();
        log.append("a", json!(["not", "an", "object"])).unwrap();
        let events = read_events(&path).unwrap();
        let mut keys: Vec<&String> = events[0].as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["kind", "seq", "ts_ms"]);
    }

    #[test]
    fn a_second_writer_is_refused_while_the_first_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG);
        let first = SessionLog::open(path.clone()).unwrap();
        assert_locked(SessionLog::open(path.clone()), &path);
        assert_eq!(first.append("a", json!({})).unwrap(), 1);
        assert_locked(SessionLog::open(path.clone()), &path);
        drop(first);
        let second = SessionLog::open(path.clone()).unwrap();
        assert_eq!(second.append("b", json!({})).unwrap(), 2);
        assert_eq!(seqs_on_disk(&path), [1, 2]);
    }

    #[test]
    fn a_torn_line_cannot_be_extended_by_a_second_writer() {
        let store = FaultLogStore::new().tear(1, 5);
        let path = store.root().join(LOG);
        let first = open_in(&store);
        assert_locked(SessionLog::open(path.clone()), &path);
        assert_io(
            first.append("force", json!({ "cell": "a" })),
            LogStep::Write,
            &path,
        );
        let torn = std::fs::read(&path).unwrap();
        assert_eq!(torn.len(), 5);
        assert_malformed(
            open_error(path.clone(), &OsLogStore),
            &path,
            (1, LogFault::MissingNewline),
            "a second writer after a torn line",
        );
        assert_poisoned(first.append("force", json!({ "cell": "a" })));
        assert_eq!(std::fs::read(&path).unwrap(), torn);
    }

    #[test]
    fn a_poisoned_writer_releases_its_lock() {
        fn after_open(store: FaultLogStore, step: Step) -> FaultLogStore {
            let nth = store.count(step) + 1;
            store.fail(step, nth)
        }
        type Arm = fn(FaultLogStore) -> FaultLogStore;
        let cases: [(&str, Arm, u64); 4] = [
            ("failed write", |store| after_open(store, Step::Write), 1),
            ("failed flush", |store| after_open(store, Step::Flush), 2),
            (
                "failed file sync",
                |store| after_open(store, Step::SyncFile),
                2,
            ),
            (
                "panic",
                |store| {
                    let nth = store.count(Step::Write) + 1;
                    store.panic_on_write(nth)
                },
                1,
            ),
        ];
        for (case, arm, next) in cases {
            let store = FaultLogStore::new();
            let path = store.root().join(LOG);
            let first = open_in(&store);
            let store = arm(store);
            let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                first.append("force", json!({ "cell": "a" }))
            }));
            assert!(!matches!(failed, Ok(Ok(_))), "{case}: the append failed");
            let second = SessionLog::open(path.clone())
                .unwrap_or_else(|error| panic!("{case}: reopen while poisoned: {error}"));
            assert_eq!(
                second.append("force", json!({ "cell": "a" })).unwrap(),
                next,
                "{case}"
            );
            assert_poisoned(first.append("force", json!({ "cell": "a" })));
            drop(store);
        }
    }

    #[test]
    fn a_reopen_syncs_the_file_and_every_ancestor() {
        let store = FaultLogStore::new();
        let path = store.root().join(LOG);
        drop(open_in(&store));
        let before = store.calls().len();
        drop(open_in(&store));
        let mut expected = vec![Call::Open(path), Call::SyncFile];
        expected.extend(synced_up_from(store.root()));
        assert_eq!(store.calls()[before..], expected);
    }

    #[test]
    fn a_reopen_after_an_interrupted_open_syncs_the_directories_it_created() {
        let store = FaultLogStore::new().fail(Step::SyncDir, 1);
        let directory = store.root().join("a").join("b");
        let path = directory.join(LOG);
        match open_error(path.clone(), &store) {
            SessionLogError::Io { step, .. } => assert_eq!(step, LogStep::SyncDirectory),
            other => panic!("expected the first open to fail a directory sync, got {other:?}"),
        }
        let before = store.calls().len();
        let log = SessionLog::open_with(path, &store).unwrap();
        let synced: Vec<Call> = store.calls()[before..]
            .iter()
            .filter(|call| matches!(call, Call::SyncDir(_)))
            .cloned()
            .collect();
        assert_eq!(synced, synced_up_from(&directory));
        assert_eq!(log.append("a", json!({})).unwrap(), 1);
    }

    #[test]
    fn read_events_refuses_a_symlinked_log() {
        let dir = tempfile::tempdir().unwrap();
        let forged = dir.path().join("forged");
        std::fs::write(
            &forged,
            b"{\"seq\":1,\"kind\":\"force\",\"cell\":\"forged\"}\n",
        )
        .unwrap();
        let planted = dir.path().join(LOG);
        std::os::unix::fs::symlink(&forged, &planted).unwrap();
        match read_events(&planted) {
            Err(error) => assert_io_error(error, LogStep::Read, &planted),
            Ok(events) => panic!("read through a symlink: {events:?}"),
        }
        match events_of_kind(&planted, "force") {
            Err(error) => assert_io_error(error, LogStep::Read, &planted),
            Ok(events) => panic!("read through a symlink: {events:?}"),
        }
        let dangling = dir.path().join("dangling.ndjson");
        std::os::unix::fs::symlink(dir.path().join("missing"), &dangling).unwrap();
        match read_events(&dangling) {
            Err(error) => assert_io_error(error, LogStep::Read, &dangling),
            Ok(events) => panic!("a dangling symlink read as {events:?}"),
        }
    }

    #[test]
    fn a_failed_read_of_an_existing_log_refuses_open_and_releases_the_lock() {
        let store = FaultLogStore::new().fail(Step::Read, 1);
        let path = store.root().join(LOG);
        std::fs::write(&path, b"{\"seq\":4,\"kind\":\"k\"}\n").unwrap();
        assert_io_error(open_error(path.clone(), &store), LogStep::Read, &path);
        assert_eq!(store.count(Step::Read), 1);
        let log = SessionLog::open_with(path, &store).unwrap();
        assert_eq!(log.append("k", json!({})).unwrap(), 5);
    }

    fn mkfifo(path: &Path) {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        let made = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
        assert_eq!(
            made,
            0,
            "mkfifo {path:?}: {}",
            std::io::Error::last_os_error()
        );
    }

    #[test]
    fn open_refuses_a_log_that_is_not_a_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG);
        mkfifo(&path);
        let (sender, receiver) = std::sync::mpsc::channel();
        let opening = path.clone();
        std::thread::spawn(move || sender.send(SessionLog::open(opening).err()));
        match receiver.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Some(SessionLogError::Io {
                path: at,
                step,
                source,
            })) => {
                assert_eq!(step, LogStep::Open);
                assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput);
                assert_eq!(at, path);
            }
            Ok(other) => panic!("expected an Io error at Open, got {other:?}"),
            Err(timeout) => panic!("open did not return: {timeout:?}"),
        }
    }

    #[test]
    fn read_events_refuses_a_fifo_without_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG);
        mkfifo(&path);
        let (sender, receiver) = std::sync::mpsc::channel();
        let reading = path.clone();
        std::thread::spawn(move || sender.send(read_events(&reading).err()));
        match receiver.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Some(SessionLogError::Io {
                path: at,
                step,
                source,
            })) => {
                assert_eq!(step, LogStep::Read);
                assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput);
                assert_eq!(at, path);
            }
            Ok(other) => panic!("expected an Io error at Read, got {other:?}"),
            Err(timeout) => panic!("read_events did not return: {timeout:?}"),
        }
    }

    #[test]
    fn an_unreadable_ancestor_refuses_open_at_its_sync() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            let reason = "skipped an_unreadable_ancestor_refuses_open_at_its_sync: root opens a \
                          directory whatever its mode\n";
            std::io::Write::write_all(&mut std::io::stderr(), reason.as_bytes()).unwrap();
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o300)).unwrap();
        let opened = SessionLog::open(locked.join("inner").join(LOG));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        match opened {
            Err(SessionLogError::Io { path, step, source }) => {
                assert_eq!(step, LogStep::SyncDirectory);
                assert_eq!(path, locked);
                assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
            }
            Err(other) => panic!("expected an Io error at SyncDirectory, got {other:?}"),
            Ok(_) => panic!("opened a log under an ancestor this process cannot read"),
        }
    }

    #[test]
    fn an_opened_log_blocks_once_it_is_known_to_be_a_regular_file() {
        use std::os::fd::AsRawFd;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG);
        std::fs::write(&path, b"").unwrap();
        let mut writer = std::fs::OpenOptions::new();
        writer.read(true).append(true);
        let mut reader = std::fs::OpenOptions::new();
        reader.read(true);
        for (case, options) in [("writer", &mut writer), ("reader", &mut reader)] {
            let file = open_regular(options, &path).unwrap();
            let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
            assert_ne!(flags, -1, "{case}: {}", std::io::Error::last_os_error());
            assert_eq!(
                flags & libc::O_NONBLOCK,
                0,
                "{case} descriptor is nonblocking"
            );
        }
    }
}
