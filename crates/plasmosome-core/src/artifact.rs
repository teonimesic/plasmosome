use std::fmt;
use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use plasmosome_backend::Digest;

use crate::nofollow::{Kind, Reached, Walk, WalkRefusal, acl_grants_write, effective_uid};
use crate::operator::trusted::kind_name;
use crate::protocol::WireError;
use crate::runtime::path::RecipePath;
use crate::runtime::recipe::Artifact;

/// Why an Artifact is not trusted: its path as the recipe names it, and the first rule it broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRefusal {
    pub path: RecipePath,
    pub reason: ArtifactFault,
}

/// The rule an Artifact broke, in the order [`verify_artifact`] checks them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactFault {
    /// The walk from `/` stopped: a component is a symlink or missing, a directory above the file
    /// is not owned by root or the controller's UID, group or other can write it, or a macOS ACL
    /// entry lets anyone replace its entries. A failed open, `fstat`, ACL read or read is `Io`.
    Walk(WalkRefusal),
    /// The object at the path is a `kind` other than a regular file.
    NotRegular { kind: Kind },
    /// The file is owned by `uid`, which is neither root nor the controller's effective UID.
    ForeignOwner { uid: u32 },
    /// The file's mode has a write bit, for its owner, its group or others; `mode` holds its
    /// permission bits.
    Writable { mode: u32 },
    /// The file's mode has the set-user-ID or the set-group-ID bit.
    SetIdBits { mode: u32 },
    /// A macOS ACL allow entry on the file grants write to someone.
    WriteAcl,
    /// On macOS, the file has neither `uchg` nor `schg` set.
    NotImmutable,
    /// Off macOS, this build cannot verify that the file is immutable. A file that passes every
    /// earlier rule is refused this way, so no Artifact passes there.
    ImmutabilityUnverifiable,
    /// The file's size, modification time or change time moved between the check and the end of
    /// the hash.
    ChangedWhileHashing,
    /// The file's bytes hash to `found`, not to the pinned digest.
    Digest { found: Digest },
}

/// Checks one Artifact before the controller trusts it, as spec 023 §1 says for `membraned` and
/// every Artifact of the template. Walks its path from `/` without following a symlink, and
/// refuses unless every directory above the file is owned by root or the effective UID, writable
/// by neither group nor other and, on macOS, carries no ACL allow entry, whoever it names, that
/// lets it replace the directory's entries, and is on no volume mounted with ownership ignored.
/// The file, opened without following and without blocking, must be a regular file owned by root
/// or the effective UID, with no write bit at all, no set-user-ID or set-group-ID bit, no ACL
/// allow entry granting write, and `uchg` or `schg` set. Off macOS, a file that passes every
/// earlier rule is refused as `ImmutabilityUnverifiable`, because this build cannot verify
/// immutability there. These are the file rules the membrane repeats at launch. Only then is the
/// file read, through the descriptor that was checked, and hashed: a file that breaks a rule is
/// refused, never trusted because its digest matches. The read stops one byte past the size
/// `fstat` reported at the check, so it ends even when the file grows while it is read.
///
/// The check records the file's size, modification time and change time when it passes, and
/// again after the hash, and refuses `ChangedWhileHashing` if any moved, before the digest is
/// compared. That catches a `write` made between the check and the end of the hash through a
/// descriptor opened before the file was locked, since the mode and `uchg` refuse only new opens.
/// A `MAP_SHARED` mapping made before the file was locked, from a descriptor opened for writing,
/// can change the bytes whatever protection it had when the file was locked, because `mprotect`
/// can add write later. Such a write moves the times only at `msync` or `munmap`, if at all, so
/// neither this check nor the membrane's sees it.
///
/// A pass describes the file as it was during the check. Whoever opens it later by path must
/// guard its own open.
pub fn verify_artifact(artifact: &Artifact) -> Result<(), ArtifactRefusal> {
    let walk = Walk::from_root(artifact.path().as_path())
        .map_err(|refusal| refused(artifact, ArtifactFault::Walk(refusal)))?;
    verify_walk(walk, artifact)
}

fn verify_walk(walk: Walk, artifact: &Artifact) -> Result<(), ArtifactRefusal> {
    let path = artifact.path().as_path();
    open_checked(walk, path)
        .and_then(|checked| hash_checked(checked, path, artifact.sha256()))
        .map_err(|reason| refused(artifact, reason))
}

