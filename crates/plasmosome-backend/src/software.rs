use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, FileType, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use crate::digest::Digest;

const COMMAND_DIRECTORY: &str = "/usr/local/bin";
const PATH_DIRECTORIES: [&str; 3] = [COMMAND_DIRECTORY, "/usr/bin", "/bin"];

/// A file's path inside a software tree: relative, with `/` between components, never `..`,
/// never absolute and never holding a NUL. Only [`pin`] makes one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TreePath(String);

impl TreePath {
    /// The path as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One file of a pinned tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFile {
    /// Where the file is in the tree.
    pub path: TreePath,
    /// The SHA-256 of the bytes [`pin`] read.
    pub sha256: Digest,
    /// True when the file is one of the tree's commands.
    pub command: bool,
}

/// A software tree whose every file was read once and matched the digest its verified package
/// lists. Only [`pin`] makes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedTree {
    files: Vec<TreeFile>,
    digest: Digest,
}

impl PinnedTree {
    /// Every file of the tree, ordered by the bytes of its path.
    pub fn files(&self) -> &[TreeFile] {
        &self.files
    }

    /// The tree digest. It is the SHA-256 of every file in [`files`](Self::files) order, each
    /// written as its path, a NUL byte, the 64 lowercase hexadecimal digits of its SHA-256, a NUL
    /// byte, and one byte that is 1 for a command and 0 otherwise. Any changed byte, path or
    /// command flag changes it, and the order the package listed files in does not.
    pub fn digest(&self) -> Digest {
        self.digest
    }

    fn assemble(mut files: Vec<TreeFile>) -> PinnedTree {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut encoding = Vec::new();
        for file in &files {
            encoding.extend_from_slice(file.path.0.as_bytes());
            encoding.push(0);
            encoding.extend_from_slice(file.sha256.hex().as_bytes());
            encoding.push(0);
            encoding.push(u8::from(file.command));
        }
        PinnedTree {
            digest: Digest::of(&encoding),
            files,
        }
    }

    fn commands(&self) -> impl Iterator<Item = &str> {
        self.files
            .iter()
            .filter(|file| file.command)
            .map(|file| file.path.as_str())
    }
}

/// A command a software tree provides: one path component that names a file at the top of the
/// tree, placed as `/usr/local/bin/<name>`.
///
/// Spec 027 leaves the declaration field that names commands blocked on consumers, so only the
/// test [`fixture`] makes one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommandName(String);

/// Why a directory cannot be pinned as a software tree. Each names the path inside the tree
/// that it is about; `.` is the tree's root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinRefusal {
    /// The file or directory could not be opened or read.
    Unreadable { path: String },
    /// The entry is neither a regular file nor a directory: a FIFO, a socket or a device.
    NotRegular { path: String },
    /// The entry is a symbolic link, wherever it points.
    Link { path: String },
    /// A listed path is absolute, has a `..` component, or holds a NUL.
    Escapes { path: String },
    /// The file's bytes do not match the digest the package lists for it.
    DigestMismatch { path: String },
    /// The package lists the file and the tree does not have it.
    Missing { path: String },
    /// The tree has a file, or an empty directory, that the package does not list.
    Unexpected { path: String },
    /// A command names no file at the top of the tree.
    CommandNotInTree { name: String },
}

impl PinRefusal {
    /// The path inside the tree the refusal is about: a command's name for
    /// [`CommandNotInTree`](PinRefusal::CommandNotInTree).
    pub fn path(&self) -> &str {
        match self {
            PinRefusal::Unreadable { path }
            | PinRefusal::NotRegular { path }
            | PinRefusal::Link { path }
            | PinRefusal::Escapes { path }
            | PinRefusal::DigestMismatch { path }
            | PinRefusal::Missing { path }
            | PinRefusal::Unexpected { path } => path,
            PinRefusal::CommandNotInTree { name } => name,
        }
    }
}

impl fmt::Display for PinRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PinRefusal::Unreadable { path } => write!(f, "{path:?} cannot be read"),
            PinRefusal::NotRegular { path } => {
                write!(f, "{path:?} is not a regular file or a directory")
            }
            PinRefusal::Link { path } => {
                write!(
                    f,
                    "{path:?} is a symbolic link, and a software tree holds none"
                )
            }
            PinRefusal::Escapes { path } => {
                write!(f, "{path:?} is not a relative path inside the tree")
            }
            PinRefusal::DigestMismatch { path } => write!(
                f,
                "{path:?} does not match the digest the package lists for it"
            ),
            PinRefusal::Missing { path } => {
                write!(
                    f,
                    "{path:?} is listed by the package and absent from the tree"
                )
            }
            PinRefusal::Unexpected { path } => {
                write!(f, "{path:?} is in the tree and not listed by the package")
            }
            PinRefusal::CommandNotInTree { name } => {
                write!(f, "command {name:?} names no file at the top of the tree")
            }
        }
    }
}

