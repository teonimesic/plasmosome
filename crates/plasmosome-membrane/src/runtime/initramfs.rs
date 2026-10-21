use std::fmt;
use std::fs::File;

use super::artifacts::VerifiedArtifact;
use super::digest::Digest;
use super::recipe::ArtifactRole;

/// The largest initramfs [`verify_initramfs`] reads: 256 MiB. A larger one is refused, never cut.
pub const MAX_INITRAMFS_BYTES: u64 = 256 << 20;

/// What [`verify_initramfs`] found in an archive that binds the guest policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitramfsBinding {
    /// The size of the `init` entry's data.
    pub init_size: u64,
    /// The SHA-256 of the `init` entry's data.
    pub init_digest: Digest,
    /// The SHA-256 of the `plasmosome/guest-policy` entry's data, equal to the guest policy's.
    pub policy_digest: Digest,
    /// How many entries come before the trailer.
    pub entries: usize,
}

/// Why an initramfs does not bind the guest policy as spec 001 §4.2 asks. An `offset` counts bytes
/// from the start of the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitramfsRefusal {
    /// An artifact with this role was passed where the initramfs or the guest policy belongs.
    WrongRole { role: ArtifactRole },
    /// The archive holds `bytes` bytes, more than [`MAX_INITRAMFS_BYTES`]. When it grew after
    /// verification, `bytes` counts only what was read: the bound plus one.
    TooLarge { bytes: u64 },
    /// The entry at `offset` does not start with the newc magic `070701`. Compressed archives,
    /// the other cpio formats and zero bytes between entries are refused here.
    NotNewc { offset: u64 },
    /// A header field of the entry at `offset` is not 8 hexadecimal digits, or is a `filesize`
    /// other than 0 on a directory or the trailer, which the kernel would skip without reading.
    BadHeaderField { offset: u64, field: &'static str },
    /// The entry at `offset` has a check field other than 0.
    NonzeroCheck { offset: u64 },
    /// The name of the entry at `offset` is not one plain relative spelling, for `reason`.
    BadName { offset: u64, reason: &'static str },
    /// A second entry has this name.
    DuplicateEntry { name: String },
    /// This entry is a symbolic link.
    SymlinkEntry { name: String },
    /// This entry is neither a regular file, a directory nor a symlink; `mode` is its mode field.
    UnsupportedType { name: String, mode: u32 },
    /// This regular entry has an `nlink` other than 1, so the kernel may make it a hard link.
    HardLink { name: String, nlink: u32 },
    /// No earlier directory entry is this entry's parent.
    MissingParent { name: String },
    /// The part of the archive that starts at `offset` runs past its end.
    Truncated { offset: u64 },
    /// The padding byte at `offset` is not zero.
    NonzeroPadding { offset: u64 },
    /// The byte at `offset` comes after the trailer and is not zero. The kernel would unpack
    /// what follows as another archive.
    DataAfterTrailer { offset: u64 },
    /// The archive ends without a `TRAILER!!!` entry.
    MissingTrailer,
    /// No regular entry is named `init`.
    MissingInit,
    /// The `init` entry has no execute bit; `mode` is its mode field.
    InitNotExecutable { mode: u32 },
    /// The `init` entry is empty.
    InitEmpty,
    /// No regular entry is named `plasmosome/guest-policy`.
    MissingGuestPolicy,
    /// The `plasmosome/guest-policy` entry hashes to `actual`, not the verified `expected`.
    GuestPolicyMismatch { expected: Digest, actual: Digest },
    /// A read of the initramfs through its descriptor failed with this errno.
    Io { op: &'static str, errno: i32 },
}

impl fmt::Display for InitramfsRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::error::Error for InitramfsRefusal {}

/// Proves that the verified `initramfs` binds the verified `guest_policy`, as spec 001 §4.2 asks
/// before launch.
///
/// `initramfs` must have the role `Initramfs` and `guest_policy` the role `GuestPolicy`, or it
/// refuses `WrongRole`. An initramfs over [`MAX_INITRAMFS_BYTES`] is refused before any read.
/// Otherwise it reads the archive from offset 0 to its end with positioned reads through
/// `initramfs.file()`, never by path, through a limit of the bound plus one byte. The archive
/// must be uncompressed newc whose entries are plain relative names, regular files with one link
/// or directories, each name once with its parent before it, ending in `TRAILER!!!` and then only
/// zero bytes. It must hold a regular, executable, non-empty `init`, a `plasmosome` directory and
/// a regular `plasmosome/guest-policy` whose bytes hash to `guest_policy.digest()`.
///
/// It reads the bytes the descriptor holds now and does not hash them again. A write through a
/// shared mapping made before the file was locked can change them without moving its size or
/// times, as [`VerifiedArtifact::reconfirm`] says. Call `reconfirm` after this, just before the
/// launch that opens the initramfs by path.
pub fn verify_initramfs(
    initramfs: &VerifiedArtifact,
    guest_policy: &VerifiedArtifact,
) -> Result<InitramfsBinding, InitramfsRefusal> {
    let _ = (initramfs, guest_policy);
    todo!()
}

pub(crate) fn audit_newc(
    bytes: &[u8],
    expected_policy: &Digest,
) -> Result<InitramfsBinding, InitramfsRefusal> {
    let _ = (bytes, expected_policy);
    todo!()
}

fn read_bounded(file: &File, size: u64, bound: u64) -> Result<Vec<u8>, InitramfsRefusal> {
    let _ = (file, size, bound);
    todo!()
}