struct Checked {
    file: File,
    stamp: Stamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

fn open_checked(walk: Walk, path: &Path) -> Result<Checked, ArtifactFault> {
    let euid = effective_uid();
    let opened = walk.open_file(|reached| judge(reached, euid))?;
    let file = File::from(opened.fd);
    if acl_grants_write(file.as_fd()).map_err(|error| io_fault("acl", path, &error))? {
        return Err(ArtifactFault::WriteAcl);
    }
    let metadata = file
        .metadata()
        .map_err(|error| io_fault("fstat", path, &error))?;
    judge_immutability(&metadata)?;
    Ok(Checked {
        stamp: stamp(&metadata),
        file,
    })
}

fn hash_checked(checked: Checked, path: &Path, pinned: &Digest) -> Result<(), ArtifactFault> {
    let found = digest_checked(&checked, path)?;
    let after = checked
        .file
        .metadata()
        .map_err(|error| io_fault("fstat", path, &error))?;
    judge_unchanged(checked.stamp, stamp(&after))?;
    if found != *pinned {
        return Err(ArtifactFault::Digest { found });
    }
    Ok(())
}

fn digest_checked(checked: &Checked, path: &Path) -> Result<Digest, ArtifactFault> {
    digest_within(&checked.file, checked.stamp.size).map_err(|error| io_fault("read", path, &error))
}

fn stamp(metadata: &Metadata) -> Stamp {
    Stamp {
        size: metadata.size(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    }
}

fn judge_unchanged(before: Stamp, after: Stamp) -> Result<(), ArtifactFault> {
    let unchanged = before.size == after.size
        && before.modified == after.modified
        && before.changed == after.changed;
    if unchanged {
        Ok(())
    } else {
        Err(ArtifactFault::ChangedWhileHashing)
    }
}

fn judge(reached: &Reached<'_>, euid: u32) -> Result<(), ArtifactFault> {
    let facts = reached.facts;
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

#[cfg(target_os = "macos")]
fn judge_immutability(metadata: &Metadata) -> Result<(), ArtifactFault> {
    use std::os::macos::fs::MetadataExt;
    if is_immutable(metadata.st_flags()) {
        Ok(())
    } else {
        Err(ArtifactFault::NotImmutable)
    }
}

#[cfg(target_os = "macos")]
fn is_immutable(flags: u32) -> bool {
    flags & (libc::UF_IMMUTABLE | libc::SF_IMMUTABLE) != 0
}

#[cfg(not(target_os = "macos"))]
fn judge_immutability(_metadata: &Metadata) -> Result<(), ArtifactFault> {
    Err(ArtifactFault::ImmutabilityUnverifiable)
}

fn digest_within(reader: impl Read, size: u64) -> io::Result<Digest> {
    Digest::of_reader(reader.take(size.saturating_add(1)))
}

fn refused(artifact: &Artifact, reason: ArtifactFault) -> ArtifactRefusal {
    ArtifactRefusal {
        path: artifact.path().clone(),
        reason,
    }
}

fn io_fault(op: &'static str, path: &Path, error: &io::Error) -> ArtifactFault {
    ArtifactFault::Walk(WalkRefusal::Io {
        op,
        path: path.to_path_buf(),
        errno: error.raw_os_error().unwrap_or(libc::EIO),
    })
}

impl From<WalkRefusal> for ArtifactFault {
    fn from(refusal: WalkRefusal) -> Self {
        ArtifactFault::Walk(refusal)
    }
}

impl ArtifactRefusal {
    /// The refusal as `cell.new` answers it: code 108 with `path` the Artifact's path, `detail`
    /// the broken rule, a message that names an untrusted artifact, and no `fix`.
    pub fn wire(&self) -> WireError {
        WireError::untrusted_artifact(self.reason.to_string(), self.path.as_str().to_owned())
    }
}

impl fmt::Display for ArtifactRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is not a trusted artifact: {}",
            self.path.as_str(),
            self.reason
        )
    }
}

impl std::error::Error for ArtifactRefusal {}

