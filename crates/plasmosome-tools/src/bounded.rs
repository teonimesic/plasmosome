use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Reads the whole regular file at `path`, through a limit of `limit` plus one byte.
///
/// The last component of `path` is opened without following a symbolic link and without waiting
/// for a writer, so a link, a FIFO, a device or a directory is refused, not read. A file longer
/// than `limit` bytes is refused. The error names `path` as the file system gives it, so it is
/// one line unless `path` holds a line break. A caller that must follow a link the person named
/// resolves it first, with `std::fs::canonicalize`.
pub fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let shown = path.display();
    let cannot = |error: std::io::Error| format!("cannot read {shown}: {error}");
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(cannot)?;
    if !file.metadata().map_err(cannot)?.is_file() {
        return Err(format!("{shown} is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(cannot)?;
    if bytes.len() as u64 > limit {
        return Err(format!("{shown} is over {limit} bytes"));
    }
    Ok(bytes)
}
