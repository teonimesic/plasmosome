use std::fmt;
use std::fs::File;
use std::path::PathBuf;

use plasmosome_core::artifact::{CheckedFile, Stamp, check_file, verify_file};
use plasmosome_core::nofollow::{Walk, WalkRefusal};

use super::digest::Digest;
use super::path::RecipePath;
use super::recipe::{Artifact, ArtifactRole, RuntimeRecipe};

#[cfg_attr(
    not(target_os = "macos"),
    allow(
        unused_imports,
        reason = "off macOS nothing in this crate reads through it yet"
    )
)]
pub(crate) use plasmosome_core::artifact::Positioned;

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
    stamp: Stamp,
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
        self.stamp.size()
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

    /// Opens the path again from the same "/" descriptor and applies every check except the hash,
    /// with core's `artifact::check_file`. Then it compares the device, inode, size, modification
    /// time and change time with the verified ones. A path that now names another file, even one
    /// with the same bytes, is `Replaced`. A change in place that moved the size or a time, such
    /// as a write through a descriptor opened for writing before the file was locked, is
    /// `ChangedSinceVerification`.
    ///
    /// It sees a change to the file's bytes only if the change moves the device, inode, size,
    /// modification time or change time. A shared mapping made before the file was locked, from
    /// a descriptor opened for writing, can change the bytes whatever protection it had when the
    /// file was locked, because `mprotect` can add write later. Such a write moves the times only
    /// at `msync` or `munmap`, if at all, so neither this nor [`verify_artifact`] sees it. Nor
    /// does it see a change made after it returns.
    pub fn reconfirm(&self) -> Result<(), ArtifactRefusal> {
        let refuse = |fault| ArtifactRefusal {
            role: self.role,
            path: self.path.as_path().to_path_buf(),
            fault,
        };
        let checked = check_file(&self.walk).map_err(|fault| refuse(fault.into()))?;
        let found = checked_identity(&checked);
        if found != self.identity {
            return Err(refuse(ArtifactFault::Replaced {
                verified: self.identity,
                found,
            }));
        }
        if checked.stamp() != self.stamp {
            return Err(refuse(ArtifactFault::ChangedSinceVerification));
        }
        Ok(())
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
    /// A rule of core's file judge, the one the controller applies before `cell.new` allocates a
    /// cell: the walk from "/", the kind, the owner, the write and set-id bits, the ACL,
    /// immutability, a change while hashing, or the digest. Core's `artifact::ArtifactFault`
    /// says what each means.
    Rule(plasmosome_core::artifact::ArtifactFault),
    /// The artifact's size, modification time or change time moved after it was verified, so
    /// something changed it in place.
    ChangedSinceVerification,
    /// The path now names another file than the one verified.
    Replaced {
        verified: FileIdentity,
        found: FileIdentity,
    },
}

impl From<plasmosome_core::artifact::ArtifactFault> for ArtifactFault {
    fn from(fault: plasmosome_core::artifact::ArtifactFault) -> Self {
        ArtifactFault::Rule(fault)
    }
}

impl From<WalkRefusal> for ArtifactFault {
    fn from(refusal: WalkRefusal) -> Self {
        ArtifactFault::Rule(refusal.into())
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
            ArtifactFault::Rule(fault) => write!(f, "{fault}"),
            ArtifactFault::ChangedSinceVerification => {
                f.write_str("its size or times moved after it was verified")
            }
            ArtifactFault::Replaced { verified, found } => write!(
                f,
                "the path names device {} inode {}, not the verified device {} inode {}",
                found.dev, found.ino, verified.dev, verified.ino
            ),
        }
    }
}

