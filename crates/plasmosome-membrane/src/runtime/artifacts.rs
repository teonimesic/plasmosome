use std::fmt;
use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::PathBuf;

use plasmosome_core::nofollow::{Facts, Kind, Walk, WalkRefusal, acl_grants_write, effective_uid};

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
        let refuse = |fault| ArtifactRefusal {
            role: self.role,
            path: self.path.as_path().to_path_buf(),
            fault,
        };
        let found = open_checked(&self.walk).map_err(refuse)?.identity;
        if found == self.identity {
            Ok(())
        } else {
            Err(refuse(ArtifactFault::Replaced {
                verified: self.identity,
                found,
            }))
        }
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

/// Why an artifact was refused: its role, its path as the recipe gave it and the first rule it
/// broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRefusal {
    pub role: ArtifactRole,
    pub path: PathBuf,
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.role, self.path.display(), self.fault)
    }
}

impl std::error::Error for ArtifactRefusal {}

impl fmt::Display for ArtifactFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArtifactFault::Walk(refusal) => write!(f, "{refusal}"),
            ArtifactFault::NotRegular { kind } => {
                write!(f, "it is {}, not a regular file", kind_name(*kind))
            }
            ArtifactFault::ForeignOwner { uid } => write!(
                f,
                "it is owned by uid {uid}, neither root nor the effective uid"
            ),
            ArtifactFault::Writable { mode } => {
                write!(f, "it has mode {mode:04o}, with a write bit")
            }
            ArtifactFault::SetIdBits { mode } => write!(
                f,
                "it has mode {mode:04o}, with a set-user-ID or set-group-ID bit"
            ),
            ArtifactFault::WriteAcl => f.write_str("it carries an ACL allow entry granting write"),
            ArtifactFault::NotImmutable => f.write_str("it has neither uchg nor schg set"),
            ArtifactFault::ImmutabilityUnverifiable => {
                f.write_str("this platform cannot show it immutable")
            }
            ArtifactFault::ChangedWhileHashing => {
                f.write_str("its size or times moved while it was hashed")
            }
            ArtifactFault::DigestMismatch { expected, actual } => {
                write!(f, "it hashes to {actual}, not the pinned {expected}")
            }
            ArtifactFault::Replaced { verified, found } => write!(
                f,
                "the path names device {} inode {}, not the verified device {} inode {}",
                found.dev, found.ino, verified.dev, verified.ino
            ),
            ArtifactFault::Io { op, errno } => write!(f, "{op} failed with errno {errno}"),
        }
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Directory => "a directory",
        Kind::Regular => "a regular file",
        Kind::Socket => "a socket",
        Kind::Symlink => "a symlink",
        Kind::Other => "a FIFO, a device or another special file",
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
    role: ArtifactRole,
    artifact: &Artifact,
) -> Result<VerifiedArtifact, ArtifactRefusal> {
    let refuse = |fault| ArtifactRefusal {
        role,
        path: artifact.path().as_path().to_path_buf(),
        fault,
    };
    let walk =
        Walk::from_root(artifact.path().as_path()).map_err(|refusal| refuse(refusal.into()))?;
    let checked = open_checked(&walk).map_err(refuse)?;
    let digest = hash(&checked.file).map_err(refuse)?;
    let after = stamp(&stat(&checked.file).map_err(refuse)?);
    judge_unchanged(checked.stamp, after).map_err(refuse)?;
    if digest != *artifact.sha256() {
        return Err(refuse(ArtifactFault::DigestMismatch {
            expected: *artifact.sha256(),
            actual: digest,
        }));
    }
    Ok(VerifiedArtifact {
        role,
        path: artifact.path().clone(),
        walk,
        file: checked.file,
        identity: checked.identity,
        size: checked.stamp.size,
        digest,
    })
}

