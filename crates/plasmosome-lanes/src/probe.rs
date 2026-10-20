use std::fs::{self, FileType, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;

use serde_json::{Value, json};

const NAMES: &[(i32, &str)] = &[
    (libc::EPERM, "EPERM"),
    (libc::ENOENT, "ENOENT"),
    (libc::ESRCH, "ESRCH"),
    (libc::EINTR, "EINTR"),
    (libc::EIO, "EIO"),
    (libc::ENXIO, "ENXIO"),
    (libc::E2BIG, "E2BIG"),
    (libc::ENOEXEC, "ENOEXEC"),
    (libc::EBADF, "EBADF"),
    (libc::ECHILD, "ECHILD"),
    (libc::EAGAIN, "EAGAIN"),
    (libc::ENOMEM, "ENOMEM"),
    (libc::EACCES, "EACCES"),
    (libc::EFAULT, "EFAULT"),
    (libc::EBUSY, "EBUSY"),
    (libc::EEXIST, "EEXIST"),
    (libc::EXDEV, "EXDEV"),
    (libc::ENODEV, "ENODEV"),
    (libc::ENOTDIR, "ENOTDIR"),
    (libc::EISDIR, "EISDIR"),
    (libc::EINVAL, "EINVAL"),
    (libc::ENFILE, "ENFILE"),
    (libc::EMFILE, "EMFILE"),
    (libc::ENOTTY, "ENOTTY"),
    (libc::ETXTBSY, "ETXTBSY"),
    (libc::EFBIG, "EFBIG"),
    (libc::ENOSPC, "ENOSPC"),
    (libc::ESPIPE, "ESPIPE"),
    (libc::EROFS, "EROFS"),
    (libc::EMLINK, "EMLINK"),
    (libc::EPIPE, "EPIPE"),
    (libc::ERANGE, "ERANGE"),
    (libc::EDEADLK, "EDEADLK"),
    (libc::ENAMETOOLONG, "ENAMETOOLONG"),
    (libc::ENOLCK, "ENOLCK"),
    (libc::ENOSYS, "ENOSYS"),
    (libc::ENOTEMPTY, "ENOTEMPTY"),
    (libc::ELOOP, "ELOOP"),
    (libc::ENODATA, "ENODATA"),
    (libc::EOVERFLOW, "EOVERFLOW"),
    (libc::ENOTSOCK, "ENOTSOCK"),
    (libc::EOPNOTSUPP, "EOPNOTSUPP"),
    (libc::ENOTSUP, "ENOTSUP"),
    (libc::EAFNOSUPPORT, "EAFNOSUPPORT"),
    (libc::EADDRINUSE, "EADDRINUSE"),
    (libc::ECONNRESET, "ECONNRESET"),
    (libc::ENOTCONN, "ENOTCONN"),
    (libc::ETIMEDOUT, "ETIMEDOUT"),
    (libc::ECONNREFUSED, "ECONNREFUSED"),
    (libc::ESTALE, "ESTALE"),
];

/// The symbolic name of an errno value on this platform, such as `ENOENT`.
pub fn errno_name(code: i32) -> String {
    NAMES
        .iter()
        .find(|(value, _)| *value == code)
        .map_or_else(|| format!("errno-{code}"), |(_, name)| name.to_string())
}

/// The refusal line for a probe whose operation failed with `error`.
pub fn failure(probe: &str, error: &io::Error) -> Value {
    match error.raw_os_error() {
        Some(code) => json!({"probe": probe, "ok": false, "errno": errno_name(code)}),
        None => json!({"probe": probe, "ok": false, "error": error.to_string()}),
    }
}

fn kind(file_type: FileType) -> &'static str {
    if file_type.is_symlink() {
        "symlink"
    } else if file_type.is_dir() {
        "dir"
    } else if file_type.is_file() {
        "file"
    } else if file_type.is_char_device() {
        "char"
    } else if file_type.is_block_device() {
        "block"
    } else if file_type.is_fifo() {
        "fifo"
    } else {
        "socket"
    }
}

fn line(probe: &str, result: io::Result<Value>) -> Value {
    match result {
        Ok(Value::Object(mut fields)) => {
            let mut out = serde_json::Map::new();
            out.insert("probe".to_string(), json!(probe));
            out.insert("ok".to_string(), json!(true));
            out.append(&mut fields);
            Value::Object(out)
        }
        Ok(other) => json!({"probe": probe, "ok": true, "value": other}),
        Err(error) => failure(probe, &error),
    }
}

