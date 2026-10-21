use std::ffi::CString;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::os::fd::OwnedFd;

use plasmosome_core::private_socket::{PrivateDir, PrivateSocketError};

use super::RemoveOutcome;
use super::artifacts::{ArtifactFault, FileIdentity, VerifiedArtifact};
use super::digest::Digest;
use super::path::RecipePath;
use super::recipe::ArtifactRole;

/// A cell's private directory, the one spec 001 §4.2 puts `writable_root`, `control_path` and
/// `data_path` under, checked by core's `PrivateDir` and kept open.
///
/// Get one from [`PrivateDirectory::open`]. The membrane does not create it: in cell mode it is
/// spec 023's `runtime/`, which `membraned` creates through core. A later rename of the path does
/// not move what is made through this value, because it holds the directory open.
#[derive(Debug)]
pub struct PrivateDirectory {
    path: RecipePath,
    dir: PrivateDir,
}

impl PrivateDirectory {
    /// Opens `path` with core's `PrivateDir::open`: walked from "/" without following a symlink,
    /// every ancestor owned by root or the effective UID and writable by neither group nor other,
    /// and `path` itself a directory owned by the effective UID with no group or other bits and
    /// no ACL. On macOS pass the canonical path, `/private/var/...` rather than `/var/...`.
    pub fn open(path: &RecipePath) -> Result<PrivateDirectory, CellDirectoryRefusal> {
        let _ = (path, PrivateDir::open);
        todo!()
    }

    /// The path it was opened at.
    pub fn path(&self) -> &RecipePath {
        &self.path
    }

    /// The device and inode of the directory held open.
    pub fn identity(&self) -> FileIdentity {
        let _ = &self.dir;
        todo!()
    }

    /// Walks the path again with the same checks and requires the directory held open. A
    /// directory renamed away and replaced at the path is `Private(Replaced)`. Call it just before
    /// a launch that relies on the path.
    pub fn reconfirm(&self) -> Result<(), CellDirectoryRefusal> {
        todo!()
    }
}

/// The per-cell RAW copy of the verified root image that spec 001 §4.2 calls `writable_root`.
///
/// Only [`WritableRoot::create`] makes one, as a new file inside a [`PrivateDirectory`]. It keeps
/// the file and that directory open. [`WritableRoot::remove`] removes it only while the name in
/// the held directory still holds the file it created; dropping it does the same, so a refusal or
/// a panic after the copy leaves no copy behind.
#[derive(Debug)]
pub struct WritableRoot {
    path: RecipePath,
    parent: OwnedFd,
    name: CString,
    file: File,
    identity: FileIdentity,
    removed: bool,
}

impl WritableRoot {
    /// Creates `path` inside `parent` and copies the verified root image into it.
    ///
    /// `source` must be the recipe's root image, and `path`'s parent must be the path `parent` was
    /// opened at, which must still pass [`PrivateDirectory::reconfirm`]. The name is created with
    /// `O_CREAT | O_EXCL | O_NOFOLLOW`, so any entry already there, a symlink included, is
    /// `Exists` and is left as it was. The file is set to mode 0600 whatever the umask. The bytes
    /// are read through `source.file()` with positioned reads, at most one byte past the size it
    /// was verified at, and hashed while they are written; a digest other than `source.digest()`
    /// is `CopyMismatch`. The copy is synced, then `source.reconfirm()` must pass, or the refusal
    /// is `Source`. Every refusal after the name was created removes the copy by identity.
    pub fn create(
        path: &RecipePath,
        parent: &PrivateDirectory,
        source: &VerifiedArtifact,
    ) -> Result<WritableRoot, CellDirectoryRefusal> {
        let _ = |reader: std::io::Empty| create_copy(path, parent, reader, source.digest());
        todo!()
    }

    /// The path it was created at.
    pub fn path(&self) -> &RecipePath {
        &self.path
    }

    /// The device and inode of the file it created.
    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    /// Removes the file only while the name in the held directory is still that regular file, by
    /// device and inode, and says what it found. A name that is gone is `AlreadyGone`; another
    /// object at the name is `LeftReplacement` and is not touched. The inspection and the unlink
    /// are two steps, so another writer with authority over the directory, the same UID included,
    /// can put an object at the name between them.
    pub fn remove(self) -> RemoveOutcome {
        let _ = (&self.parent, &self.name, &self.file, self.removed);
        todo!()
    }
}

fn create_copy(
    path: &RecipePath,
    parent: &PrivateDirectory,
    reader: impl Read,
    expected: &Digest,
) -> Result<WritableRoot, CellDirectoryRefusal> {
    let _ = |reader: std::io::Empty, writer: &File| copy_verified(reader, writer, expected);
    let _ = (path, parent);
    drop(reader);
    todo!()
}

fn copy_verified(
    reader: impl Read,
    writer: &File,
    expected: &Digest,
) -> Result<(), CellDirectoryFault> {
    let _ = (writer, expected);
    drop(reader);
    todo!()
}

/// Why a cell directory or a writable root was refused: the path asked for and the first rule it
/// broke. When the refusal returns, no file this call created remains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellDirectoryRefusal {
    pub path: RecipePath,
    pub fault: CellDirectoryFault,
}

/// The first rule a cell directory or a writable root broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellDirectoryFault {
    /// Core's private directory refused. Its error names the rule and the component that broke
    /// it: a symlink on the way, a replaceable ancestor, a directory that is not private, one
    /// that carries an ACL, or one that no longer names the directory held open (`Replaced`).
    Private(PrivateSocketError),
    /// The writable root's parent is not the path the [`PrivateDirectory`] was opened at.
    WrongParent,
    /// The source is this artifact of the recipe, not its root image.
    WrongSource { role: ArtifactRole },
    /// An entry is already at the name: a file, a directory, a socket or a symlink, dangling or
    /// not. It was left as it was.
    Exists,
    /// The bytes copied hash to `actual`, not the verified `expected`.
    CopyMismatch { expected: Digest, actual: Digest },
    /// After the copy, the root image's `reconfirm` refused: its path names another file, or its
    /// size or times moved since it was verified.
    Source(ArtifactFault),
    /// A call failed with this errno.
    Io { op: &'static str, errno: i32 },
}

impl fmt::Display for CellDirectoryRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.as_str(), self.fault)
    }
}

impl std::error::Error for CellDirectoryRefusal {}

impl fmt::Display for CellDirectoryFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
    }
}
