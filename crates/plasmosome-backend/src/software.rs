use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, FileType, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::digest::Digest;

const COMMAND_DIRECTORY: &str = "/usr/local/bin";
const PATH_DIRECTORIES: [&str; 3] = [COMMAND_DIRECTORY, "/usr/bin", "/bin"];

#[cfg(target_os = "macos")]
const NO_LINKS: libc::c_int = libc::O_NOFOLLOW_ANY;
#[cfg(not(target_os = "macos"))]
const NO_LINKS: libc::c_int = libc::O_NOFOLLOW;

/// A file's path inside a software tree, one that spec 020 §2's file path rule accepts: relative,
/// with `/` between nonempty components that are neither `.` nor `..`, and with no backslash, no
/// ASCII control character and no drive prefix such as `C:`. Only [`pin`] makes one.
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

    /// The tree's commands, in [`files`](Self::files) order. Passing them to [`pin`] with the
    /// same listing pins this tree again.
    pub fn command_names(&self) -> Vec<CommandName> {
        self.commands()
            .map(|name| CommandName(name.to_string()))
            .collect()
    }

    fn commands(&self) -> impl Iterator<Item = &str> {
        self.files
            .iter()
            .filter(|file| file.command)
            .map(|file| file.path.as_str())
    }
}

/// One file a verified package lists: spec 020 §2's `{path, digest, size}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageFile {
    /// The file's path inside the tree.
    pub path: String,
    /// The SHA-256 of the file's bytes.
    pub digest: Digest,
    /// The number of bytes in the file.
    pub size: u64,
}

/// A command a software tree provides: one path component that names a file at the top of the
/// tree, placed as `/usr/local/bin/<name>`.
///
/// Spec 027 leaves the declaration field that names commands blocked on consumers, so only the
/// test [`fixture`] makes one from text. [`PinnedTree::command_names`] hands back the ones a
/// tree was pinned with.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommandName(String);

/// Why a directory cannot be pinned as a software tree. Each names the path inside the tree
/// that it is about, relative to the tree's root; `.` is the root itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinRefusal {
    /// The file or directory could not be opened or read.
    Unreadable { path: String },
    /// The entry is neither a regular file nor a directory: a FIFO, a socket or a device.
    NotRegular { path: String },
    /// The entry is a symbolic link, wherever it points. On macOS it is also a file or directory
    /// that pin reached through a link. For the root, `.`, it is also a root replaced between
    /// its check and its resolution.
    Link { path: String },
    /// A listed path breaks spec 020 §2's file path rule; [`TreePath`] says what it allows.
    InvalidPath { path: String },
    /// A listed path repeats an earlier one, also under ASCII case folding, or one of the two is
    /// the other's ancestor.
    Collides { path: String },
    /// The file's size or bytes do not match what the package lists for it. A size that differs
    /// is refused before any byte is read.
    DigestMismatch { path: String },
    /// The package lists the file and the tree does not have it.
    Missing { path: String },
    /// The tree has a file, or an empty directory, that the package does not list. A name that is
    /// not UTF-8 is never listed; its path here is the name's lossy text.
    Unexpected { path: String },
    /// A command names no file at the top of the tree. No product path reaches it today, because
    /// no product tree declares a command; plasmosome-tbru gives it a field and a `fix`.
    CommandNotInTree { name: String },
}

