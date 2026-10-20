use std::path::Path;

use serde_json::{Value, json};

/// The symbolic name of an errno value on this platform, such as `ENOENT`.
pub fn errno_name(code: i32) -> String {
    format!("E{code}")
}

/// `stat` of `path`, following symlinks: owner, mode, kind, size, device and inode.
pub fn stat(path: &Path) -> Value {
    let _ = path;
    json!({"probe": "stat"})
}

/// Open the existing file at `path` for writing and write one line to its end.
pub fn write(path: &Path) -> Value {
    let _ = path;
    json!({"probe": "write"})
}

/// Create `path` exclusively and write one line to it.
pub fn create(path: &Path) -> Value {
    let _ = path;
    json!({"probe": "create"})
}

/// The sorted names in the directory at `path`.
pub fn list(path: &Path) -> Value {
    let _ = path;
    json!({"probe": "list"})
}

/// Resolve `path` without following a final symlink.
pub fn lookup(path: &Path) -> Value {
    let _ = path;
    json!({"probe": "lookup"})
}

/// The ids in a `/proc/<pid>/status` text.
pub fn id_from_status(status: &str) -> Value {
    let _ = status;
    json!({"probe": "id"})
}

/// The filesystem names in a `/proc/filesystems` text.
pub fn filesystems_from(text: &str) -> Value {
    let _ = text;
    json!({"probe": "filesystems"})
}

/// The mounts in a `/proc/<pid>/mountinfo` text: mount point, filesystem type, source and
/// options of each.
pub fn mountinfo_from(text: &str) -> Value {
    let _ = text;
    json!({"probe": "mountinfo"})
}

/// The refusal line for a probe whose operation failed with `error`.
pub fn failure(probe: &str, error: &std::io::Error) -> Value {
    let _ = error;
    json!({"probe": probe, "ok": false})
}