/// Verifies every artifact of `recipe` with [`verify_artifact`] in field order: kernel,
/// initramfs, root image, helper, each library, host policy, guest policy. Stops at the first
/// refusal and closes every descriptor it opened.
pub fn verify_recipe_artifacts(
    recipe: &RuntimeRecipe,
) -> Result<VerifiedArtifacts, ArtifactRefusal> {
    Ok(VerifiedArtifacts {
        kernel: verify_artifact(ArtifactRole::Kernel, recipe.kernel())?,
        initramfs: verify_artifact(ArtifactRole::Initramfs, recipe.initramfs())?,
        root_image: verify_artifact(ArtifactRole::RootImage, recipe.root_image())?,
        helper: verify_artifact(ArtifactRole::Helper, recipe.helper())?,
        libraries: recipe
            .libraries()
            .iter()
            .enumerate()
            .map(|(index, library)| verify_artifact(ArtifactRole::Library(index), library))
            .collect::<Result<_, _>>()?,
        host_policy: verify_artifact(ArtifactRole::HostPolicy, recipe.host_policy())?,
        guest_policy: verify_artifact(ArtifactRole::GuestPolicy, recipe.guest_policy())?,
    })
}

struct Checked {
    file: File,
    identity: FileIdentity,
    stamp: Stamp,
}

fn open_checked(walk: &Walk) -> Result<Checked, ArtifactFault> {
    let euid = effective_uid();
    let opened = walk.open_file(|reached| judge_artifact(&reached.facts, euid))?;
    let identity = FileIdentity {
        dev: opened.facts.dev,
        ino: opened.facts.ino,
    };
    let file = File::from(opened.fd);
    if acl_grants_write(file.as_fd()).map_err(|error| io_fault("acl", &error))? {
        return Err(ArtifactFault::WriteAcl);
    }
    let metadata = stat(&file)?;
    judge_immutability(&metadata)?;
    Ok(Checked {
        file,
        identity,
        stamp: stamp(&metadata),
    })
}

fn stat(file: &File) -> Result<Metadata, ArtifactFault> {
    file.metadata().map_err(|error| io_fault("fstat", &error))
}

fn stamp(metadata: &Metadata) -> Stamp {
    Stamp {
        size: metadata.size(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    }
}

#[cfg(target_os = "macos")]
fn judge_immutability(metadata: &Metadata) -> Result<(), ArtifactFault> {
    use std::os::macos::fs::MetadataExt;
    if metadata.st_flags() & (libc::UF_IMMUTABLE | libc::SF_IMMUTABLE) != 0 {
        Ok(())
    } else {
        Err(ArtifactFault::NotImmutable)
    }
}

#[cfg(not(target_os = "macos"))]
fn judge_immutability(_metadata: &Metadata) -> Result<(), ArtifactFault> {
    Err(ArtifactFault::ImmutabilityUnverifiable)
}

fn hash(file: &File) -> Result<Digest, ArtifactFault> {
    Digest::of_reader(Positioned { file, offset: 0 }).map_err(|error| io_fault("pread", &error))
}

struct Positioned<'a> {
    file: &'a File,
    offset: u64,
}

impl Read for Positioned<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read_at(buffer, self.offset)?;
        self.offset += read as u64;
        Ok(read)
    }
}

