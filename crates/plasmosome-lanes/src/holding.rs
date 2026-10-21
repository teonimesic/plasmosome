use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One entry the host moved into the holding directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub from: PathBuf,
    pub to: PathBuf,
    pub is_dir: bool,
}

fn deepest_first(path: &Path, order: &mut Vec<(PathBuf, bool)>) -> io::Result<()> {
    let is_dir = fs::symlink_metadata(path)?.is_dir();
    if is_dir {
        let mut children = fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        children.sort();
        for child in children {
            deepest_first(&child, order)?;
        }
    }
    order.push((path.to_path_buf(), is_dir));
    Ok(())
}

fn unused_name(holding: &Path, next: &mut u64) -> PathBuf {
    loop {
        let candidate = holding.join(format!("{:06}", *next));
        *next += 1;
        if matches!(fs::symlink_metadata(&candidate), Err(error) if error.kind() == io::ErrorKind::NotFound)
        {
            return candidate;
        }
    }
}

/// Move `tree` and every entry under it into `holding`, deepest first and directories included,
/// each under a name not already used there. Every moved inode keeps a directory entry, and every
/// moved directory is empty when it moves. A symbolic link is moved as a link and never followed.
/// `holding` must exist, on the same volume as `tree` and outside it. Returns the moves in the
/// order they were made.
pub fn move_to_holding(tree: &Path, holding: &Path) -> io::Result<Vec<Moved>> {
    let mut order = Vec::new();
    deepest_first(tree, &mut order)?;
    let mut next = 0;
    let mut moved = Vec::with_capacity(order.len());
    for (from, is_dir) in order {
        let to = unused_name(holding, &mut next);
        fs::rename(&from, &to)?;
        moved.push(Moved { from, to, is_dir });
    }
    Ok(moved)
}
