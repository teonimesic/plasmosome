use std::fmt;
use std::fs::File;

use plasmosome_core::nofollow::{Facts, Kind, Walk, WalkRefusal};

use super::digest::Digest;
use super::path::RecipePath;
use super::recipe::{Artifact, ArtifactRole, RuntimeRecipe};

/// The device and inode of a verified file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
}

/// One pinned artifact that passed spec 001 §4.2's checks, with the descriptor it was checked
/// and hashed through.
///
/// Only [`verify_artifact`] makes one. The file stays open, so [`VerifiedArtifact::file`] reads
/// the verified inode even after its path is made to name another file; read it with positioned
/// reads, such as `FileExt::read_at`, so readers do not share an offset. The helper and libkrun
/// open artifacts by path, so call [`VerifiedArtifact::reconfirm`] just before the launch that
/// relies on the path.
#[derive(Debug)]
pub struct VerifiedArtifact {
    role: ArtifactRole,
    path: RecipePath,
    walk: Walk,
    file: File,
    identity: FileIdentity,
    size: u64,
    digest: Digest,
}

impl VerifiedArtifact {
    /// Which artifact of the recipe this is.
    pub fn role(&self) -> ArtifactRole {
        self.role
    }

    /// The path it was verified at.
    pub fn path(&self) -> &RecipePath {
        &self.path
    }

    /// The device and inode of the file that was verified.
    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    /// Its size in bytes when it was hashed.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The digest of its bytes, equal to the pinned one.
    pub fn digest(&self) -> &Digest {
        &self.digest
    }

    /// The open descriptor the checks and the hash went through. Its offset is 0 after
    /// verification. Do not change the file through it.
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Opens the path again from the same "/" descriptor, applies every check except the hash,
    /// and requires the same device and inode. A path that now names another file, even one
    /// with the same bytes, is `Replaced`.
    pub fn reconfirm(&self) -> Result<(), ArtifactRefusal> {
        todo!()
    }
}

/// Every artifact of one recipe, verified in field order.
#[derive(Debug)]
pub struct VerifiedArtifacts {
    pub kernel: VerifiedArtifact,
    pub initramfs: VerifiedArtifact,
    pub root_image: VerifiedArtifact,
    pub helper: VerifiedArtifact,
    /// In the recipe's order: `libraries[i]` has the role `Library(i)`.
    pub libraries: Vec<VerifiedArtifact>,
    pub host_policy: VerifiedArtifact,
    pub guest_policy: VerifiedArtifact,
}

/// Why an artifact was refused: its role, its path and the first rule it broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRefusal {
    pub role: ArtifactRole,
    pub path: RecipePath,
    pub fault: ArtifactFault,
}

/// The first rule an artifact broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactFault {
    /// The no-follow walk from "/" refused. Its path field names the component where it
    /// stopped: a symlink, a missing name, a directory another principal can replace entries
    /// in, a component that is not a directory, or one it could not open.
    Walk(WalkRefusal),
    /// The artifact is not a regular file.
    NotRegular { kind: Kind },
    /// The artifact is owned by `uid`, which is neither root nor the effective UID.
    ForeignOwner { uid: u32 },
    /// Someone may write the artifact: its mode has a write bit.
    Writable { mode: u32 },
    /// The artifact's mode has the set-user-ID or set-group-ID bit.
    SetIdBits { mode: u32 },
    /// On macOS, the artifact carries an ACL allow entry granting write, append, write
    /// attributes or security, chown or delete to some principal.
    WriteAcl,
    /// Neither `uchg` nor `schg` is set on the artifact.
    NotImmutable,
    /// This platform cannot show the artifact immutable. Every artifact off macOS is refused
    /// this way.
    ImmutabilityUnverifiable,
    /// The artifact's size, modification time or change time moved while it was hashed.
    ChangedWhileHashing,
    /// The bytes hash to `actual`, not the pinned `expected`.
    DigestMismatch { expected: Digest, actual: Digest },
    /// The path now names another file than the one verified.
    Replaced {
        verified: FileIdentity,
        found: FileIdentity,
    },
    /// A call on the open descriptor failed with this errno.
    Io { op: &'static str, errno: i32 },
}

impl From<WalkRefusal> for ArtifactFault {
    fn from(refusal: WalkRefusal) -> Self {
        ArtifactFault::Walk(refusal)
    }
}

impl fmt::Display for ArtifactRefusal {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl std::error::Error for ArtifactRefusal {}

impl fmt::Display for ArtifactFault {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

/// Verifies one pinned artifact as spec 001 §4.2 asks, before anything trusts its bytes.
///
/// Walks `artifact.path()` from "/" one component at a time without following a symlink; every
/// directory on the way must be owned by root or the effective UID and writable by neither group
/// nor other, with no sticky-bit exception, and on macOS carry no ACL entry letting another
/// principal replace its entries. The last name is opened without blocking and must be a regular
/// file owned by root or the effective UID, with no write bit, no set-id bit, no ACL entry
/// granting write, and `uchg` or `schg` set. Only then are its bytes hashed through that
/// descriptor and compared with `artifact.sha256()`. A file that breaks a rule is refused, never
/// trusted after hashing. Off macOS every artifact is refused `ImmutabilityUnverifiable`.
///
/// On macOS pass the canonical path, `/private/var/...` rather than `/var/...`: a symlink
/// anywhere on the path is refused. Root and the effective UID can still change an artifact
/// after it passes; the checks keep out every other principal.
pub fn verify_artifact(
    _role: ArtifactRole,
    _artifact: &Artifact,
) -> Result<VerifiedArtifact, ArtifactRefusal> {
    todo!()
}

/// Verifies every artifact of `recipe` with [`verify_artifact`] in field order: kernel,
/// initramfs, root image, helper, each library, host policy, guest policy. Stops at the first
/// refusal and closes every descriptor it opened.
pub fn verify_recipe_artifacts(
    _recipe: &RuntimeRecipe,
) -> Result<VerifiedArtifacts, ArtifactRefusal> {
    todo!()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Immutability {
    Set,
    Clear,
    Unverifiable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub(crate) size: u64,
    pub(crate) modified: (i64, i64),
    pub(crate) changed: (i64, i64),
}

pub(crate) fn judge_artifact(_facts: &Facts, _euid: u32) -> Result<(), ArtifactFault> {
    todo!()
}

pub(crate) fn judge_immutability(_immutable: Immutability) -> Result<(), ArtifactFault> {
    todo!()
}

pub(crate) fn judge_unchanged(_before: Stamp, _after: Stamp) -> Result<(), ArtifactFault> {
    todo!()
}