/// Verifies one pinned artifact as spec 001 §4.2 asks, before anything trusts its bytes.
///
/// Walks `artifact.path()` from "/" with `Walk::from_root`, then judges and hashes the file with
/// core's `artifact::verify_file`, the check the controller runs before `cell.new` allocates a
/// cell, so a file the controller admits is one this admits. Core's README, "Artifacts", gives
/// the rule. This keeps the walk, the open descriptor and the stamp taken at the check, so
/// [`VerifiedArtifact::reconfirm`] can open the path again from the same "/".
///
/// On macOS pass the canonical path, `/private/var/...` rather than `/var/...`: a symlink
/// anywhere on the path is refused. Root and the effective UID can still change an artifact
/// after it passes. So can a process that opened it for writing before it was locked, through
/// that descriptor or through a shared mapping made from it, whatever protection the mapping had
/// then: the mode and `uchg` refuse only new opens and new writable mappings, `mprotect` can
/// still add write to a mapping made before the lock, and this process cannot see another
/// process's descriptors or mappings. Such a writer can also extend the file before the check,
/// and the hash reads up to the checked size, so it can make verification slow.
/// [`VerifiedArtifact::reconfirm`] refuses a change made before it runs if the change moved the
/// size or a time. A write through such a mapping moves neither until `msync` or `munmap`, if at
/// all, so neither this check nor `reconfirm` sees it.
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
    let checked = verify_file(&walk, artifact.sha256()).map_err(|fault| refuse(fault.into()))?;
    Ok(VerifiedArtifact {
        role,
        path: artifact.path().clone(),
        walk,
        identity: checked_identity(&checked),
        stamp: checked.stamp(),
        digest: *artifact.sha256(),
        file: checked.into_file(),
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

fn checked_identity(checked: &CheckedFile) -> FileIdentity {
    let facts = checked.facts();
    FileIdentity {
        dev: facts.dev,
        ino: facts.ino,
    }
}

#[cfg(all(test, not(target_os = "macos")))]
pub(crate) use tests::verified_for_test;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::strict_json;
    use crate::runtime::test_fixtures::tree::{ArtifactTree, make_fifo};
    use plasmosome_core::artifact::ArtifactFault as CoreFault;
    use plasmosome_core::nofollow::Kind;
    use std::os::unix::fs::{OpenOptionsExt, symlink};
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::Duration;

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

    fn fault_of(outcome: Result<VerifiedArtifact, ArtifactRefusal>) -> CoreFault {
        match outcome {
            Ok(verified) => panic!("{} verified", verified.path().as_str()),
            Err(ArtifactRefusal {
                fault: ArtifactFault::Rule(fault),
                ..
            }) => fault,
            Err(refusal) => panic!("{:?} is not a rule of core's judge", refusal.fault),
        }
    }

    fn walk_fault(path: &Path) -> CoreFault {
        fault_of(verify_pinned(path, &Digest::of(b"kernel")))
    }

    #[cfg(not(target_os = "macos"))]
    pub(crate) fn verified_for_test(role: ArtifactRole, path: &Path) -> VerifiedArtifact {
        use std::os::unix::fs::MetadataExt;
        let file = File::open(path).expect("the test file opens");
        let metadata = file.metadata().expect("fstat");
        VerifiedArtifact {
            role,
            path: RecipePath::parse(path.to_str().expect("UTF-8")).expect("a canonical path"),
            walk: Walk::from_root(path).expect("the walk starts"),
            identity: FileIdentity {
                dev: metadata.dev(),
                ino: metadata.ino(),
            },
            stamp: Stamp::of(&metadata),
            digest: Digest::of(b""),
            file,
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
            CoreFault::Walk(WalkRefusal::Symlink {
                path: tree.join("link")
            })
        );
        assert_eq!(
            walk_fault(&tree.join("kernel-link")),
            CoreFault::Walk(WalkRefusal::Symlink {
                path: tree.join("kernel-link")
            })
        );
    }

    #[test]
    fn a_missing_name_refuses_where_the_walk_stopped() {
        let tree = ArtifactTree::new();
        assert_eq!(
            walk_fault(&tree.join("absent/kernel")),
            CoreFault::Walk(WalkRefusal::Missing {
                path: tree.join("absent")
            })
        );
        assert_eq!(
            walk_fault(&tree.join("kernel")),
            CoreFault::Walk(WalkRefusal::Missing {
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
                CoreFault::Walk(WalkRefusal::Replaceable { path: dir, mode }),
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
            CoreFault::NotRegular {
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
            CoreFault::NotRegular { kind: Kind::Other }
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_a_read_only_file_is_refused_as_immutability_unverifiable() {
        let tree = ArtifactTree::new();
        let path = tree.file("kernel", b"kernel", 0o444);
        assert_eq!(
            fault_of(verify_pinned(&path, &Digest::of(b"kernel"))),
            CoreFault::ImmutabilityUnverifiable
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
        let replaced = Replaced {
            verified: FileIdentity { dev: 7, ino: 8 },
            found: FileIdentity { dev: 7, ino: 9 },
        };
        let rows = [
            (
                Rule(CoreFault::NotImmutable),
                "it has neither uchg nor schg set",
            ),
            (ChangedSinceVerification, "moved after it was verified"),
            (replaced, "inode 9, not the verified device 7 inode 8"),
        ];
        for (fault, part) in rows {
            let shown = fault.to_string();
            assert!(shown.contains(part), "{shown:?} names {part}");
        }
    }

    #[cfg(target_os = "macos")]
    mod macos {
        use super::*;
        use crate::runtime::test_fixtures::tree::{add_acl, set_flags, set_mode};
        use crate::runtime::test_fixtures::{recipe, recipe_value};
        use std::fs;
        use std::io::Seek;
        use std::os::fd::AsRawFd;
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

        fn reconfirmed(verified: &VerifiedArtifact) -> Result<(), ArtifactFault> {
            verified.reconfirm().map_err(|refusal| refusal.fault)
        }

        fn by_rule(fault: CoreFault) -> Result<(), ArtifactFault> {
            Err(ArtifactFault::Rule(fault))
        }

        #[allow(unsafe_code)]
        fn close_on_exec(file: &File) -> bool {
            // SAFETY: F_GETFD only reads the flags of a descriptor this File keeps open.
            let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
            flags != -1 && flags & libc::FD_CLOEXEC != 0
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

        const ROLES: [(ArtifactRole, &str, &str); 8] = [
            (ArtifactRole::Kernel, "kernel", "kernel"),
            (ArtifactRole::Initramfs, "initramfs", "initramfs"),
            (ArtifactRole::RootImage, "root_image", "rootfs.raw"),
            (ArtifactRole::Helper, "helper", "krun-helper"),
            (ArtifactRole::Library(0), "libraries", "libkrun.dylib"),
            (ArtifactRole::Library(1), "libraries", "libkrunfw.dylib"),
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

        fn recipe_pinning(bundle: &Bundle, tampered: &[ArtifactRole]) -> RuntimeRecipe {
            let mut value = recipe_value("/opt/cell/helper");
            value["libraries"] = serde_json::json!([]);
            for ((role, field, name), (_, path)) in ROLES.iter().zip(&bundle.paths) {
                let pinned = if tampered.contains(role) {
                    Digest::of(b"tampered")
                } else {
                    Digest::of(name.as_bytes())
                };
                let artifact = serde_json::json!({
                    "path": path.to_str().expect("UTF-8"),
                    "sha256": pinned.hex(),
                });
                match value[*field].as_array_mut() {
                    Some(libraries) => libraries.push(artifact),
                    None => value[*field] = artifact,
                }
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
                CoreFault::Walk(WalkRefusal::Symlink {
                    path: "/var".into()
                })
            );
        }

        #[test]
        fn a_wrong_pin_refuses_with_the_digest_found() {
            let tree = ArtifactTree::new();
            let path = tree.immutable_file("kernel", b"kernel");
            assert_eq!(
                fault_of(verify_pinned(&path, &Digest::of(b"another kernel"))),
                CoreFault::Digest {
                    found: Digest::of(b"kernel")
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
                CoreFault::NotImmutable
            );
            assert_eq!(
                fault_of(verify_pinned(&writable, &pin)),
                CoreFault::Writable { mode: 0o644 }
            );
            assert_eq!(
                fault_of(verify_pinned(&set_id, &pin)),
                CoreFault::SetIdBits { mode: 0o4555 }
            );
        }

        #[test]
        fn an_acl_entry_granting_write_refuses_with_or_without_uchg() {
            let tree = ArtifactTree::new();
            let path = tree.file("kernel", b"kernel", 0o444);
            add_acl(&path, "everyone allow write");
            let unlocked = tree.file("unlocked", b"kernel", 0o444);
            add_acl(&unlocked, "everyone allow write");
            set_flags(&path, libc::UF_IMMUTABLE);
            assert_eq!(
                fault_of(verify_pinned(&path, &Digest::of(b"kernel"))),
                CoreFault::WriteAcl
            );
            assert_eq!(
                fault_of(verify_pinned(&unlocked, &Digest::of(b"kernel"))),
                CoreFault::WriteAcl
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
        fn reconfirm_refuses_a_file_a_held_writer_rewrote_after_verification() {
            let tree = ArtifactTree::new();
            let path = tree.file("kernel", b"kernel", 0o644);
            let writer = fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("a writer opened before the lock");
            set_mode(&path, 0o444);
            set_flags(&path, libc::UF_IMMUTABLE);
            let verified = verify_pinned(&path, &Digest::of(b"kernel")).expect("verifies");
            writer
                .write_all_at(b"kernel", 0)
                .expect("the held writer still writes");
            assert_eq!(
                verified.reconfirm().map_err(|refusal| refusal.fault),
                Err(ArtifactFault::ChangedSinceVerification)
            );
        }

        #[test]
        fn reconfirm_refuses_an_owner_rewrite_that_kept_the_size_and_restored_the_mtime() {
            let tree = ArtifactTree::new();
            let path = tree.immutable_file("kernel", b"kernel");
            let verified = verify_pinned(&path, &Digest::of(b"kernel")).expect("verifies");
            let before = fs::metadata(&path).expect("stat");
            set_flags(&path, 0);
            set_mode(&path, 0o644);
            let writer = fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("the owner opens it for writing");
            writer.write_all_at(b"KERNEL", 0).expect("rewrite");
            writer
                .set_modified(before.modified().expect("mtime"))
                .expect("restore the mtime");
            drop(writer);
            set_mode(&path, 0o444);
            set_flags(&path, libc::UF_IMMUTABLE);
            let after = fs::metadata(&path).expect("stat");
            let size_and_mtime = |m: &fs::Metadata| (m.len(), m.mtime(), m.mtime_nsec());
            assert_eq!(size_and_mtime(&after), size_and_mtime(&before));
            assert_ne!(
                (after.ctime(), after.ctime_nsec()),
                (before.ctime(), before.ctime_nsec())
            );
            assert_eq!(
                reconfirmed(&verified),
                Err(ArtifactFault::ChangedSinceVerification)
            );
        }

        #[test]
        fn reconfirm_closes_the_file_it_opens_whether_it_passes_or_refuses() {
            let tree = ArtifactTree::new();
            let path = tree.immutable_file("kernel", b"kernel");
            let verified = verify_pinned(&path, &Digest::of(b"kernel")).expect("verifies");
            let identity = [verified.identity()];
            assert_eq!(descriptors_naming(&identity).len(), 1);
            for _ in 0..50 {
                assert_eq!(reconfirmed(&verified), Ok(()));
            }
            assert_eq!(descriptors_naming(&identity).len(), 1, "after it passed");
            set_flags(&path, 0);
            for _ in 0..50 {
                assert_eq!(reconfirmed(&verified), by_rule(CoreFault::NotImmutable));
            }
            assert_eq!(descriptors_naming(&identity).len(), 1, "after it refused");
        }

        #[test]
        fn reconfirm_refuses_a_file_whose_uchg_was_cleared_or_that_was_made_writable() {
            let tree = ArtifactTree::new();
            let path = tree.immutable_file("kernel", b"kernel");
            let verified = verify_pinned(&path, &Digest::of(b"kernel")).expect("verifies");
            set_flags(&path, 0);
            assert_eq!(reconfirmed(&verified), by_rule(CoreFault::NotImmutable));
            set_mode(&path, 0o644);
            set_flags(&path, libc::UF_IMMUTABLE);
            let writable = CoreFault::Writable { mode: 0o644 };
            assert_eq!(reconfirmed(&verified), by_rule(writable));
        }

        #[test]
        fn reconfirm_refuses_a_parent_made_group_writable() {
            let tree = ArtifactTree::new();
            let boot = tree.dir("boot", 0o700);
            let path = tree.immutable_file("boot/kernel", b"kernel");
            let verified = verify_pinned(&path, &Digest::of(b"kernel")).expect("verifies");
            set_mode(&boot, 0o775);
            assert_eq!(
                reconfirmed(&verified),
                by_rule(CoreFault::Walk(WalkRefusal::Replaceable {
                    path: boot,
                    mode: 0o775
                }))
            );
        }

        #[test]
        fn reconfirm_refuses_a_symlinked_parent_even_when_it_reaches_the_same_inode() {
            let tree = ArtifactTree::new();
            let boot = tree.dir("boot", 0o700);
            let other = tree.dir("other", 0o700);
            let path = tree.file("boot/kernel", b"kernel", 0o444);
            fs::hard_link(&path, other.join("kernel")).expect("a link made before the lock");
            set_flags(&path, libc::UF_IMMUTABLE);
            let verified = verify_pinned(&path, &Digest::of(b"kernel")).expect("verifies");
            fs::rename(&boot, tree.join("boot.old")).expect("rename");
            symlink(&other, &boot).expect("symlink");
            assert_eq!(identity_of(&path), verified.identity());
            assert_eq!(
                reconfirmed(&verified),
                by_rule(CoreFault::Walk(WalkRefusal::Symlink { path: boot }))
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
            let verified = verify_recipe_artifacts(&recipe_pinning(&bundle, &[]))
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
            for artifact in &all {
                assert!(close_on_exec(artifact.file()), "{}", artifact.role());
            }
            assert_eq!(descriptors_naming(&bundle.identities()).len(), 8);
            drop(verified);
            assert_eq!(descriptors_naming(&bundle.identities()), Vec::<i32>::new());
        }

        #[test]
        fn of_two_tampered_artifacts_the_first_in_field_order_is_reported() {
            let bundle = bundle();
            let tampered = [ArtifactRole::GuestPolicy, ArtifactRole::RootImage];
            let refusal = verify_recipe_artifacts(&recipe_pinning(&bundle, &tampered))
                .expect_err("two artifacts are tampered");
            assert_eq!(refusal.role, ArtifactRole::RootImage);
            let library = [ArtifactRole::Library(1)];
            let refusal = verify_recipe_artifacts(&recipe_pinning(&bundle, &library))
                .expect_err("the second library is tampered");
            assert_eq!(refusal.role, ArtifactRole::Library(1));
        }

        #[test]
        fn each_role_is_reported_when_it_and_every_role_after_it_are_tampered() {
            let bundle = bundle();
            for (index, (role, _, _)) in ROLES.iter().enumerate() {
                let tampered: Vec<ArtifactRole> =
                    ROLES[index..].iter().map(|(role, _, _)| *role).collect();
                let refusal = verify_recipe_artifacts(&recipe_pinning(&bundle, &tampered))
                    .expect_err("an artifact is tampered");
                assert_eq!(refusal.role, *role, "{tampered:?}");
            }
        }
    }
}
