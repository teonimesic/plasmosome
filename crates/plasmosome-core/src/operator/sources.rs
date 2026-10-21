#[cfg(test)]
mod tests {
    use super::*;
    use crate::nofollow::fixtures::{TestRoot, identity_of, make_dir, write_file};
    use crate::nofollow::{Walk, WalkRefusal};
    use crate::operator::trusted::{TrustCheck, TrustRefusal, TrustedFile};
    use plasmosome_backend::CellId;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};

    fn walker(root: &TestRoot) -> impl Fn(&Path) -> Result<Walk, WalkRefusal> + '_ {
        |path| {
            let relative = path
                .strip_prefix(root.path())
                .expect("a path under the test root");
            Walk::from_dir(root.start(), root.path(), relative)
        }
    }

    fn dirs(root: &TestRoot, relatives: &[&str]) {
        for relative in relatives {
            make_dir(&root.join(relative), 0o700);
        }
    }

    fn trusted(root: &TestRoot, relative: &str) -> TrustedFile {
        write_file(&root.join(relative), b"{}", 0o600);
        TrustedFile::from_walk(root.walk(relative)).expect("a trusted file opens")
    }

    fn held(cell: &str, root: &TestRoot, relative: &str) -> Source {
        Source {
            holder: Holder::Cell(CellId::from(cell)),
            path: root.join(relative),
        }
    }

    fn declared(root: &TestRoot, relative: &str) -> Source {
        Source {
            holder: Holder::Declared,
            path: root.join(relative),
        }
    }

    fn volume(root: &TestRoot, relative: &str) -> Result<Option<DataVolume>, WalkRefusal> {
        Ok(Some(DataVolume {
            path: root.join(relative),
            identity: identity_of(&root.join(relative)),
        }))
    }

    fn unopenable_volume() -> Result<Option<DataVolume>, WalkRefusal> {
        Err(WalkRefusal::Io {
            op: "openat",
            path: PathBuf::from("/System/Volumes/Data"),
            errno: libc::EIO,
        })
    }

    fn probe(root: &TestRoot, sources: &[Source]) -> Probes {
        Probes::probe_with(sources, walker(root), Ok(None))
    }

    fn checked(
        probes: &Probes,
        file: &TrustedFile,
        cell: Option<&str>,
    ) -> Result<(), TrustRefusal> {
        probes.check(file, cell.map(CellId::from).as_ref())
    }

    fn refused(probes: &Probes, file: &TrustedFile, cell: Option<&str>) -> TrustRefusal {
        let refusal = checked(probes, file, cell).expect_err("refused");
        assert_eq!(
            (refusal.path.as_path(), refusal.check),
            (file.path(), TrustCheck::InsideMountSource)
        );
        refusal
    }

    #[cfg(target_os = "macos")]
    fn data_side(path: &Path) -> PathBuf {
        Path::new("/System/Volumes/Data").join(path.strip_prefix("/").expect("absolute"))
    }

    #[test]
    fn a_file_in_or_below_a_held_source_is_inside_mount_source() {
        let root = TestRoot::new();
        dirs(&root, &["S", "S/sub", "T", "S-x"]);
        let probes = probe(&root, &[held("cell-1", &root, "S")]);
        for relative in ["S/op.json", "S/sub/decl.toml"] {
            let file = trusted(&root, relative);
            for cell in [None, Some("cell-1"), Some("cell-2")] {
                let refusal = refused(&probes, &file, cell);
                assert_eq!(
                    refusal.reason,
                    format!(
                        "{} lies inside mount source {}, which cell-1 holds",
                        root.join(relative).display(),
                        root.join("S").display()
                    )
                );
            }
        }
        for relative in ["T/op.json", "S-x/op.json"] {
            let file = trusted(&root, relative);
            assert_eq!(
                checked(&probes, &file, Some("cell-1")),
                Ok(()),
                "{relative}"
            );
        }
    }

    #[test]
    fn a_declared_source_counts_before_any_cell_holds_it() {
        let root = TestRoot::new();
        dirs(&root, &["S", "T"]);
        let probes = probe(&root, &[declared(&root, "S")]);
        let file = trusted(&root, "S/op.json");
        for cell in [None, Some("cell-1")] {
            let refusal = refused(&probes, &file, cell);
            assert!(
                refusal.reason.ends_with("which the operator file declares"),
                "{}",
                refusal.reason
            );
        }
        assert_eq!(checked(&probes, &trusted(&root, "T/op.json"), None), Ok(()));
    }

    #[test]
    fn a_held_source_renamed_away_refuses_only_its_cells_requests() {
        let root = TestRoot::new();
        dirs(&root, &["S", "T"]);
        let file = trusted(&root, "T/op.json");
        fs::rename(root.join("S"), root.join("S.old")).expect("rename");
        let probes = probe(&root, &[held("cell-1", &root, "S")]);
        let refusal = refused(&probes, &file, Some("cell-1"));
        let source = root.join("S").display().to_string();
        assert_eq!(
            refusal.reason,
            format!(
                "mount source {source}, which cell-1 holds, does not open: {source} does not exist"
            )
        );
        assert_eq!(checked(&probes, &file, Some("cell-2")), Ok(()));
        assert_eq!(checked(&probes, &file, None), Ok(()));
    }

    #[test]
    fn an_unopenable_declared_source_is_skipped() {
        let root = TestRoot::new();
        dirs(&root, &["S2"]);
        symlink("S2", root.join("link")).expect("symlink");
        let probes = probe(
            &root,
            &[declared(&root, "missing"), declared(&root, "link")],
        );
        let file = trusted(&root, "S2/op.json");
        for cell in [None, Some("cell-1")] {
            assert_eq!(checked(&probes, &file, cell), Ok(()), "{cell:?}");
        }
    }

    #[test]
    fn a_held_source_now_a_symlink_refuses_only_its_cell() {
        let root = TestRoot::new();
        dirs(&root, &["S"]);
        symlink("S", root.join("link")).expect("symlink");
        let probes = probe(&root, &[held("cell-1", &root, "link")]);
        let file = trusted(&root, "S/op.json");
        let refusal = refused(&probes, &file, Some("cell-1"));
        assert!(
            refusal.reason.ends_with(&format!(
                "{} is a symlink, which the walk does not follow",
                root.join("link").display()
            )),
            "{}",
            refusal.reason
        );
        assert_eq!(
            checked(&probes, &file, Some("cell-2")),
            Ok(()),
            "a probe that follows the link would refuse cell-2"
        );
    }

    #[test]
    fn a_file_through_the_data_volume_is_inside_mount_source() {
        let root = TestRoot::new();
        dirs(&root, &["data", "data/x", "y"]);
        let through = trusted(&root, "data/x/op.json");
        let outside = trusted(&root, "y/op.json");
        let probes = Probes::probe_with(&[], walker(&root), volume(&root, "data"));
        let refusal = refused(&probes, &through, None);
        assert!(
            refusal.reason.starts_with(&format!(
                "{} is reached through the data volume {}",
                root.join("data/x/op.json").display(),
                root.join("data").display()
            )),
            "{}",
            refusal.reason
        );
        assert_eq!(checked(&probes, &outside, None), Ok(()));
        let blind = Probes::probe_with(&[], walker(&root), unopenable_volume());
        let refusal = refused(&blind, &outside, None);
        assert_eq!(
            refusal.reason,
            "the data volume does not open, so a firmlink cannot be ruled out: openat failed at \
             /System/Volumes/Data (errno 5)"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_trusted_file_spelled_through_system_volumes_data_is_refused() {
        let root = TestRoot::walkable_from_root();
        write_file(&root.join("op.json"), b"{}", 0o600);
        let firmlinked = TrustedFile::open(&root.join("op.json")).expect("the file opens");
        let spelled = data_side(&root.join("op.json"));
        let through =
            TrustedFile::open(&spelled).expect("the data-side spelling meets rules 1 to 4");
        let probes = Probes::probe(&[]);
        let refusal = refused(&probes, &through, None);
        assert_eq!(refusal.path, spelled);
        assert_eq!(probes.check(&firmlinked, None), Ok(()));
    }

    #[test]
    fn a_missing_data_volume_is_none_and_one_that_does_not_open_is_an_error() {
        let root = TestRoot::new();
        dirs(&root, &["Data"]);
        symlink("Data", root.join("link")).expect("symlink");
        assert_eq!(
            volume_at(&root.join("Data"), walker(&root)),
            volume(&root, "Data")
        );
        assert_eq!(volume_at(&root.join("absent"), walker(&root)), Ok(None));
        assert_eq!(
            volume_at(&root.join("link"), walker(&root)),
            Err(WalkRefusal::Symlink {
                path: root.join("link")
            })
        );
        #[cfg(target_os = "macos")]
        let host = Some(DataVolume {
            path: PathBuf::from("/System/Volumes/Data"),
            identity: identity_of(Path::new("/System/Volumes/Data")),
        });
        #[cfg(target_os = "linux")]
        let host = None;
        assert_eq!(data_volume(), Ok(host));
    }
}
