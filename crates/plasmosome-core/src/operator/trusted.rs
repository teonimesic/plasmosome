#[cfg(test)]
mod tests {
    use super::*;
    use crate::nofollow::fixtures::{
        TestRoot, make_dir, make_fifo, set_mode, within_a_second, write_file,
    };
    use crate::nofollow::{DirIdentity, Facts, Kind, Reached, effective_uid};
    use std::fs;
    use std::os::unix::fs::{MetadataExt, symlink};
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};

    const LIMIT: usize = 1_048_576;

    fn open_in(root: &TestRoot, relative: &str) -> Result<TrustedFile, TrustRefusal> {
        TrustedFile::from_walk(root.walk(relative))
    }

    fn refused(outcome: Result<TrustedFile, TrustRefusal>) -> TrustRefusal {
        match outcome {
            Ok(file) => panic!("{} opened", file.path().display()),
            Err(refusal) => refusal,
        }
    }

    fn check_of(outcome: Result<TrustedFile, TrustRefusal>) -> (PathBuf, TrustCheck) {
        let refusal = refused(outcome);
        (refusal.path, refusal.check)
    }

    fn identity_of(path: &Path) -> DirIdentity {
        let metadata = fs::symlink_metadata(path).expect("stat");
        DirIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }

    fn facts(kind: Kind, uid: u32, mode: u32) -> Facts {
        Facts {
            kind,
            uid,
            mode,
            dev: 1,
            ino: 2,
        }
    }

    #[test]
    fn opens_a_0600_file_in_a_0700_directory_with_its_bytes_and_directory_identities() {
        let root = TestRoot::new();
        make_dir(&root.join("dir"), 0o700);
        write_file(&root.join("dir/operator.json"), b"{\"version\": 1}", 0o600);
        let file = open_in(&root, "dir/operator.json").expect("a trusted file opens");
        assert_eq!(file.path(), root.join("dir/operator.json"));
        assert_eq!(file.bytes(), b"{\"version\": 1}");
        assert_eq!(
            file.chain(),
            [identity_of(root.path()), identity_of(&root.join("dir"))]
        );
    }

    #[test]
    fn a_final_symlink_to_a_valid_file_is_path() {
        let root = TestRoot::new();
        write_file(&root.join("real.json"), b"{}", 0o600);
        symlink("real.json", root.join("link.json")).expect("symlink");
        let refusal = refused(open_in(&root, "link.json"));
        assert_eq!(
            (refusal.path.clone(), refusal.check),
            (root.join("link.json"), TrustCheck::Path)
        );
        assert!(
            refusal.reason.contains("link.json is a symlink"),
            "{}",
            refusal.reason
        );
    }

    #[test]
    fn a_symlinked_directory_above_a_valid_file_is_path() {
        let root = TestRoot::new();
        make_dir(&root.join("real"), 0o700);
        write_file(&root.join("real/operator.json"), b"{}", 0o600);
        symlink("real", root.join("alias")).expect("symlink");
        let refusal = refused(open_in(&root, "alias/operator.json"));
        assert_eq!(
            (refusal.path.clone(), refusal.check),
            (root.join("alias/operator.json"), TrustCheck::Path)
        );
        assert!(
            refusal
                .reason
                .contains(&format!("{} is a symlink", root.join("alias").display())),
            "{}",
            refusal.reason
        );
    }

    #[test]
    fn a_file_group_or_other_can_write_is_mode() {
        let root = TestRoot::new();
        for (name, mode) in [("group.json", 0o620), ("other.json", 0o602)] {
            write_file(&root.join(name), b"{}", mode);
            assert_eq!(
                check_of(open_in(&root, name)),
                (root.join(name), TrustCheck::Mode),
                "{mode:o}"
            );
        }
    }

    #[test]
    fn a_file_with_a_second_hard_link_is_links() {
        let root = TestRoot::new();
        write_file(&root.join("one.json"), b"{}", 0o600);
        fs::hard_link(root.join("one.json"), root.join("two.json")).expect("link");
        for name in ["one.json", "two.json"] {
            assert_eq!(
                check_of(open_in(&root, name)),
                (root.join(name), TrustCheck::Links)
            );
        }
    }

    #[test]
    fn a_group_writable_parent_or_grandparent_is_ancestor() {
        let root = TestRoot::new();
        make_dir(&root.join("a"), 0o700);
        make_dir(&root.join("a/b"), 0o700);
        write_file(&root.join("a/b/operator.json"), b"{}", 0o600);
        set_mode(&root.join("a/b"), 0o770);
        assert_eq!(
            check_of(open_in(&root, "a/b/operator.json")),
            (root.join("a/b/operator.json"), TrustCheck::Ancestor)
        );
        set_mode(&root.join("a/b"), 0o700);
        set_mode(&root.join("a"), 0o707);
        let refusal = refused(open_in(&root, "a/b/operator.json"));
        assert_eq!(refusal.check, TrustCheck::Ancestor);
        assert!(
            refusal
                .reason
                .contains(&format!("{} has mode 0707", root.join("a").display())),
            "{}",
            refusal.reason
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_file_with_an_everyone_allow_write_acl_is_mode() {
        let root = TestRoot::new();
        write_file(&root.join("operator.json"), b"{}", 0o600);
        crate::nofollow::fixtures::add_acl(&root.join("operator.json"), "everyone allow write");
        assert_eq!(
            check_of(open_in(&root, "operator.json")),
            (root.join("operator.json"), TrustCheck::Mode)
        );
    }

    #[test]
    fn a_fifo_is_not_regular_within_a_second() {
        let root = TestRoot::new();
        let fifo = root.join("operator.json");
        make_fifo(&fifo);
        let walk = root.walk("operator.json");
        let outcome = within_a_second(&fifo, move || {
            TrustedFile::from_walk(walk).map(|file| file.bytes().len())
        });
        assert_eq!(
            outcome.map(|outcome| outcome.map_err(|refusal| (refusal.path, refusal.check))),
            Some(Err((fifo, TrustCheck::NotRegular)))
        );
    }

    #[test]
    fn a_directory_or_a_socket_is_not_regular() {
        let root = TestRoot::new();
        make_dir(&root.join("dir.json"), 0o700);
        let _socket = UnixListener::bind(root.join("sock.json")).expect("bind");
        for name in ["dir.json", "sock.json"] {
            assert_eq!(
                check_of(open_in(&root, name)),
                (root.join(name), TrustCheck::NotRegular)
            );
        }
    }

    #[test]
    fn one_byte_over_a_mebibyte_is_size_and_exactly_a_mebibyte_opens() {
        let root = TestRoot::new();
        write_file(&root.join("exact.json"), &vec![b' '; LIMIT], 0o600);
        write_file(&root.join("over.json"), &vec![b' '; LIMIT + 1], 0o600);
        assert_eq!(
            open_in(&root, "exact.json").map(|file| file.bytes().len()),
            Ok(LIMIT)
        );
        assert_eq!(
            check_of(open_in(&root, "over.json")),
            (root.join("over.json"), TrustCheck::Size)
        );
    }

    #[test]
    fn etc_hosts_is_parent_because_root_owns_its_directory() {
        if effective_uid() == 0 {
            println!("skipped: root owns /etc, so it passes the parent check");
            return;
        }
        #[cfg(target_os = "macos")]
        let hosts = Path::new("/private/etc/hosts");
        #[cfg(target_os = "linux")]
        let hosts = Path::new("/etc/hosts");
        assert_eq!(
            check_of(TrustedFile::open(hosts)),
            (hosts.to_path_buf(), TrustCheck::Parent)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn etc_hosts_through_the_etc_symlink_is_path() {
        assert_eq!(
            check_of(TrustedFile::open(Path::new("/etc/hosts"))),
            (PathBuf::from("/etc/hosts"), TrustCheck::Path)
        );
    }

    #[test]
    fn a_missing_file_or_a_relative_path_is_path() {
        let root = TestRoot::new();
        assert_eq!(
            check_of(open_in(&root, "absent.json")),
            (root.join("absent.json"), TrustCheck::Path)
        );
        assert_eq!(
            check_of(TrustedFile::open(Path::new("operator.json"))),
            (PathBuf::from("operator.json"), TrustCheck::Path)
        );
    }

    #[test]
    fn a_file_the_controller_cannot_read_is_path() {
        if effective_uid() == 0 {
            println!("skipped: root reads every file");
            return;
        }
        let root = TestRoot::new();
        write_file(&root.join("operator.json"), b"{}", 0o200);
        let refusal = refused(open_in(&root, "operator.json"));
        assert_eq!(refusal.check, TrustCheck::Path);
        assert!(refusal.reason.contains("errno 13"), "{}", refusal.reason);
    }

    #[test]
    fn the_file_judge_checks_the_parent_then_the_type_then_the_owner() {
        let euid = 501;
        let path = Path::new("/r/operator.json");
        let judged = |file: Facts, parent: Facts| {
            judge_file(
                &Reached {
                    path,
                    facts: file,
                    parent: Some(parent),
                },
                euid,
            )
        };
        let own_dir = facts(Kind::Directory, euid, 0o700);
        assert!(matches!(
            judged(facts(Kind::Regular, euid, 0o600), own_dir),
            Ok(())
        ));
        assert!(matches!(
            judged(facts(Kind::Regular, 4242, 0o600), own_dir),
            Err(Fault::Check(TrustCheck::Owner, _))
        ));
        assert!(matches!(
            judged(facts(Kind::Regular, 0, 0o600), own_dir),
            Err(Fault::Check(TrustCheck::Owner, _))
        ));
        assert!(matches!(
            judged(facts(Kind::Other, 4242, 0o600), own_dir),
            Err(Fault::Check(TrustCheck::NotRegular, _))
        ));
        assert!(matches!(
            judged(
                facts(Kind::Other, 4242, 0o600),
                facts(Kind::Directory, 0, 0o755)
            ),
            Err(Fault::Check(TrustCheck::Parent, _))
        ));
    }

    #[test]
    fn checks_carry_the_names_spec_022_gives_them() {
        let cases = [
            (TrustCheck::Path, "path"),
            (TrustCheck::Ancestor, "ancestor"),
            (TrustCheck::Parent, "parent"),
            (TrustCheck::NotRegular, "not_regular"),
            (TrustCheck::Owner, "owner"),
            (TrustCheck::Links, "links"),
            (TrustCheck::Mode, "mode"),
            (TrustCheck::Size, "size"),
        ];
        for (check, name) in cases {
            assert_eq!(check.as_str(), name);
            assert_eq!(check.to_string(), name);
        }
    }

    #[test]
    fn a_refusal_names_the_file_the_check_and_the_reason() {
        let refusal = TrustRefusal {
            path: PathBuf::from("/r/operator.json"),
            check: TrustCheck::Links,
            reason: "/r/operator.json has 2 links; a trusted file has exactly one".to_string(),
        };
        assert_eq!(
            refusal.to_string(),
            "/r/operator.json fails the links check: /r/operator.json has 2 links; a trusted \
             file has exactly one"
        );
    }
}