/// `stat` of `path`, following symlinks: owner, mode, kind, size, device and inode.
pub fn stat(path: &Path) -> Value {
    line(
        "stat",
        fs::metadata(path).map(|metadata| {
            json!({
                "kind": kind(metadata.file_type()),
                "mode": format!("{:04o}", metadata.mode() & 0o7777),
                "uid": metadata.uid(),
                "gid": metadata.gid(),
                "size": metadata.len(),
                "nlink": metadata.nlink(),
                "dev": metadata.dev(),
                "ino": metadata.ino(),
            })
        }),
    )
}

const PAYLOAD: &[u8] = b"lane-probe\n";

/// Open the existing file at `path` for writing and write one line to its end.
pub fn write(path: &Path) -> Value {
    line(
        "write",
        OpenOptions::new()
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(PAYLOAD))
            .map(|()| json!({"bytes": PAYLOAD.len()})),
    )
}

/// Create `path` exclusively and write one line to it.
pub fn create(path: &Path) -> Value {
    line(
        "create",
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .and_then(|mut file| file.write_all(PAYLOAD))
            .map(|()| json!({"bytes": PAYLOAD.len()})),
    )
}

/// The sorted names in the directory at `path`.
pub fn list(path: &Path) -> Value {
    let names = fs::read_dir(path).and_then(|entries| {
        let mut names = entries
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().to_string()))
            .collect::<io::Result<Vec<_>>>()?;
        names.sort();
        Ok(json!({"names": names}))
    });
    line("list", names)
}

/// Resolve `path` without following a final symlink.
pub fn lookup(path: &Path) -> Value {
    line(
        "lookup",
        fs::symlink_metadata(path).map(|metadata| json!({"kind": kind(metadata.file_type())})),
    )
}

fn status_field<'a>(status: &'a str, key: &str) -> Option<Vec<&'a str>> {
    status
        .lines()
        .find_map(|row| row.strip_prefix(key))
        .map(|rest| rest.split_whitespace().collect())
}

fn ids(values: &[&str]) -> Option<(u32, u32)> {
    let real = values.first()?.parse().ok()?;
    let effective = values.get(1)?.parse().ok()?;
    Some((real, effective))
}

/// The ids in a `/proc/<pid>/status` text.
pub fn id_from_status(status: &str) -> Value {
    let uids = status_field(status, "Uid:").as_deref().and_then(ids);
    let gids = status_field(status, "Gid:").as_deref().and_then(ids);
    let groups: Option<Vec<u32>> = status_field(status, "Groups:")
        .unwrap_or_default()
        .iter()
        .map(|group| group.parse().ok())
        .collect();
    match (uids, gids, groups) {
        (Some((uid, euid)), Some((gid, egid)), Some(groups)) => json!({
            "probe": "id", "ok": true, "uid": uid, "euid": euid, "gid": gid, "egid": egid,
            "groups": groups,
        }),
        _ => {
            json!({"probe": "id", "ok": false, "error": "status has no readable Uid and Gid lines"})
        }
    }
}

/// The filesystem names in a `/proc/filesystems` text.
pub fn filesystems_from(text: &str) -> Value {
    let names: Vec<&str> = text
        .lines()
        .filter_map(|row| row.split('\t').next_back())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    json!({"probe": "filesystems", "ok": true, "names": names})
}

fn mount(row: &str) -> Option<Value> {
    let fields: Vec<&str> = row.split(' ').collect();
    let separator = fields.iter().position(|field| *field == "-")?;
    Some(json!({
        "mount_point": fields.get(4)?,
        "fstype": fields.get(separator + 1)?,
        "source": fields.get(separator + 2)?,
        "mount_options": fields.get(5)?,
        "super_options": fields.get(separator + 3)?,
    }))
}

/// The mounts in a `/proc/<pid>/mountinfo` text: mount point, filesystem type, source and
/// options of each.
pub fn mountinfo_from(text: &str) -> Value {
    let mounts: Option<Vec<Value>> = text
        .lines()
        .filter(|row| !row.is_empty())
        .map(mount)
        .collect();
    match mounts {
        Some(mounts) => json!({"probe": "mountinfo", "ok": true, "mounts": mounts}),
        None => {
            json!({"probe": "mountinfo", "ok": false, "error": "a mountinfo line has no `-` field"})
        }
    }
}