fn io_fault(op: &'static str, error: &io::Error) -> ArtifactFault {
    ArtifactFault::Io {
        op,
        errno: error.raw_os_error().unwrap_or(libc::EIO),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub(crate) size: u64,
    pub(crate) modified: (i64, i64),
    pub(crate) changed: (i64, i64),
}

pub(crate) fn judge_artifact(facts: &Facts, euid: u32) -> Result<(), ArtifactFault> {
    if facts.kind != Kind::Regular {
        return Err(ArtifactFault::NotRegular { kind: facts.kind });
    }
    if facts.uid != 0 && facts.uid != euid {
        return Err(ArtifactFault::ForeignOwner { uid: facts.uid });
    }
    if facts.mode & 0o222 != 0 {
        return Err(ArtifactFault::Writable { mode: facts.mode });
    }
    if facts.mode & 0o6000 != 0 {
        return Err(ArtifactFault::SetIdBits { mode: facts.mode });
    }
    Ok(())
}

pub(crate) fn judge_unchanged(before: Stamp, after: Stamp) -> Result<(), ArtifactFault> {
    if before == after {
        Ok(())
    } else {
        Err(ArtifactFault::ChangedWhileHashing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::strict_json;
    use crate::runtime::test_fixtures::tree::{ArtifactTree, make_fifo};
    use std::os::unix::fs::{OpenOptionsExt, symlink};
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::Duration;

    const EUID: u32 = 501;
    const OTHER: u32 = 502;

    fn facts(kind: Kind, uid: u32, mode: u32) -> Facts {
        Facts {
            kind,
            uid,
            mode,
            dev: 1,
            ino: 2,
        }
    }

    fn artifact_at(path: &Path, sha256: &Digest) -> Artifact {
        let text = serde_json::json!({
            "path": path.to_str().expect("test paths are UTF-8"),
            "sha256": sha256.hex(),
        })
        .to_string();
        let value = strict_json::parse_value(text.as_bytes()).expect("the artifact is JSON");
        Artifact::from_value(&value, "kernel").expect("the artifact is valid")
    }

    fn verify_pinned(path: &Path, sha256: &Digest) -> Result<VerifiedArtifact, ArtifactRefusal> {
        verify_artifact(ArtifactRole::Kernel, &artifact_at(path, sha256))
    }

    fn fault_of(outcome: Result<VerifiedArtifact, ArtifactRefusal>) -> ArtifactFault {
        match outcome {
            Ok(verified) => panic!("{} verified", verified.path().as_str()),
            Err(refusal) => refusal.fault,
        }
    }

    fn walk_fault(path: &Path) -> ArtifactFault {
        fault_of(verify_pinned(path, &Digest::of(b"kernel")))
    }

    #[test]
    fn the_judge_accepts_a_regular_file_of_root_or_the_effective_uid_with_no_write_or_set_id_bit() {
        for accepted in [
            facts(Kind::Regular, EUID, 0o444),
            facts(Kind::Regular, 0, 0o555),
            facts(Kind::Regular, EUID, 0o400),
            facts(Kind::Regular, EUID, 0o1555),
        ] {
            assert_eq!(judge_artifact(&accepted, EUID), Ok(()), "{accepted:?}");
        }
    }

    #[test]
    fn the_judge_refuses_another_type_a_foreign_owner_a_write_bit_and_a_set_id_bit_in_that_order() {
        use ArtifactFault::{ForeignOwner, SetIdBits, Writable};
        use Kind::{Directory, Other, Regular, Socket, Symlink};
        let not_regular = |kind| ArtifactFault::NotRegular { kind };
        let rows = [
            (facts(Directory, EUID, 0o555), not_regular(Directory)),
            (facts(Other, EUID, 0o444), not_regular(Other)),
            (facts(Socket, EUID, 0o444), not_regular(Socket)),
            (facts(Symlink, EUID, 0o444), not_regular(Symlink)),
            (facts(Other, OTHER, 0o666), not_regular(Other)),
            (facts(Regular, OTHER, 0o444), ForeignOwner { uid: OTHER }),
            (facts(Regular, OTHER, 0o4777), ForeignOwner { uid: OTHER }),
            (facts(Regular, EUID, 0o644), Writable { mode: 0o644 }),
            (facts(Regular, EUID, 0o464), Writable { mode: 0o464 }),
            (facts(Regular, 0, 0o446), Writable { mode: 0o446 }),
            (facts(Regular, EUID, 0o4755), Writable { mode: 0o4755 }),
            (facts(Regular, EUID, 0o4555), SetIdBits { mode: 0o4555 }),
            (facts(Regular, 0, 0o2555), SetIdBits { mode: 0o2555 }),
        ];
        for (facts, fault) in rows {
            assert_eq!(judge_artifact(&facts, EUID), Err(fault), "{facts:?}");
        }
    }

    #[test]
    fn a_size_modification_or_change_time_that_moved_while_hashing_refuses() {
        let before = Stamp {
            size: 10,
            modified: (100, 1),
            changed: (200, 2),
        };
        assert_eq!(judge_unchanged(before, before), Ok(()));
        for after in [
            Stamp { size: 11, ..before },
            Stamp {
                modified: (100, 2),
                ..before
            },
            Stamp {
                changed: (201, 2),
                ..before
            },
        ] {
            assert_eq!(
                judge_unchanged(before, after),
                Err(ArtifactFault::ChangedWhileHashing),
                "{after:?}"
            );
        }
    }

    #[test]
    fn a_symlinked_directory_or_a_symlink_at_the_name_refuses_at_that_component() {
        let tree = ArtifactTree::new();
        let real = tree.dir("real", 0o700);
        let file = tree.file("real/kernel", b"kernel", 0o444);
        symlink(&real, tree.join("link")).expect("symlink");
        symlink(&file, tree.join("kernel-link")).expect("symlink");
        assert_eq!(
            walk_fault(&tree.join("link/kernel")),
            ArtifactFault::Walk(WalkRefusal::Symlink {
                path: tree.join("link")
            })
        );
        assert_eq!(
            walk_fault(&tree.join("kernel-link")),
            ArtifactFault::Walk(WalkRefusal::Symlink {
                path: tree.join("kernel-link")
            })
        );
    }

    #[test]
    fn a_missing_name_refuses_where_the_walk_stopped() {
        let tree = ArtifactTree::new();
        assert_eq!(
            walk_fault(&tree.join("absent/kernel")),
            ArtifactFault::Walk(WalkRefusal::Missing {
                path: tree.join("absent")
            })
        );
        assert_eq!(
            walk_fault(&tree.join("kernel")),
            ArtifactFault::Walk(WalkRefusal::Missing {
                path: tree.join("kernel")
            })
        );
    }

    #[test]
    fn a_group_writable_or_sticky_world_writable_directory_on_the_way_refuses_replaceable() {
        let tree = ArtifactTree::new();
        for (name, mode) in [("group", 0o775), ("other", 0o757), ("sticky", 0o1777)] {
            let dir = tree.dir(name, 0o700);
            tree.file(&format!("{name}/kernel"), b"kernel", 0o444);
            crate::runtime::test_fixtures::tree::set_mode(&dir, mode);
            assert_eq!(
                walk_fault(&dir.join("kernel")),
                ArtifactFault::Walk(WalkRefusal::Replaceable { path: dir, mode }),
                "{name}"
            );
        }
    }

    #[test]
    fn a_directory_at_the_artifact_path_refuses_not_regular() {
        let tree = ArtifactTree::new();
        let dir = tree.dir("kernel", 0o555);
        assert_eq!(
            walk_fault(&dir),
            ArtifactFault::NotRegular {
                kind: Kind::Directory
            }
        );
    }

    #[test]
    fn a_fifo_at_the_artifact_path_refuses_not_regular_within_two_seconds() {
        let tree = ArtifactTree::new();
        let fifo = tree.join("kernel");
        make_fifo(&fifo);
        let (sender, receiver) = mpsc::channel();
        let target = fifo.clone();
        let worker = std::thread::spawn(move || {
            let _ = sender.send(walk_fault(&target));
        });
        let fault = receiver.recv_timeout(Duration::from_secs(2));
        if fault.is_err() {
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&fifo);
        }
        let _ = worker.join();
        assert_eq!(
            fault.expect("the verification returns within 2 s"),
            ArtifactFault::NotRegular { kind: Kind::Other }
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_a_read_only_file_is_refused_as_immutability_unverifiable() {
        let tree = ArtifactTree::new();
        let path = tree.file("kernel", b"kernel", 0o444);
        assert_eq!(
            fault_of(verify_pinned(&path, &Digest::of(b"kernel"))),
            ArtifactFault::ImmutabilityUnverifiable
        );
    }

    #[test]
    fn a_refusal_names_its_role_and_path_and_shows_where_the_walk_stopped() {
        let tree = ArtifactTree::new();
        let dir = tree.dir("shared", 0o700);
        let path = tree.file("shared/kernel", b"kernel", 0o444);
        crate::runtime::test_fixtures::tree::set_mode(&dir, 0o775);
        let shown = verify_pinned(&path, &Digest::of(b"kernel"))
            .expect_err("the directory is group-writable")
            .to_string();
        let path = path.to_str().expect("UTF-8");
        let dir = dir.to_str().expect("UTF-8");
        for part in ["kernel ", path, &format!("{dir} has mode 0775")] {
            assert!(shown.contains(part), "{shown:?} names {part}");
        }
    }

    #[test]
    fn each_fault_says_what_was_found() {
        use ArtifactFault::*;
        use Kind::{Directory, Other, Regular, Socket, Symlink};
        let (expected, actual) = (Digest::of(b"a"), Digest::of(b"b"));
        let mismatch = format!("{actual}, not the pinned {expected}");
        let replaced = Replaced {
            verified: FileIdentity { dev: 7, ino: 8 },
            found: FileIdentity { dev: 7, ino: 9 },
        };
        let io = Io {
            op: "fstat",
            errno: 5,
        };
        let rows = [
            (NotRegular { kind: Other }, "special file, not a regular"),
            (NotRegular { kind: Directory }, "a directory, not"),
            (NotRegular { kind: Socket }, "a socket, not"),
            (NotRegular { kind: Symlink }, "a symlink, not"),
            (NotRegular { kind: Regular }, "a regular file, not"),
            (ForeignOwner { uid: OTHER }, "owned by uid 502"),
            (Writable { mode: 0o644 }, "mode 0644, with a write bit"),
            (SetIdBits { mode: 0o4555 }, "mode 4555, with a set-user"),
            (WriteAcl, "ACL allow entry granting write"),
            (NotImmutable, "neither uchg nor schg"),
            (ImmutabilityUnverifiable, "cannot show it immutable"),
            (ChangedWhileHashing, "moved while it was hashed"),
            (DigestMismatch { expected, actual }, &mismatch),
            (replaced, "inode 9, not the verified device 7 inode 8"),
            (io, "fstat failed with errno 5"),
        ];
        for (fault, part) in rows {
            let shown = fault.to_string();
            assert!(shown.contains(part), "{shown:?} names {part}");
        }
    }

    #[cfg(target_os = "macos")]
    mod macos {
        use super::*;
        use crate::runtime::test_fixtures::tree::{add_acl, set_flags};
        use crate::runtime::test_fixtures::{recipe, recipe_value};
        use std::fs;
        use std::io::Seek;
        use std::os::unix::fs::{FileExt, MetadataExt};
        use std::path::PathBuf;

        fn identity_of(path: &Path) -> FileIdentity {
            let metadata = fs::symlink_metadata(path).expect("stat");
            FileIdentity {
                dev: metadata.dev(),
                ino: metadata.ino(),
            }
        }

        fn read_all(file: &File, size: usize) -> Vec<u8> {
            let mut bytes = vec![0; size];
            file.read_exact_at(&mut bytes, 0).expect("pread");
            bytes
        }

        #[allow(unsafe_code)]
        fn descriptors_naming(identities: &[FileIdentity]) -> Vec<i32> {
            fs::read_dir("/dev/fd")
                .expect("/dev/fd lists this process's descriptors")
                .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<i32>().ok())
                .filter(|fd| {
                    // SAFETY: libc::stat is a plain C struct, for which all zeroes is a valid
                    // value.
                    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
                    // SAFETY: fstat writes only the local `stat`. A descriptor another thread
                    // closed meanwhile fails with EBADF, which is skipped.
                    let found = unsafe { libc::fstat(*fd, &mut stat) } == 0;
                    found
                        && identities.contains(&FileIdentity {
                            dev: stat.st_dev as u64,
                            ino: stat.st_ino,
                        })
                })
                .collect()
        }

        struct Bundle {
            _tree: ArtifactTree,
            paths: Vec<(ArtifactRole, PathBuf)>,
        }

        impl Bundle {
            fn identities(&self) -> Vec<FileIdentity> {
                self.paths
                    .iter()
                    .map(|(_, path)| identity_of(path))
                    .collect()
            }
        }

        const ROLES: [(ArtifactRole, &str, &str); 7] = [
            (ArtifactRole::Kernel, "kernel", "kernel"),
            (ArtifactRole::Initramfs, "initramfs", "initramfs"),
            (ArtifactRole::RootImage, "root_image", "rootfs.raw"),
            (ArtifactRole::Helper, "helper", "krun-helper"),
            (ArtifactRole::Library(0), "libraries", "libkrun.dylib"),
            (ArtifactRole::HostPolicy, "host_policy", "host-policy"),
            (ArtifactRole::GuestPolicy, "guest_policy", "guest-policy"),
        ];

        fn bundle() -> Bundle {
            let tree = ArtifactTree::new();
            let paths = ROLES
                .iter()
                .map(|(role, _, name)| (*role, tree.immutable_file(name, name.as_bytes())))
                .collect();
            Bundle { _tree: tree, paths }
        }

        fn recipe_pinning(bundle: &Bundle, tampered: Option<ArtifactRole>) -> RuntimeRecipe {
            let mut value = recipe_value("/opt/cell/helper");
            for ((role, field, name), (_, path)) in ROLES.iter().zip(&bundle.paths) {
                let pinned = if Some(*role) == tampered {
                    Digest::of(b"tampered")
                } else {
                    Digest::of(name.as_bytes())
                };
                let artifact = serde_json::json!({
                    "path": path.to_str().expect("UTF-8"),
                    "sha256": pinned.hex(),
                });
                value[*field] = if *field == "libraries" {
                    serde_json::json!([artifact])
                } else {
                    artifact
                };
            }
            recipe(&value)
        }

        #[test]
        fn a_read_only_immutable_file_verifies_with_its_identity_size_and_digest() {
            let tree = ArtifactTree::new();
            let path = tree.immutable_file("kernel", b"the kernel bytes");
            let verified =
                verify_pinned(&path, &Digest::of(b"the kernel bytes")).expect("the file verifies");
            assert_eq!(verified.role(), ArtifactRole::Kernel);
            assert_eq!(verified.path().as_path(), path);
            assert_eq!(verified.identity(), identity_of(&path));
            assert_eq!(verified.size(), 16);
            assert_eq!(verified.digest(), &Digest::of(b"the kernel bytes"));
            assert_eq!(read_all(verified.file(), 16), b"the kernel bytes");
            let mut file = verified.file();
            assert_eq!(file.stream_position().expect("the offset"), 0);
        }

        #[test]
        fn the_var_alias_of_the_temp_directory_refuses_at_var() {
            let tree = ArtifactTree::new();
            let path = tree.immutable_file("kernel", b"kernel");
            let canonical = path.to_str().expect("UTF-8");
            let alias = canonical
                .strip_prefix("/private")
                .expect("the per-user temp directory is under /private/var");
            assert_eq!(
                walk_fault(Path::new(alias)),
                ArtifactFault::Walk(WalkRefusal::Symlink {
                    path: "/var".into()
                })
            );
        }

        #[test]
        fn a_wrong_pin_refuses_with_the_pinned_and_the_actual_digest() {
            let tree = ArtifactTree::new();
            let path = tree.immutable_file("kernel", b"kernel");
            assert_eq!(
                fault_of(verify_pinned(&path, &Digest::of(b"another kernel"))),
                ArtifactFault::DigestMismatch {
                    expected: Digest::of(b"another kernel"),
                    actual: Digest::of(b"kernel"),
                }
            );
        }

        #[test]
        fn a_file_without_uchg_a_writable_one_and_a_set_id_one_refuse() {
            let tree = ArtifactTree::new();
            let plain = tree.file("plain", b"kernel", 0o444);
            let writable = tree.file("writable", b"kernel", 0o644);
            set_flags(&writable, libc::UF_IMMUTABLE);
            let set_id = tree.file("set-id", b"kernel", 0o4555);
            set_flags(&set_id, libc::UF_IMMUTABLE);
            let pin = Digest::of(b"kernel");
            assert_eq!(
                fault_of(verify_pinned(&plain, &pin)),
                ArtifactFault::NotImmutable
            );
            assert_eq!(
                fault_of(verify_pinned(&writable, &pin)),
                ArtifactFault::Writable { mode: 0o644 }
            );
            assert_eq!(
                fault_of(verify_pinned(&set_id, &pin)),
                ArtifactFault::SetIdBits { mode: 0o4555 }
            );
        }

        #[test]
        fn an_acl_entry_granting_write_refuses() {
            let tree = ArtifactTree::new();
            let path = tree.file("kernel", b"kernel", 0o444);
            add_acl(&path, "everyone allow write");
            set_flags(&path, libc::UF_IMMUTABLE);
            assert_eq!(
                fault_of(verify_pinned(&path, &Digest::of(b"kernel"))),
                ArtifactFault::WriteAcl
            );
        }

        #[test]
        fn reconfirm_passes_while_the_path_names_the_verified_file_and_refuses_a_replacement() {
            let tree = ArtifactTree::new();
            let path = tree.immutable_file("kernel", b"kernel");
            let verified = verify_pinned(&path, &Digest::of(b"kernel")).expect("verifies");
            assert_eq!(verified.reconfirm(), Ok(()));
            set_flags(&path, 0);
            fs::rename(&path, tree.join("kernel.old")).expect("rename");
            let replacement = tree.immutable_file("kernel", b"kernel");
            let refusal = verified
                .reconfirm()
                .expect_err("the path names another file");
            assert_eq!(refusal.role, ArtifactRole::Kernel);
            assert_eq!(refusal.path.as_path(), path);
            assert_eq!(
                refusal.fault,
                ArtifactFault::Replaced {
                    verified: verified.identity(),
                    found: identity_of(&replacement),
                }
            );
        }

        #[test]
        fn the_file_reads_the_verified_bytes_after_the_path_names_another_file() {
            let tree = ArtifactTree::new();
            let path = tree.immutable_file("kernel", b"kernel");
            let verified = verify_pinned(&path, &Digest::of(b"kernel")).expect("verifies");
            set_flags(&path, 0);
            fs::rename(&path, tree.join("kernel.old")).expect("rename");
            tree.immutable_file("kernel", b"KERNEL");
            assert_eq!(read_all(verified.file(), 6), b"kernel");
        }

        #[test]
        fn the_recipe_verifies_every_role_in_field_order_and_holds_each_file_open() {
            let bundle = bundle();
            let verified = verify_recipe_artifacts(&recipe_pinning(&bundle, None))
                .expect("every artifact verifies");
            let all: Vec<&VerifiedArtifact> = [
                &verified.kernel,
                &verified.initramfs,
                &verified.root_image,
                &verified.helper,
            ]
            .into_iter()
            .chain(&verified.libraries)
            .chain([&verified.host_policy, &verified.guest_policy])
            .collect();
            let found: Vec<(ArtifactRole, PathBuf)> = all
                .iter()
                .map(|artifact| (artifact.role(), artifact.path().as_path().to_path_buf()))
                .collect();
            assert_eq!(found, bundle.paths);
            assert_eq!(descriptors_naming(&bundle.identities()).len(), 7);
            drop(verified);
            assert_eq!(descriptors_naming(&bundle.identities()), Vec::<i32>::new());
        }

        #[test]
        fn the_recipe_stops_at_a_tampered_root_image_and_leaves_no_descriptor_open() {
            let bundle = bundle();
            let refusal =
                verify_recipe_artifacts(&recipe_pinning(&bundle, Some(ArtifactRole::RootImage)))
                    .expect_err("the root image is tampered");
            assert_eq!(refusal.role, ArtifactRole::RootImage);
            assert_eq!(
                refusal.fault,
                ArtifactFault::DigestMismatch {
                    expected: Digest::of(b"tampered"),
                    actual: Digest::of(b"rootfs.raw"),
                }
            );
            assert_eq!(descriptors_naming(&bundle.identities()), Vec::<i32>::new());
        }
    }
}
