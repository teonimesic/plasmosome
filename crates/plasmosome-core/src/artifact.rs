use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::AsFd;
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
    /// Group or other can write the file; `mode` holds its permission bits.
    Writable { mode: u32 },
    /// A macOS ACL allow entry on the file grants write to someone.
    WriteAcl,
    /// The file's bytes hash to `found`, not to the pinned digest.
    Digest { found: Digest },
}

/// Checks one Artifact before the controller trusts it, as spec 023 §1 says for `membraned` and
/// every Artifact of the template. Walks its path from `/` without following a symlink, and
/// refuses unless every directory above the file is owned by root or the effective UID, writable
/// by neither group nor other and, on macOS, carries no ACL entry that lets anyone replace its
/// entries. The file, opened without following and without blocking, must be a regular file
/// owned by root or the effective UID, writable by neither group nor other, with no ACL allow
/// entry granting write. Only then is it read, through the descriptor that was checked, and
/// hashed: a file that breaks a rule is refused, never trusted because its digest matches. The
/// read stops one byte past the size `fstat` reported, so it ends even when the file grows while
/// it is read.
///
/// A pass describes the file as it was during the check. Whoever opens it later by path must
/// guard its own open.
pub fn verify_artifact(artifact: &Artifact) -> Result<(), ArtifactRefusal> {
    let walk = Walk::from_root(artifact.path().as_path())
        .map_err(|refusal| refused(artifact, ArtifactFault::Walk(refusal)))?;
    verify_walk(walk, artifact)
}

fn verify_walk(walk: Walk, artifact: &Artifact) -> Result<(), ArtifactRefusal> {
    check(walk, artifact.path().as_path(), artifact.sha256())
        .map_err(|reason| refused(artifact, reason))
}

fn check(walk: Walk, path: &Path, pinned: &Digest) -> Result<(), ArtifactFault> {
    let euid = effective_uid();
    let opened = walk.open_file(|reached| judge(reached, euid))?;
    let file = File::from(opened.fd);
    if acl_grants_write(file.as_fd()).map_err(|error| io_fault("acl", path, &error))? {
        return Err(ArtifactFault::WriteAcl);
    }
    let size = file
        .metadata()
        .map_err(|error| io_fault("fstat", path, &error))?
        .len();
    let found = digest_within(&file, size).map_err(|error| io_fault("read", path, &error))?;
    if found != *pinned {
        return Err(ArtifactFault::Digest { found });
    }
    Ok(())
}