impl fmt::Display for ArtifactFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArtifactFault::Walk(refusal) => refusal.fmt(f),
            ArtifactFault::NotRegular { kind } => {
                write!(f, "it is {}, not a regular file", kind_name(*kind))
            }
            ArtifactFault::ForeignOwner { uid } => write!(
                f,
                "it is owned by uid {uid}, neither root nor the controller's uid {}",
                effective_uid()
            ),
            ArtifactFault::Writable { mode } => {
                write!(f, "it has mode {mode:04o}, with a write bit")
            }
            ArtifactFault::SetIdBits { mode } => write!(
                f,
                "it has mode {mode:04o}, with a set-user-ID or set-group-ID bit"
            ),
            ArtifactFault::WriteAcl => {
                f.write_str("it carries an ACL allow entry that grants write")
            }
            ArtifactFault::NotImmutable => f.write_str("it has neither uchg nor schg set"),
            ArtifactFault::ImmutabilityUnverifiable => f.write_str(
                "this build can verify immutability only on macOS, through uchg or schg",
            ),
            ArtifactFault::ChangedWhileHashing => {
                f.write_str("its size or times moved while it was hashed")
            }
            ArtifactFault::Digest { found } => {
                write!(f, "its SHA-256 is {found}, not the pinned digest")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nofollow::fixtures::{
        TestRoot, facts, make_dir, make_fifo, within_a_second, write_file,
    };
    use crate::nofollow::{Kind, Reached, WalkRefusal, effective_uid};
    use crate::protocol::ErrorCode;
    use crate::runtime::recipe::Artifact;
    use plasmosome_backend::Digest;
    use serde_json::json;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};

    const BYTES: &[u8] = b"membraned, built for this test";

    fn pinned(path: &Path, bytes: &[u8]) -> Artifact {
        let text = path.to_str().expect("test paths are UTF-8");
        Artifact::from_value(
            &json!({"path": text, "sha256": Digest::of(bytes).hex()}),
            "artifact",
        )
        .expect("the test pins a well-formed artifact")
    }

    fn verify_in(root: &TestRoot, relative: &str, bytes: &[u8]) -> Result<(), ArtifactRefusal> {
        verify_walk(root.walk(relative), &pinned(&root.join(relative), bytes))
    }

    fn fault_in(root: &TestRoot, relative: &str, bytes: &[u8]) -> ArtifactFault {
        let refusal = verify_in(root, relative, bytes).expect_err("the artifact is refused");
        assert_eq!(
            refusal.path.as_str(),
            root.join(relative).to_str().expect("UTF-8"),
            "the refusal names the artifact's path"
        );
        refusal.reason
    }

    fn with_file(mode: u32) -> TestRoot {
        let root = TestRoot::new();
        make_dir(&root.join("lib"), 0o755);
        write_file(&root.join("lib/membraned"), BYTES, mode);
        root
    }

    fn reached(path: &Path, kind: Kind, uid: u32, mode: u32) -> Result<(), ArtifactFault> {
        judge(
            &Reached {
                path,
                facts: facts(kind, uid, mode),
                parent: Some(facts(Kind::Directory, uid, 0o755)),
            },
            effective_uid(),
        )
    }

    fn stranger() -> u32 {
        if effective_uid() == 4242 { 4243 } else { 4242 }
    }

    #[test]
    fn the_judge_accepts_a_regular_file_of_root_or_the_effective_uid_with_no_write_or_set_id_bit() {
        let path = PathBuf::from("/opt/membraned");
        let euid = effective_uid();
        for (row, (uid, mode)) in [
            (0, 0o555),
            (0, 0o444),
            (euid, 0o444),
            (euid, 0o400),
            (euid, 0o000),
            (euid, 0o1555),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                reached(&path, Kind::Regular, uid, mode),
                Ok(()),
                "row {row}, mode {mode:04o}"
            );
        }
    }

    #[test]
    fn the_judge_refuses_another_kind_then_a_foreign_owner_then_a_write_bit_then_a_set_id_bit() {
        use ArtifactFault::{ForeignOwner, NotRegular, SetIdBits, Writable};
        let path = PathBuf::from("/opt/membraned");
        let euid = effective_uid();
        let stranger = stranger();
        for kind in [Kind::Symlink, Kind::Other, Kind::Directory, Kind::Socket] {
            assert_eq!(
                reached(&path, kind, stranger, 0o6777),
                Err(NotRegular { kind }),
                "{kind:?}"
            );
        }
        for (row, (uid, mode, fault)) in [
            (stranger, 0o444, ForeignOwner { uid: stranger }),
            (stranger, 0o6777, ForeignOwner { uid: stranger }),
            (euid, 0o644, Writable { mode: 0o644 }),
            (euid, 0o755, Writable { mode: 0o755 }),
            (euid, 0o600, Writable { mode: 0o600 }),
            (euid, 0o200, Writable { mode: 0o200 }),
            (0, 0o464, Writable { mode: 0o464 }),
            (0, 0o446, Writable { mode: 0o446 }),
            (euid, 0o4755, Writable { mode: 0o4755 }),
            (euid, 0o4555, SetIdBits { mode: 0o4555 }),
            (0, 0o2555, SetIdBits { mode: 0o2555 }),
            (euid, 0o6444, SetIdBits { mode: 0o6444 }),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                reached(&path, Kind::Regular, uid, mode),
                Err(fault),
                "row {row}, mode {mode:04o}"
            );
        }
    }

    #[test]
    fn a_file_with_a_write_bit_or_a_set_id_bit_is_refused_whatever_its_digest() {
        use ArtifactFault::{SetIdBits, Writable};
        for (mode, fault) in [
            (0o644, Writable { mode: 0o644 }),
            (0o755, Writable { mode: 0o755 }),
            (0o600, Writable { mode: 0o600 }),
            (0o664, Writable { mode: 0o664 }),
            (0o646, Writable { mode: 0o646 }),
            (0o620, Writable { mode: 0o620 }),
            (0o602, Writable { mode: 0o602 }),
            (0o4555, SetIdBits { mode: 0o4555 }),
        ] {
            let root = with_file(mode);
            for pinned_bytes in [BYTES, b"other bytes".as_slice()] {
                let found = fault_in(&root, "lib/membraned", pinned_bytes);
                assert_eq!(
                    found, fault,
                    "mode {mode:04o}: nothing is trusted after hashing"
                );
                let shown = found.to_string();
                assert!(shown.contains(&format!("mode {mode:04o}")), "{shown}");
            }
        }
    }

    #[test]
    fn a_group_writable_directory_anywhere_above_the_file_is_refused() {
        let root = with_file(0o444);
        let lib = root.join("lib");
        crate::nofollow::fixtures::set_mode(&lib, 0o775);
        assert_eq!(
            fault_in(&root, "lib/membraned", BYTES),
            ArtifactFault::Walk(WalkRefusal::Replaceable {
                path: lib,
                mode: 0o775
            })
        );

        let root = TestRoot::new();
        make_dir(&root.join("a"), 0o757);
        make_dir(&root.join("a/b"), 0o755);
        write_file(&root.join("a/b/membraned"), BYTES, 0o444);
        assert_eq!(
            fault_in(&root, "a/b/membraned", BYTES),
            ArtifactFault::Walk(WalkRefusal::Replaceable {
                path: root.join("a"),
                mode: 0o757
            })
        );
    }

    #[test]
    fn a_symlinked_directory_or_a_final_symlink_is_refused_at_the_symlink() {
        let root = with_file(0o444);
        symlink(root.join("lib"), root.join("link")).expect("symlink");
        symlink(root.join("lib/membraned"), root.join("alias")).expect("symlink");
        assert_eq!(
            fault_in(&root, "link/membraned", BYTES),
            ArtifactFault::Walk(WalkRefusal::Symlink {
                path: root.join("link")
            })
        );
        assert_eq!(
            fault_in(&root, "alias", BYTES),
            ArtifactFault::Walk(WalkRefusal::Symlink {
                path: root.join("alias")
            })
        );
    }

    #[test]
    fn a_missing_file_is_refused_naming_it() {
        let root = with_file(0o444);
        assert_eq!(
            fault_in(&root, "lib/absent", BYTES),
            ArtifactFault::Walk(WalkRefusal::Missing {
                path: root.join("lib/absent")
            })
        );
    }

    #[test]
    fn a_fifo_at_the_path_is_refused_within_a_second() {
        let root = with_file(0o444);
        let fifo = root.join("lib/fifo");
        make_fifo(&fifo);
        let walk = root.walk("lib/fifo");
        let artifact = pinned(&fifo, BYTES);
        let outcome = within_a_second(&fifo, move || verify_walk(walk, &artifact))
            .expect("a FIFO does not block the check");
        assert_eq!(
            outcome.map_err(|refusal| refusal.reason),
            Err(ArtifactFault::NotRegular { kind: Kind::Other })
        );
    }

    #[test]
    fn a_directory_or_a_socket_at_the_path_is_not_a_regular_file() {
        let root = with_file(0o444);
        make_dir(&root.join("lib/dir"), 0o555);
        let _listener = UnixListener::bind(root.join("lib/sock")).expect("bind");
        assert_eq!(
            fault_in(&root, "lib/dir", BYTES),
            ArtifactFault::NotRegular {
                kind: Kind::Directory
            }
        );
        assert_eq!(
            fault_in(&root, "lib/sock", BYTES),
            ArtifactFault::NotRegular { kind: Kind::Socket }
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_a_read_only_file_with_its_digest_is_refused_as_immutability_unverifiable() {
        let root = TestRoot::walkable_from_root();
        for (name, mode) in [("membraned", 0o444), ("helper", 0o555)] {
            let path = root.join(name);
            write_file(&path, BYTES, mode);
            for pinned_bytes in [BYTES, b"other bytes".as_slice()] {
                let refusal = verify_artifact(&pinned(&path, pinned_bytes))
                    .expect_err("off macOS no artifact is trusted");
                assert_eq!(
                    refusal.reason,
                    ArtifactFault::ImmutabilityUnverifiable,
                    "mode {mode:04o}"
                );
            }
        }
    }

    #[test]
    fn a_size_modification_time_or_change_time_that_moved_while_hashing_is_refused() {
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
                modified: (101, 1),
                ..before
            },
            Stamp {
                changed: (200, 3),
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
    fn the_hash_reads_at_most_one_byte_past_the_size_that_was_checked() {
        let bytes = b"0123456789";
        assert_eq!(
            digest_within(&bytes[..], 4).expect("an in-memory read"),
            Digest::of(b"01234")
        );
        assert_eq!(
            digest_within(&bytes[..], 10).expect("an in-memory read"),
            Digest::of(bytes)
        );
        assert_eq!(
            digest_within(&bytes[..], u64::MAX).expect("an in-memory read"),
            Digest::of(bytes)
        );
    }

    #[test]
    fn a_refusal_names_the_artifact_and_reads_on_the_wire_as_a_108_with_its_path() {
        let root = with_file(0o664);
        let refusal = verify_in(&root, "lib/membraned", BYTES).expect_err("refused");
        let path = root.join("lib/membraned");
        let shown = path.to_str().expect("UTF-8");
        let reason = refusal.reason.to_string();
        assert_eq!(
            refusal.to_string(),
            format!("{shown} is not a trusted artifact: {reason}")
        );
        let wire = refusal.wire();
        assert_eq!(wire.code(), ErrorCode::ManifestInvalid);
        assert_eq!(
            serde_json::to_value(&wire).expect("a wire error serializes"),
            json!({
                "code": 108,
                "message": format!("`{shown}` is not a trusted artifact: {reason}"),
                "detail": reason,
                "path": shown,
            }),
            "the 108 carries exactly code, message, detail and path"
        );
    }

    #[test]
    fn each_fault_says_what_was_found() {
        let euid = effective_uid();
        for (fault, text) in [
            (
                ArtifactFault::NotRegular { kind: Kind::Other },
                "it is a FIFO, a device or another special file, not a regular file".to_owned(),
            ),
            (
                ArtifactFault::ForeignOwner { uid: 4242 },
                format!("it is owned by uid 4242, neither root nor the controller's uid {euid}"),
            ),
            (
                ArtifactFault::Writable { mode: 0o644 },
                "it has mode 0644, with a write bit".to_owned(),
            ),
            (
                ArtifactFault::SetIdBits { mode: 0o4555 },
                "it has mode 4555, with a set-user-ID or set-group-ID bit".to_owned(),
            ),
            (
                ArtifactFault::WriteAcl,
                "it carries an ACL allow entry that grants write".to_owned(),
            ),
            (
                ArtifactFault::NotImmutable,
                "it has neither uchg nor schg set".to_owned(),
            ),
            (
                ArtifactFault::ImmutabilityUnverifiable,
                "this build can verify immutability only on macOS, through uchg or schg".to_owned(),
            ),
            (
                ArtifactFault::ChangedWhileHashing,
                "its size or times moved while it was hashed".to_owned(),
            ),
            (
                ArtifactFault::Digest {
                    found: Digest::of(b""),
                },
                format!(
                    "its SHA-256 is {}, not the pinned digest",
                    Digest::of(b"").hex()
                ),
            ),
            (
                ArtifactFault::Walk(WalkRefusal::Missing {
                    path: PathBuf::from("/opt/x"),
                }),
                "/opt/x does not exist".to_owned(),
            ),
        ] {
            assert_eq!(fault.to_string(), text);
        }
    }

    #[cfg(target_os = "macos")]
    mod macos {
        use super::*;
        use crate::nofollow::fixtures::{add_acl, set_flags, set_mode};
        use std::fs::{self, OpenOptions};
        use std::io::Write;
        use std::os::unix::fs::FileExt;
        use std::time::Duration;

        fn with_immutable_file(mode: u32) -> TestRoot {
            let root = with_file(mode);
            set_flags(&root.join("lib/membraned"), libc::UF_IMMUTABLE);
            root
        }

        #[test]
        fn uchg_or_schg_marks_a_file_immutable_and_no_other_flag_does() {
            for (flags, immutable) in [
                (0, false),
                (libc::UF_IMMUTABLE, true),
                (libc::SF_IMMUTABLE, true),
                (libc::UF_IMMUTABLE | libc::SF_IMMUTABLE, true),
                (libc::UF_NODUMP | libc::UF_IMMUTABLE, true),
                (libc::UF_NODUMP, false),
                (libc::UF_APPEND, false),
                (libc::SF_APPEND, false),
                (libc::UF_HIDDEN, false),
            ] {
                assert_eq!(is_immutable(flags), immutable, "flags {flags:#x}");
            }
        }

        #[test]
        fn a_read_only_file_with_uchg_and_its_digest_is_verified() {
            for mode in [0o444, 0o555, 0o400] {
                let root = with_immutable_file(mode);
                assert_eq!(
                    verify_in(&root, "lib/membraned", BYTES),
                    Ok(()),
                    "mode {mode:04o}"
                );
            }
        }

        #[test]
        fn verify_artifact_walks_the_artifact_path_from_the_root() {
            let root = TestRoot::walkable_from_root();
            let path = root.join("membraned");
            write_file(&path, BYTES, 0o444);
            set_flags(&path, libc::UF_IMMUTABLE);
            assert_eq!(verify_artifact(&pinned(&path, BYTES)), Ok(()));
            let refusal = verify_artifact(&pinned(&path, b"other bytes"))
                .expect_err("a wrong digest is refused");
            assert_eq!(
                refusal.reason,
                ArtifactFault::Digest {
                    found: Digest::of(BYTES)
                }
            );
        }

        #[test]
        fn a_wrong_digest_is_refused_with_the_digest_found() {
            let root = with_immutable_file(0o444);
            assert_eq!(
                fault_in(&root, "lib/membraned", b"the bytes that were pinned"),
                ArtifactFault::Digest {
                    found: Digest::of(BYTES)
                }
            );
        }

        #[test]
        fn a_read_only_file_without_uchg_is_refused_whatever_its_digest() {
            for mode in [0o444, 0o555] {
                let root = with_file(mode);
                for pinned_bytes in [BYTES, b"other bytes".as_slice()] {
                    let found = fault_in(&root, "lib/membraned", pinned_bytes);
                    assert_eq!(found, ArtifactFault::NotImmutable, "mode {mode:04o}");
                    assert!(found.to_string().contains("uchg"), "{found}");
                }
            }
        }

        #[test]
        fn uchg_does_not_excuse_a_write_bit_or_a_set_id_bit() {
            for (mode, fault) in [
                (0o644, ArtifactFault::Writable { mode: 0o644 }),
                (0o755, ArtifactFault::Writable { mode: 0o755 }),
                (0o4555, ArtifactFault::SetIdBits { mode: 0o4555 }),
            ] {
                let root = with_immutable_file(mode);
                assert_eq!(
                    fault_in(&root, "lib/membraned", BYTES),
                    fault,
                    "mode {mode:04o}"
                );
            }
        }

        #[test]
        fn a_write_acl_is_refused_before_immutability_and_before_the_digest() {
            let root = with_file(0o444);
            let path = root.join("lib/membraned");
            add_acl(&path, "everyone deny write");
            set_flags(&path, libc::UF_IMMUTABLE);
            assert_eq!(verify_in(&root, "lib/membraned", BYTES), Ok(()));
            set_flags(&path, 0);
            add_acl(&path, "everyone allow write");
            assert_eq!(
                fault_in(&root, "lib/membraned", BYTES),
                ArtifactFault::WriteAcl,
                "without uchg"
            );
            set_flags(&path, libc::UF_IMMUTABLE);
            for pinned_bytes in [BYTES, b"other bytes".as_slice()] {
                assert_eq!(
                    fault_in(&root, "lib/membraned", pinned_bytes),
                    ArtifactFault::WriteAcl,
                    "nothing is hashed before the ACL is checked"
                );
            }
        }

        fn grow_by_two_bytes(path: &Path) {
            set_flags(path, 0);
            set_mode(path, 0o644);
            OpenOptions::new()
                .append(true)
                .open(path)
                .and_then(|mut file| file.write_all(b"XY"))
                .expect("the file grows by two bytes");
        }

        fn after_the_clock_moves() {
            std::thread::sleep(Duration::from_millis(20));
        }

        #[test]
        fn the_hash_reads_the_checked_descriptor_and_stops_one_byte_past_the_checked_size() {
            let root = with_immutable_file(0o444);
            let path = root.join("lib/membraned");
            let checked =
                open_checked(root.walk("lib/membraned"), &path).expect("the file passes the rule");
            grow_by_two_bytes(&path);
            write_file(&root.join("lib/other"), b"another file", 0o444);
            fs::rename(root.join("lib/other"), &path).expect("another file takes the path");
            assert_eq!(
                digest_checked(&checked, &path),
                Ok(Digest::of(&[BYTES, b"X"].concat()))
            );
        }

        #[test]
        fn a_held_writer_rewriting_the_same_bytes_after_the_check_is_refused_as_changed() {
            let root = with_file(0o644);
            let path = root.join("lib/membraned");
            let writer = OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("a writer opened before the file is locked");
            set_mode(&path, 0o444);
            set_flags(&path, libc::UF_IMMUTABLE);
            let checked = open_checked(root.walk("lib/membraned"), &path)
                .expect("the locked file passes the rule");
            after_the_clock_moves();
            writer
                .write_all_at(BYTES, 0)
                .expect("the held writer still writes under uchg");
            assert_eq!(
                hash_checked(checked, &path, &Digest::of(BYTES)),
                Err(ArtifactFault::ChangedWhileHashing)
            );
        }

        #[test]
        fn clearing_and_resetting_uchg_after_the_check_moves_only_ctime_and_is_refused() {
            let root = with_immutable_file(0o444);
            let path = root.join("lib/membraned");
            let checked =
                open_checked(root.walk("lib/membraned"), &path).expect("the file passes the rule");
            let times = |path: &Path| {
                let metadata = fs::metadata(path).expect("the file is still there");
                (
                    (metadata.size(), metadata.mtime(), metadata.mtime_nsec()),
                    (metadata.ctime(), metadata.ctime_nsec()),
                )
            };
            let (kept, changed) = times(&path);
            after_the_clock_moves();
            set_flags(&path, 0);
            set_flags(&path, libc::UF_IMMUTABLE);
            assert_eq!(times(&path).0, kept, "the size and the mtime stay");
            assert_ne!(times(&path).1, changed, "the ctime moves");
            assert_eq!(
                hash_checked(checked, &path, &Digest::of(BYTES)),
                Err(ArtifactFault::ChangedWhileHashing)
            );
        }

        #[test]
        fn a_file_that_grew_after_the_check_is_refused_as_changed_before_its_digest_is_compared() {
            let root = with_immutable_file(0o444);
            let path = root.join("lib/membraned");
            let checked =
                open_checked(root.walk("lib/membraned"), &path).expect("the file passes the rule");
            grow_by_two_bytes(&path);
            assert_eq!(
                hash_checked(checked, &path, &Digest::of(BYTES)),
                Err(ArtifactFault::ChangedWhileHashing)
            );
        }

        #[test]
        fn a_path_that_names_another_file_after_the_check_leaves_the_checked_file_verified() {
            let root = with_immutable_file(0o444);
            let path = root.join("lib/membraned");
            let checked =
                open_checked(root.walk("lib/membraned"), &path).expect("the file passes the rule");
            fs::rename(root.join("lib"), root.join("lib.old")).expect("the directory moves away");
            make_dir(&root.join("lib"), 0o755);
            write_file(&path, b"another file", 0o444);
            assert_eq!(hash_checked(checked, &path, &Digest::of(BYTES)), Ok(()));
        }
    }
}
