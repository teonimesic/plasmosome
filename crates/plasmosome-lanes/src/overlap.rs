use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

type Identity = (u64, u64);

fn identity(path: &Path) -> io::Result<Identity> {
    let metadata = fs::metadata(path)?;
    Ok((metadata.dev(), metadata.ino()))
}

fn chain(path: &Path) -> io::Result<Vec<Identity>> {
    let resolved = fs::canonicalize(path)?;
    resolved.ancestors().map(identity).collect()
}

fn inside(inner: &Path, outer: &Path) -> io::Result<bool> {
    let outer = identity(outer)?;
    Ok(chain(inner)?.contains(&outer))
}

/// True when `a` and `b` are one directory or file, or either lies inside the other. Each side
/// is judged by the device and inode of every directory above its canonical path, never by path
/// text, so a symbolic link or a spelling the volume treats as the same name cannot hide an
/// overlap. The walk follows the canonical path's own parents, not the volume's: on macOS the
/// data volume's root `/System/Volumes/Data` holds `/private/tmp` through a firmlink, yet the
/// canonical parents of `/private/tmp` are `/private` and `/`, so `/System/Volumes/Data` is not
/// judged as overlapping a share under `/private/tmp`. Both paths must exist.
pub fn overlaps(a: &Path, b: &Path) -> io::Result<bool> {
    Ok(inside(a, b)? || inside(b, a)?)
}