fn judge(reached: &Reached<'_>, euid: u32) -> Result<(), ArtifactFault> {
    let facts = reached.facts;
    if facts.kind != Kind::Regular {
        return Err(ArtifactFault::NotRegular { kind: facts.kind });
    }
    if facts.uid != 0 && facts.uid != euid {
        return Err(ArtifactFault::ForeignOwner { uid: facts.uid });
    }
    if facts.mode & 0o022 != 0 {
        return Err(ArtifactFault::Writable { mode: facts.mode });
    }
    Ok(())
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
    /// The refusal as `cell.new` answers it: code 108 with `path` the Artifact's path and
    /// `detail` the broken rule, and no `fix`.
    pub fn wire(&self) -> WireError {
        WireError::manifest_invalid(self.reason.to_string(), self.path.as_str().to_owned())
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
                write!(f, "it has mode {mode:04o}: group or other can write it")
            }
            ArtifactFault::WriteAcl => {
                f.write_str("it carries an ACL allow entry that grants write")
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
    use serde_json::{Value, json};
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

    #[test]
    fn a_file_with_its_digest_under_owned_unwritable_directories_is_verified() {
        for mode in [0o644, 0o755, 0o600, 0o400] {
            let root = with_file(mode);
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
        write_file(&root.join("membraned"), BYTES, 0o644);
        assert_eq!(
            verify_artifact(&pinned(&root.join("membraned"), BYTES)),
            Ok(())
        );
        let refusal = verify_artifact(&pinned(&root.join("membraned"), b"other bytes"))
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
        let root = with_file(0o644);
        assert_eq!(
            fault_in(&root, "lib/membraned", b"the bytes that were pinned"),
            ArtifactFault::Digest {
                found: Digest::of(BYTES)
            }
        );
    }

    #[test]
    fn a_group_or_other_writable_file_is_refused_whatever_its_digest() {
        for mode in [0o664, 0o646, 0o666, 0o620, 0o602] {
            let root = with_file(mode);
            for pinned_bytes in [BYTES, b"other bytes".as_slice()] {
                assert_eq!(
                    fault_in(&root, "lib/membraned", pinned_bytes),
                    ArtifactFault::Writable { mode },
                    "mode {mode:04o}: nothing is trusted after hashing"
                );
            }
        }
    }

    #[test]
    fn a_group_writable_directory_anywhere_above_the_file_is_refused() {
        let root = with_file(0o644);
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
        write_file(&root.join("a/b/membraned"), BYTES, 0o644);
        assert_eq!(
            fault_in(&root, "a/b/membraned", BYTES),
            ArtifactFault::Walk(WalkRefusal::Replaceable {
                path: root.join("a"),
                mode: 0o757
            })
        );
    }

    #[test]
    fn a_symlinked_directory_or_a_final_symlink_is_refused_even_when_its_target_verifies() {
        let root = with_file(0o644);
        symlink(root.join("lib"), root.join("link")).expect("symlink");
        symlink(root.join("lib/membraned"), root.join("alias")).expect("symlink");
        assert_eq!(verify_in(&root, "lib/membraned", BYTES), Ok(()));
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
        let root = with_file(0o644);
        assert_eq!(
            fault_in(&root, "lib/absent", BYTES),
            ArtifactFault::Walk(WalkRefusal::Missing {
                path: root.join("lib/absent")
            })
        );
    }

    #[test]
    fn a_fifo_at_the_path_is_refused_within_a_second() {
        let root = with_file(0o644);
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
        let root = with_file(0o644);
        make_dir(&root.join("lib/dir"), 0o755);
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

    #[test]
    fn the_file_must_be_regular_then_owned_by_root_or_the_effective_uid_then_unwritable() {
        let path = PathBuf::from("/opt/membraned");
        let euid = effective_uid();
        let stranger = if euid == 4242 { 4243 } else { 4242 };
        assert_eq!(reached(&path, Kind::Regular, 0, 0o755), Ok(()));
        assert_eq!(reached(&path, Kind::Regular, euid, 0o644), Ok(()));
        assert_eq!(
            reached(&path, Kind::Regular, stranger, 0o644),
            Err(ArtifactFault::ForeignOwner { uid: stranger })
        );
        assert_eq!(
            reached(&path, Kind::Regular, stranger, 0o666),
            Err(ArtifactFault::ForeignOwner { uid: stranger })
        );
        assert_eq!(
            reached(&path, Kind::Regular, 0, 0o664),
            Err(ArtifactFault::Writable { mode: 0o664 })
        );
        for kind in [Kind::Symlink, Kind::Other, Kind::Directory, Kind::Socket] {
            assert_eq!(
                reached(&path, kind, stranger, 0o666),
                Err(ArtifactFault::NotRegular { kind }),
                "{kind:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_file_whose_acl_lets_anyone_write_it_is_refused() {
        use crate::nofollow::fixtures::add_acl;
        let root = with_file(0o644);
        add_acl(&root.join("lib/membraned"), "everyone deny write");
        assert_eq!(verify_in(&root, "lib/membraned", BYTES), Ok(()));
        add_acl(&root.join("lib/membraned"), "everyone allow write");
        assert_eq!(
            fault_in(&root, "lib/membraned", BYTES),
            ArtifactFault::WriteAcl
        );
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
        assert_eq!(
            refusal.to_string(),
            format!("{shown} is not a trusted artifact: {}", refusal.reason)
        );
        let wire = refusal.wire();
        assert_eq!(wire.code(), ErrorCode::ManifestInvalid);
        let value = serde_json::to_value(&wire).expect("a wire error serializes");
        assert_eq!(value["code"], Value::from(108), "{value}");
        assert_eq!(value["path"], Value::from(shown), "{value}");
        assert_eq!(
            value["detail"],
            Value::from(refusal.reason.to_string()),
            "{value}"
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
                ArtifactFault::Writable { mode: 0o664 },
                "it has mode 0664: group or other can write it".to_owned(),
            ),
            (
                ArtifactFault::WriteAcl,
                "it carries an ACL allow entry that grants write".to_owned(),
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
}
