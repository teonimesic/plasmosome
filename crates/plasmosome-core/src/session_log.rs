use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub enum SessionLogError {
    Io(std::io::Error),
}

impl std::fmt::Display for SessionLogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionLogError::Io(e) => write!(f, "session log io error: {e}"),
        }
    }
}

impl std::error::Error for SessionLogError {}

pub struct SessionLog {
    path: PathBuf,
    next_seq: AtomicU64,
    file: Mutex<std::fs::File>,
}

impl SessionLog {
    pub fn create(path: PathBuf) -> Result<SessionLog, SessionLogError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(SessionLogError::Io)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(SessionLogError::Io)?;
        Ok(SessionLog {
            path,
            next_seq: AtomicU64::new(1),
            file: Mutex::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, kind: &str, payload: serde_json::Value) -> u64 {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
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
        let mut file = self
            .file
            .lock()
            .expect("session log file lock is never poisoned while held");
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
        seq
    }
}

pub fn read_events(path: &Path) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

pub fn events_of_kind(path: &Path, kind: &str) -> Vec<serde_json::Value> {
    read_events(path)
        .into_iter()
        .filter(|event| event.get("kind").and_then(serde_json::Value::as_str) == Some(kind))
        .collect()
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

    #[test]
    fn events_are_appended_with_monotonic_seq_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path().join("session.ndjson")).unwrap();
        log.append("plugin_attach", serde_json::json!({ "id": "github-pr" }));
        log.append("tool_invoke", serde_json::json!({ "name": "pr.read" }));
        let events = read_events(&dir.path().join("session.ndjson"));
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
        let log = SessionLog::create(dir.path().join("session.ndjson")).unwrap();
        log.append("turn", serde_json::json!({ "kind": "spoofed", "extra": 1 }));
        let events = read_events(&dir.path().join("session.ndjson"));
        assert_eq!(
            events[0]["kind"], "turn",
            "the envelope owns the kind field"
        );
        assert_eq!(events[0]["extra"], 1);
    }

    #[test]
    fn events_of_kind_filters_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path().join("session.ndjson")).unwrap();
        log.append("a", serde_json::json!({}));
        log.append("b", serde_json::json!({}));
        log.append("a", serde_json::json!({}));
        let a_events = events_of_kind(&dir.path().join("session.ndjson"), "a");
        assert_eq!(a_events.len(), 2);
    }

    #[test]
    fn reading_a_missing_log_yields_no_events() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_events(&dir.path().join("missing.ndjson")).is_empty());
    }
}
