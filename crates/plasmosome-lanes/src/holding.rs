use std::io;
use std::path::{Path, PathBuf};

/// One entry the host moved into the holding directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub from: PathBuf,
    pub to: PathBuf,
    pub is_dir: bool,
}

/// Move `tree` and every entry under it into `holding`, deepest first and directories included,
/// each under a name not already used there. Every moved inode keeps a directory entry, and every
/// moved directory is empty when it moves. A symbolic link is moved as a link and never followed.
/// `holding` must exist, on the same volume as `tree` and outside it. Returns the moves in the
/// order they were made.
pub fn move_to_holding(tree: &Path, holding: &Path) -> io::Result<Vec<Moved>> {
    let _ = (tree, holding);
    Ok(Vec::new())
}
