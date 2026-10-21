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