impl PinRefusal {
    /// The path inside the tree the refusal is about, relative to its root: a command's name for
    /// [`CommandNotInTree`](PinRefusal::CommandNotInTree).
    pub fn path(&self) -> &str {
        match self {
            PinRefusal::Unreadable { path }
            | PinRefusal::NotRegular { path }
            | PinRefusal::Link { path }
            | PinRefusal::InvalidPath { path }
            | PinRefusal::Collides { path }
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
            PinRefusal::Link { path } => write!(
                f,
                "{path:?} is a symbolic link or is reached through one, and a software tree holds none"
            ),
            PinRefusal::InvalidPath { path } => write!(
                f,
                "{path:?} is not a relative path of nonempty components other than . and .., \
                 free of backslashes, control characters and a drive prefix"
            ),
            PinRefusal::Collides { path } => write!(
                f,
                "{path:?} is listed twice, also under ASCII case folding, or with a file as its \
                 ancestor or descendant"
            ),
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
    RootChecked,
    Listed,
    Opened,
}

/// Pins the software tree in the directory `root` against `expected`, every file of the
/// plasmid's verified package, and marks each of `commands` as a command.
///
/// Each listed path must pass spec 020 §2's file path rule and be unique, also under ASCII case
/// folding, with no path another's ancestor. Each file is opened once, checked through that
/// descriptor to be a regular file of the listed size, and hashed from it in one pass. So a
/// digest is of the bytes read, not of whatever the path names later: `pin` keeps neither the
/// bytes nor the descriptor, so a caller that places a file must check what it places against
/// [`TreeFile::sha256`] again. Nothing is written. The tree must hold exactly the listed files,
/// each a regular file, with no link and no directory the listing does not imply. A refusal
/// names the first path at fault.
///
/// A root that is itself a link is refused. The root is then resolved once to its canonical
/// path, which is opened without following a link; it must be the directory the first check
/// saw, by device and inode, so a root swapped for a link in between is refused. Every
/// directory is opened before it is listed, and every file before it is read, with a flag that
/// refuses links. What that flag covers differs:
///
/// - On macOS it is `O_NOFOLLOW_ANY`, which refuses a link in any component. So no file is
///   read, and no directory opened, through a link, even one swapped in while `pin` runs. A
///   directory is listed by path after its check, so a link swapped in between is listed
///   through, but nothing under it is opened.
/// - On Linux it is `O_NOFOLLOW`, which refuses a link only as the last component. A directory
///   swapped for a link while `pin` runs is followed, so a file outside `root` can be read; it
///   is pinned only when its bytes match the listing. plasmosome-0v4o walks by directory
///   descriptors instead.
pub fn pin(
    root: &Path,
    expected: &[PackageFile],
    commands: &[CommandName],
) -> Result<PinnedTree, PinRefusal> {
    pin_observed(root, expected, commands, &mut |_, _| {})
}

fn pin_observed(
    root: &Path,
    expected: &[PackageFile],
    commands: &[CommandName],
    observe: &mut dyn FnMut(Step, &str),
) -> Result<PinnedTree, PinRefusal> {
    check_listing(expected)?;
    let listed: BTreeMap<&str, &PackageFile> = expected
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    if let Some(command) = commands
        .iter()
        .find(|command| !listed.contains_key(command.0.as_str()))
    {
        return Err(PinRefusal::CommandNotInTree {
            name: command.0.clone(),
        });
    }
    let root = resolve(root, observe)?;
    let mut walk = Walk {
        listed: &listed,
        files: Vec::new(),
        observe,
    };
    walk.directory(&root, "")?;
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

fn check_listing(expected: &[PackageFile]) -> Result<(), PinRefusal> {
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    for file in expected {
        let path = file.path.as_str();
        if !follows_path_rule(path) {
            return Err(PinRefusal::InvalidPath { path: path.into() });
        }
        let folded = path.to_ascii_lowercase();
        let ancestors: Vec<String> = folded
            .match_indices('/')
            .map(|(at, _)| folded[..at].to_string())
            .collect();
        if files.contains(&folded)
            || directories.contains(&folded)
            || ancestors.iter().any(|ancestor| files.contains(ancestor))
        {
            return Err(PinRefusal::Collides { path: path.into() });
        }
        directories.extend(ancestors);
        files.insert(folded);
    }
    Ok(())
}

fn follows_path_rule(path: &str) -> bool {
    let drive = matches!(path.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    !drive
        && !path.contains(|character: char| character == '\\' || character.is_ascii_control())
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

fn resolve(root: &Path, observe: &mut dyn FnMut(Step, &str)) -> Result<PathBuf, PinRefusal> {
    let unreadable = || PinRefusal::Unreadable { path: ".".into() };
    let metadata = fs::symlink_metadata(root).map_err(|_| unreadable())?;
    if metadata.file_type().is_symlink() {
        return Err(PinRefusal::Link { path: ".".into() });
    }
    observe(Step::RootChecked, ".");
    let canonical = fs::canonicalize(root).map_err(|_| unreadable())?;
    let resolved = open_without_links(&canonical, ".")?
        .metadata()
        .map_err(|_| unreadable())?;
    if (resolved.dev(), resolved.ino()) != (metadata.dev(), metadata.ino()) {
        return Err(PinRefusal::Link { path: ".".into() });
    }
    Ok(canonical)
}

fn open_without_links(at: &Path, path: &str) -> Result<File, PinRefusal> {
    OpenOptions::new()
        .read(true)
        .custom_flags(NO_LINKS | libc::O_NONBLOCK)
        .open(at)
        .map_err(|error| {
            if error.raw_os_error() == Some(libc::ELOOP) {
                PinRefusal::Link { path: path.into() }
            } else {
                PinRefusal::Unreadable { path: path.into() }
            }
        })
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
    listed: &'a BTreeMap<&'a str, &'a PackageFile>,
    files: Vec<TreeFile>,
    observe: &'a mut dyn FnMut(Step, &str),
}

impl Walk<'_> {
    fn directory(&mut self, at: &Path, prefix: &str) -> Result<(), PinRefusal> {
        let name = if prefix.is_empty() { "." } else { prefix };
        let unreadable = || PinRefusal::Unreadable { path: name.into() };
        open_without_links(at, name)?;
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
        (self.observe)(Step::Listed, &path);
        let file = open_without_links(at, &path)?;
        let size = match file.metadata() {
            Ok(metadata) if metadata.is_file() => metadata.len(),
            Ok(_) => return Err(PinRefusal::NotRegular { path }),
            Err(_) => return Err(PinRefusal::Unreadable { path }),
        };
        if size != listed.size {
            return Err(PinRefusal::DigestMismatch { path });
        }
        (self.observe)(Step::Opened, &path);
        let sha256 =
            Digest::of_reader(&file).map_err(|_| PinRefusal::Unreadable { path: path.clone() })?;
        if sha256 != listed.digest {
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
    /// An earlier command of the same tree, by its name, folds to the same name on a volume that
    /// ignores case.
    Command(String),
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
            Holder::Command(command) => write!(
                f,
                "{target:?} is the same name as command {command:?} of the same tree on a volume \
                 that ignores case"
            ),
        }
    }
}

impl std::error::Error for Conflict {}

/// Decides spec 027's name refusals for `tree` before attach has any effect.
///
/// `attached` holds each other attached plasmid's ID and pinned tree; a reload leaves out the
/// plasmid it replaces. `image_names` returns the names the root image has in one directory of
/// spec 024's `PATH`, and is asked about `/usr/local/bin`, `/usr/bin` and `/bin`. Pass
/// `case_insensitive` when the volume that holds the names ignores case. Then two names collide
/// when their Unicode lowercase forms are equal, and two commands of `tree` itself can collide.
/// That fold is not the volume's: it misses names that differ in Unicode normalization, such as
/// NFC and NFD `café`, and names that only full case folding joins, such as `ß` and `SS`. The
/// tree's own names are checked first, then other plasmids', then the image's.
pub fn check_names(
    tree: &PinnedTree,
    attached: &[(&str, &PinnedTree)],
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
    let names: Vec<&str> = tree.commands().collect();
    for (at, name) in names.iter().enumerate() {
        if let Some(earlier) = names[..at].iter().find(|earlier| same(earlier, name)) {
            return Err(conflict(name, Holder::Command(earlier.to_string())));
        }
    }
    for name in &names {
        if let Some((plasmid, _)) = attached
            .iter()
            .find(|(_, other)| other.commands().any(|held| same(held, name)))
        {
            return Err(conflict(name, Holder::Plasmid(plasmid.to_string())));
        }
    }
    let image: Vec<String> = PATH_DIRECTORIES
        .iter()
        .flat_map(|directory| image_names(directory))
        .collect();
    if let Some(name) = names
        .iter()
        .find(|name| image.iter().any(|held| same(held, name)))
    {
        return Err(conflict(name, Holder::Image));
    }
    Ok(())
}

/// The test-only route to a command, until spec 027's declaration field exists. It compiles only
/// in this crate's tests and under the `test-fixture` feature. Enable that feature only from
/// `[dev-dependencies]`; a reviewer checks this, and nothing else does.
#[cfg(any(test, feature = "test-fixture"))]
pub mod fixture {
    use super::{CommandName, follows_path_rule};

    /// Declares each of `names` as a command. Panics when a name is not exactly one component
    /// that [`TreePath`](super::TreePath)'s rule accepts.
    pub fn declare_commands(names: &[&str]) -> Vec<CommandName> {
        names
            .iter()
            .map(|name| {
                assert!(
                    follows_path_rule(name) && !name.contains('/'),
                    "{name:?} is not one path component"
                );
                CommandName(name.to_string())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