impl std::error::Error for PinRefusal {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Listed,
    Opened,
}

/// Pins the software tree in the directory `root` against `expected`, the path and SHA-256 of
/// every file in the plasmid's verified package, and marks each of `commands` as a command.
///
/// Each file is opened once without following a link, checked to be a regular file through that
/// descriptor, and hashed from it in one pass. So a digest is of the bytes read, not of whatever
/// the path names later: a caller that places a file must place those bytes or check them
/// against [`TreeFile::sha256`] again. Nothing is written, and no path outside `root` is opened.
/// The tree must hold exactly the listed files, each a regular file, with no link and no
/// directory the listing does not imply. A refusal names the first path at fault.
pub fn pin(
    root: &Path,
    expected: &[(String, Digest)],
    commands: &[CommandName],
) -> Result<PinnedTree, PinRefusal> {
    pin_observed(root, expected, commands, &mut |_, _| {})
}

fn pin_observed(
    root: &Path,
    expected: &[(String, Digest)],
    commands: &[CommandName],
    observe: &mut dyn FnMut(Step, &Path),
) -> Result<PinnedTree, PinRefusal> {
    let mut listed = BTreeMap::new();
    for (path, sha256) in expected {
        if escapes(path) {
            return Err(PinRefusal::Escapes { path: path.clone() });
        }
        if listed
            .insert(path.as_str(), *sha256)
            .is_some_and(|earlier| earlier != *sha256)
        {
            return Err(PinRefusal::DigestMismatch { path: path.clone() });
        }
    }
    if let Some(command) = commands
        .iter()
        .find(|command| !listed.contains_key(command.0.as_str()))
    {
        return Err(PinRefusal::CommandNotInTree {
            name: command.0.clone(),
        });
    }
    let mut walk = Walk {
        listed: &listed,
        files: Vec::new(),
        observe,
    };
    walk.directory(root, "")?;
    let mut files = walk.files;
    let seen: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
    if let Some(path) = listed.keys().find(|path| !seen.contains(*path)) {
        return Err(PinRefusal::Missing {
            path: path.to_string(),
        });
    }
    for file in &mut files {
        file.command = commands.iter().any(|command| command.0 == file.path.0);
    }
    Ok(PinnedTree::assemble(files))
}

fn escapes(path: &str) -> bool {
    path.starts_with('/') || path.contains('\0') || path.split('/').any(|part| part == "..")
}

#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Directory,
    File,
}

fn entry_kind(kind: FileType, path: &str) -> Result<Entry, PinRefusal> {
    if kind.is_symlink() {
        Err(PinRefusal::Link { path: path.into() })
    } else if kind.is_dir() {
        Ok(Entry::Directory)
    } else if kind.is_file() {
        Ok(Entry::File)
    } else {
        Err(PinRefusal::NotRegular { path: path.into() })
    }
}

struct Walk<'a> {
    listed: &'a BTreeMap<&'a str, Digest>,
    files: Vec<TreeFile>,
    observe: &'a mut dyn FnMut(Step, &Path),
}

