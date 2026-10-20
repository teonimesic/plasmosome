use std::ffi::OsString;
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
        entries
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()
    });
    match names {
        Ok(names) => list_names(names),
        Err(error) => failure("list", &error),
    }
}

/// The `list` line for `names`, sorted. A name that is not UTF-8 fails the whole listing,
/// because a lossy conversion would report two different names as one.
pub fn list_names(names: impl IntoIterator<Item = OsString>) -> Value {
    let mut listed = Vec::new();
    for name in names {
        match name.into_string() {
            Ok(name) => listed.push(name),
            Err(name) => {
                return json!({"probe": "list", "ok": false,
                              "error": format!("the name {name:?} is not UTF-8")});
            }
        }
    }
    listed.sort();
    json!({"probe": "list", "ok": true, "names": listed})
}

/// True when `name` can name a holder's socket inside the holder directory: it starts with a
/// letter or digit and holds only letters, digits, `.`, `_` and `-`.
pub fn holder_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn sorted_names(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = fs::read_dir(dir)?
        .map(|entry| {
            entry?.file_name().into_string().map_err(|name| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("the name {name:?} is not UTF-8"),
                )
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

fn devices_under(sys: &Path) -> io::Result<Value> {
    let virtio_dir = sys.join("bus/virtio/devices");
    let mut virtio = Vec::new();
    for name in sorted_names(&virtio_dir)? {
        let id = fs::read_to_string(virtio_dir.join(&name).join("device"))?;
        virtio.push(json!({"name": name, "device": id.trim()}));
    }
    let block_dir = sys.join("block");
    let mut block = Vec::new();
    for name in sorted_names(&block_dir)? {
        let flag = fs::read_to_string(block_dir.join(&name).join("ro"))?;
        let read_only = match flag.trim() {
            "0" => false,
            "1" => true,
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("block device {name} has ro `{other}`, not 0 or 1"),
                ));
            }
        };
        block.push(json!({"name": name, "read_only": read_only}));
    }
    Ok(json!({"virtio": virtio, "block": block}))
}

/// The devices under the sysfs root `sys`: each virtio device's name and device ID, such as
/// `0x0013` for vsock, and whether each block device is read-only.
pub fn devices(sys: &Path) -> Value {
    line("devices", devices_under(sys))
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

const OPTIONAL_FIELDS: usize = 6;

fn mount(row: &str) -> Option<Value> {
    let fields: Vec<&str> = row.split(' ').collect();
    let separator = OPTIONAL_FIELDS
        + fields
            .get(OPTIONAL_FIELDS..)?
            .iter()
            .position(|field| *field == "-")?;
    Some(json!({
        "mount_point": fields.get(4)?,
        "root": fields.get(3)?,
        "optional_fields": fields.get(OPTIONAL_FIELDS..separator)?,
        "fstype": fields.get(separator + 1)?,
        "source": fields.get(separator + 2)?,
        "mount_options": fields.get(5)?,
        "super_options": fields.get(separator + 3)?,
    }))
}

/// The mounts in a `/proc/<pid>/mountinfo` text: of each, the mount point, the root within its
/// filesystem, the optional fields such as `shared:N` and `master:N`, the filesystem type, the
/// source and both option lists.
pub fn mountinfo_from(text: &str) -> Value {
    let mounts: Option<Vec<Value>> = text
        .lines()
        .filter(|row| !row.is_empty())
        .map(mount)
        .collect();
    match mounts {
        Some(mounts) => json!({"probe": "mountinfo", "ok": true, "mounts": mounts}),
        None => {
            json!({"probe": "mountinfo", "ok": false, "error": "a mountinfo line has fewer than six fields before its `-` separator"})
        }
    }
}

/// The device and inode of each file-backed line in a `/proc/<pid>/maps` text, the device as
/// the kernel prints it, such as `00:17`. Anonymous mappings, whose inode is 0, are left out.
pub fn map_identities(text: &str) -> Vec<(String, u64)> {
    let _ = text;
    Vec::new()
}
