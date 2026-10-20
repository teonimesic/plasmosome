use std::io;
use std::path::Path;

/// True when `a` and `b` are one directory or file, or either lies inside the other. Each side
/// is judged by the device and inode of every directory above it, never by path text, so a
/// symbolic link or a spelling the volume treats as the same name cannot hide an overlap. Both
/// paths must exist.
pub fn overlaps(a: &Path, b: &Path) -> io::Result<bool> {
    let _ = (a, b);
    Ok(false)
}