impl Walk<'_> {
    fn directory(&mut self, at: &Path, prefix: &str) -> Result<(), PinRefusal> {
        let unreadable = || PinRefusal::Unreadable {
            path: if prefix.is_empty() { "." } else { prefix }.to_string(),
        };
        let mut entries = fs::read_dir(at)
            .map_err(|_| unreadable())?
            .map(|entry| entry.map_err(|_| unreadable()))
            .collect::<Result<Vec<_>, _>>()?;
        if entries.is_empty() && !prefix.is_empty() {
            return Err(PinRefusal::Unexpected {
                path: prefix.to_string(),
            });
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            let text = name.to_string_lossy();
            let path = if prefix.is_empty() {
                text.to_string()
            } else {
                format!("{prefix}/{text}")
            };
            if name.to_str().is_none() {
                return Err(PinRefusal::Unexpected { path });
            }
            let kind = entry
                .file_type()
                .map_err(|_| PinRefusal::Unreadable { path: path.clone() })?;
            match entry_kind(kind, &path)? {
                Entry::Directory => self.directory(&entry.path(), &path)?,
                Entry::File => self.file(&entry.path(), path)?,
            }
        }
        Ok(())
    }

    fn file(&mut self, at: &Path, path: String) -> Result<(), PinRefusal> {
        let Some(&listed) = self.listed.get(path.as_str()) else {
            return Err(PinRefusal::Unexpected { path });
        };
        (self.observe)(Step::Listed, at);
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(at)
            .map_err(|error| {
                if error.raw_os_error() == Some(libc::ELOOP) {
                    PinRefusal::Link { path: path.clone() }
                } else {
                    PinRefusal::Unreadable { path: path.clone() }
                }
            })?;
        match file.metadata() {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(PinRefusal::NotRegular { path }),
            Err(_) => return Err(PinRefusal::Unreadable { path }),
        }
        (self.observe)(Step::Opened, at);
        let sha256 =
            Digest::of_reader(&file).map_err(|_| PinRefusal::Unreadable { path: path.clone() })?;
        if sha256 != listed {
            return Err(PinRefusal::DigestMismatch { path });
        }
        self.files.push(TreeFile {
            path: TreePath(path),
            sha256,
            command: false,
        });
        Ok(())
    }
}

/// A command name that is already taken, found before attach has any effect. It maps to code
/// 102 with `target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The command's path: `/usr/local/bin/<name>`, spelled as the new tree spells it.
    pub target: String,
    /// What already holds the name.
    pub holder: Holder,
}

/// What holds a command name a new tree wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    /// Another attached plasmid, by its ID, provides the name.
    Plasmid(String),
    /// The root image resolves the name in a directory on `PATH`.
    Image,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let target = &self.target;
        match &self.holder {
            Holder::Plasmid(plasmid) => {
                write!(f, "{target:?} is already provided by plasmid {plasmid:?}")
            }
            Holder::Image => write!(
                f,
                "{target:?} would shadow a command the root image resolves on PATH"
            ),
        }
    }
}

impl std::error::Error for Conflict {}

/// Decides spec 027's name refusals for `tree` before attach has any effect.
///
/// `attached` holds each other attached plasmid's ID and command names; a reload leaves out the
/// plasmid it replaces. `image_names` returns the names the root image has in one directory of
/// spec 024's `PATH`, and is asked about `/usr/local/bin`, `/usr/bin` and `/bin`. Pass
/// `case_insensitive` when the volume that holds the names ignores case, and then names that
/// differ only in case collide. A name another plasmid holds is reported before one the image
/// resolves, so when both apply the plasmid is the holder.
pub fn check_names(
    tree: &PinnedTree,
    attached: &[(String, Vec<CommandName>)],
    image_names: impl Fn(&str) -> Vec<String>,
    case_insensitive: bool,
) -> Result<(), Conflict> {
    let same = |held: &str, wanted: &str| {
        if case_insensitive {
            held.to_lowercase() == wanted.to_lowercase()
        } else {
            held == wanted
        }
    };
    let conflict = |name: &str, holder| Conflict {
        target: format!("{COMMAND_DIRECTORY}/{name}"),
        holder,
    };
    for name in tree.commands() {
        if let Some((plasmid, _)) = attached
            .iter()
            .find(|(_, names)| names.iter().any(|held| same(&held.0, name)))
        {
            return Err(conflict(name, Holder::Plasmid(plasmid.clone())));
        }
    }
    let image: Vec<String> = PATH_DIRECTORIES
        .iter()
        .flat_map(|directory| image_names(directory))
        .collect();
    if let Some(name) = tree
        .commands()
        .find(|name| image.iter().any(|held| same(held, name)))
    {
        return Err(conflict(name, Holder::Image));
    }
    Ok(())
}

/// The test-only route to a command, until spec 027's declaration field exists. It compiles only
/// in this crate's tests and under the `test-fixture` feature, which only dev-dependencies enable.
#[cfg(any(test, feature = "test-fixture"))]
pub mod fixture {
    use super::CommandName;

    /// Declares each of `names` as a command. Panics when a name is not exactly one path
    /// component: empty, `.`, `..`, or holding `/` or a NUL.
    pub fn declare_commands(names: &[&str]) -> Vec<CommandName> {
        names
            .iter()
            .map(|name| {
                assert!(
                    !matches!(*name, "" | "." | "..") && !name.contains(['/', '\0']),
                    "{name:?} is not one path component"
                );
                CommandName(name.to_string())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
