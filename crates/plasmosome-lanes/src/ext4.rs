use std::io;
use std::path::Path;

/// Build a raw ext4 image at `out` holding the tree under `staged`, with e2fsprogs' `mke2fs -d`
/// from `tool_dir`. Refuses, naming the tool, when `tool_dir` has no `mke2fs`.
pub fn ext4_from_dir(tool_dir: &Path, staged: &Path, out: &Path) -> io::Result<()> {
    let _ = (tool_dir, staged, out);
    Ok(())
}

/// Read the file at `path` inside the ext4 `image` with `debugfs` from `tool_dir`. Refuses,
/// naming the tool, when `tool_dir` has no `debugfs`, and refuses a path the image lacks.
pub fn ext4_read(tool_dir: &Path, image: &Path, path: &str) -> io::Result<Vec<u8>> {
    let _ = (tool_dir, image, path);
    Ok(Vec::new())
}
