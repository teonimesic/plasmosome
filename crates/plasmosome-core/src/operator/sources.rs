#[cfg(test)]
mod tests {
    use super::GuardedKind::*;
    use super::Relation::*;
    use super::*;
    use crate::nofollow::fixtures::{TestRoot, identity_of, make_dir, set_mode, write_file};
    use crate::nofollow::{Walk, WalkRefusal, effective_uid};
    use crate::operator::trusted::{TrustCheck, TrustRefusal, TrustedFile};
    use plasmosome_backend::CellId;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};

    const KINDS: [GuardedKind; 8] = [
        InstanceRoot,
        RegistryRoot,
        ControlSocket,
        Configuration,
        Executable,
        OperatorFile,
        Declaration,
        CellRuntimeArtifact,
    ];

    const BIN: &str = "opt/homebrew/Cellar/plasmosome/0.1.0/bin";

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

    fn guard(root: &TestRoot, guarded: &[(GuardedKind, &str)]) -> Result<Guard, GuardRefusal> {
        let guarded: Vec<_> = guarded
            .iter()
            .map(|(kind, relative)| (*kind, root.join(relative)))
            .collect();
        Guard::new_with(&guarded, &[], walker(root), Ok(None))
    }

    fn source(guard: &Guard, root: &TestRoot, relative: &str) -> Result<(), GuardRefusal> {
        guard.check_with(&root.join(relative), walker(root))
    }

    fn overlap(
        root: &TestRoot,
        source: &str,
        relation: Relation,
        what: GuardedKind,
        guarded: &str,
    ) -> Result<(), GuardRefusal> {
        Err(GuardRefusal::Overlap {
            source: root.join(source),
            relation,
            what,
            guarded: root.join(guarded),
        })
    }

    fn homebrew(root: &TestRoot) -> Guard {
        for (relative, mode) in [
            ("opt", 0o755),
            ("opt/homebrew", 0o755),
            ("opt/homebrew/Cellar", 0o775),
            ("opt/homebrew/Cellar/plasmosome", 0o755),
            ("opt/homebrew/Cellar/plasmosome/0.1.0", 0o755),
            (BIN, 0o755),
            ("opt/homebrew/Cellar/other", 0o755),
            ("src", 0o700),
            ("wide", 0o777),
        ] {
            make_dir(&root.join(relative), mode);
        }
        guard(root, &[(Executable, BIN)]).expect("a guarded directory below a 0775 one opens")
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

    #[test]
    fn a_source_equal_to_inside_or_above_each_guarded_kind_is_refused() {
        let root = TestRoot::new();
        let mut guarded = Vec::new();
        for (index, kind) in KINDS.into_iter().enumerate() {
            let parent = format!("k{index}");
            for relative in ["", "/g", "/g/in", "/g-x"] {
                make_dir(&root.join(&format!("{parent}{relative}")), 0o700);
            }
            guarded.push((kind, root.join(&format!("{parent}/g"))));
        }
        let guard =
            Guard::new_with(&guarded, &[], walker(&root), Ok(None)).expect("every one opens");
        for (index, kind) in KINDS.into_iter().enumerate() {
            let parent = format!("k{index}");
            let dir = format!("{parent}/g");
            for (relative, relation) in [
                (dir.clone(), Same),
                (format!("{dir}/in"), Inside),
                (parent.clone(), Above),
            ] {
                assert_eq!(
                    source(&guard, &root, &relative),
                    overlap(&root, &relative, relation, kind, &dir),
                    "{relative}"
                );
            }
            assert_eq!(source(&guard, &root, &format!("{parent}/g-x")), Ok(()));
        }
    }

    #[test]
    fn a_guarded_directory_under_a_group_writable_user_owned_directory_stays_guarded() {
        let root = TestRoot::new();
        let guard = homebrew(&root);
        for relative in ["opt/homebrew/Cellar", "opt/homebrew"] {
            assert_eq!(
                source(&guard, &root, relative),
                overlap(&root, relative, Above, Executable, BIN)
            );
        }
        assert_eq!(source(&guard, &root, "src"), Ok(()));
    }

    #[test]
    fn a_source_below_a_directory_failing_rule_two_is_refused_and_the_source_itself_is_not_judged()
    {
        let root = TestRoot::new();
        let guard = homebrew(&root);
        assert_eq!(
            source(&guard, &root, "opt/homebrew/Cellar/other"),
            Err(GuardRefusal::Source {
                source: root.join("opt/homebrew/Cellar/other"),
                refusal: WalkRefusal::Replaceable {
                    path: root.join("opt/homebrew/Cellar"),
                    mode: 0o775
                }
            })
        );
        assert_eq!(source(&guard, &root, "wide"), Ok(()));
    }

    #[test]
    fn a_symlinked_regular_file_or_missing_source_is_refused() {
        let root = TestRoot::new();
        dirs(&root, &["inst", "src", "real", "real/x"]);
        symlink("src", root.join("link")).expect("symlink");
        symlink("real", root.join("alias")).expect("symlink");
        write_file(&root.join("file"), b"", 0o600);
        let guard = guard(&root, &[(InstanceRoot, "inst")]).expect("opens");
        let at = |relative: &str| root.join(relative);
        for (relative, refusal) in [
            ("link", WalkRefusal::Symlink { path: at("link") }),
            ("alias/x", WalkRefusal::Symlink { path: at("alias") }),
            ("file", WalkRefusal::NotDirectory { path: at("file") }),
            ("absent", WalkRefusal::Missing { path: at("absent") }),
        ] {
            assert_eq!(
                source(&guard, &root, relative),
                Err(GuardRefusal::Source {
                    source: root.join(relative),
                    refusal
                }),
                "{relative}"
            );
        }
        assert_eq!(source(&guard, &root, "src"), Ok(()));
    }

    #[test]
    fn a_guarded_directory_the_walk_cannot_open_refuses_every_mount() {
        let root = TestRoot::new();
        dirs(&root, &["inst", "real", "real/dir"]);
        symlink("real", root.join("link")).expect("symlink");
        assert_eq!(
            guard(&root, &[(InstanceRoot, "inst"), (Configuration, "missing")]).map(|_| ()),
            Err(GuardRefusal::Guarded {
                what: Configuration,
                path: root.join("missing"),
                refusal: WalkRefusal::Missing {
                    path: root.join("missing")
                }
            })
        );
        assert_eq!(
            guard(&root, &[(Declaration, "link/dir")]).map(|_| ()),
            Err(GuardRefusal::Guarded {
                what: Declaration,
                path: root.join("link/dir"),
                refusal: WalkRefusal::Symlink {
                    path: root.join("link")
                }
            })
        );
        if effective_uid() == 0 {
            println!("skipped the unsearchable case: root searches every directory");
            return;
        }
        dirs(&root, &["locked", "locked/inst"]);
        set_mode(&root.join("locked"), 0o000);
        let outcome = guard(&root, &[(InstanceRoot, "locked/inst")]).map(|_| ());
        set_mode(&root.join("locked"), 0o700);
        assert!(
            matches!(
                &outcome,
                Err(GuardRefusal::Guarded {
                    what: InstanceRoot,
                    path,
                    refusal: WalkRefusal::Io { errno: libc::EACCES, .. }
                }) if *path == root.join("locked/inst")
            ),
            "{outcome:?}"
        );
    }

    #[test]
    fn a_trusted_files_directory_is_guarded_from_its_own_chain() {
        let root = TestRoot::new();
        dirs(&root, &["top", "top/ops", "top/ops/sub", "top/ops2"]);
        let file = trusted(&root, "top/ops/operator.json");
        let guard = Guard::new_with(&[], &[(OperatorFile, &file)], walker(&root), Ok(None))
            .expect("a trusted file's directory is guarded");
        for (relative, relation) in [("top/ops", Same), ("top/ops/sub", Inside), ("top", Above)] {
            assert_eq!(
                source(&guard, &root, relative),
                overlap(&root, relative, relation, OperatorFile, "top/ops")
            );
        }
        assert_eq!(source(&guard, &root, "top/ops2"), Ok(()));
        fs::rename(root.join("top/ops"), root.join("top/moved")).expect("rename");
        let guard = Guard::new_with(&[], &[(OperatorFile, &file)], walker(&root), Ok(None))
            .expect("the file's own chain needs no second walk");
        assert_eq!(
            source(&guard, &root, "top/moved"),
            overlap(&root, "top/moved", Same, OperatorFile, "top/ops")
        );
    }

    #[test]
    fn a_source_spelled_in_another_case_is_the_same_directory() {
        let root = TestRoot::new();
        dirs(&root, &["Inst"]);
        if fs::metadata(root.join("INST")).is_err() {
            println!("skipped: this volume is case-sensitive, so INST is another name");
            return;
        }
        let guard = guard(&root, &[(InstanceRoot, "Inst")]).expect("opens");
        assert_eq!(
            source(&guard, &root, "INST"),
            overlap(&root, "INST", Same, InstanceRoot, "Inst")
        );
    }

    #[test]
    fn a_source_or_guarded_directory_through_the_data_volume_is_refused() {
        let root = TestRoot::new();
        dirs(
            &root,
            &[
                "data",
                "data/x",
                "data/x/inst",
                "data/y",
                "data/ops",
                "inst",
                "z",
            ],
        );
        let file = trusted(&root, "data/ops/operator.json");
        let data = root.join("data");
        let built = |dirs: &[(GuardedKind, PathBuf)], files: &[(GuardedKind, &TrustedFile)]| {
            Guard::new_with(dirs, files, walker(&root), volume(&root, "data")).map(|_| ())
        };
        assert_eq!(
            built(&[(InstanceRoot, root.join("data/x/inst"))], &[]),
            Err(GuardRefusal::ThroughDataVolume {
                what: Some(InstanceRoot),
                path: root.join("data/x/inst"),
                data_volume: data.clone()
            })
        );
        assert_eq!(
            built(&[], &[(OperatorFile, &file)]),
            Err(GuardRefusal::ThroughDataVolume {
                what: Some(OperatorFile),
                path: root.join("data/ops"),
                data_volume: data.clone()
            })
        );
        let inst = [(InstanceRoot, root.join("inst"))];
        let guard =
            Guard::new_with(&inst, &[], walker(&root), volume(&root, "data")).expect("opens");
        for relative in ["data", "data/y"] {
            assert_eq!(
                source(&guard, &root, relative),
                Err(GuardRefusal::ThroughDataVolume {
                    what: None,
                    path: root.join(relative),
                    data_volume: data.clone()
                })
            );
        }
        assert_eq!(source(&guard, &root, "z"), Ok(()));
        assert_eq!(
            Guard::new_with(&inst, &[], walker(&root), unopenable_volume()).map(|_| ()),
            Err(GuardRefusal::DataVolume {
                refusal: unopenable_volume().expect_err("an error")
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_volumes_data_is_refused_over_a_firmlinked_guarded_directory() {
        let root = TestRoot::walkable_from_root();
        make_dir(&root.join("inst"), 0o700);
        make_dir(&root.join("src"), 0o700);
        let volume = PathBuf::from("/System/Volumes/Data");
        let guard = Guard::new(&[(InstanceRoot, root.join("inst"))], &[]).expect("opens");
        for path in [volume.clone(), data_side(&root.join("src"))] {
            assert_eq!(
                guard.check(&path),
                Err(GuardRefusal::ThroughDataVolume {
                    what: None,
                    path: path.clone(),
                    data_volume: volume.clone()
                })
            );
        }
        assert_eq!(guard.check(&root.join("src")), Ok(()));
        let inst = data_side(&root.join("inst"));
        assert_eq!(
            Guard::new(&[(InstanceRoot, inst.clone())], &[]).map(|_| ()),
            Err(GuardRefusal::ThroughDataVolume {
                what: Some(InstanceRoot),
                path: inst,
                data_volume: volume
            })
        );
    }

    #[test]
    fn a_guard_refusal_names_the_source_the_relation_the_kind_and_the_path() {
        let names = [
            "the instance root",
            "the registry root",
            "the control socket's directory",
            "the configuration file's directory",
            "the controller executable's directory",
            "the operator file's directory",
            "a listed declaration's directory",
            "a cell_runtime Artifact's directory",
        ];
        for (kind, name) in KINDS.into_iter().zip(names) {
            assert_eq!(kind.to_string(), name);
        }
        for (relation, name) in [
            (Same, "the same directory as"),
            (Inside, "inside"),
            (Above, "above"),
        ] {
            assert_eq!(relation.to_string(), name);
        }
        let at = PathBuf::from;
        let cases = [
            (
                GuardRefusal::Overlap {
                    source: at("/opt/homebrew/Cellar"),
                    relation: Above,
                    what: Executable,
                    guarded: at("/opt/homebrew/Cellar/plasmosome/0.1.0/bin"),
                },
                "mount source /opt/homebrew/Cellar is above the controller executable's directory \
                 /opt/homebrew/Cellar/plasmosome/0.1.0/bin",
            ),
            (
                GuardRefusal::Overlap {
                    source: at("/srv/inst"),
                    relation: Same,
                    what: InstanceRoot,
                    guarded: at("/srv/inst"),
                },
                "mount source /srv/inst is the same directory as the instance root /srv/inst",
            ),
            (
                GuardRefusal::Guarded {
                    what: Configuration,
                    path: at("/srv/conf"),
                    refusal: WalkRefusal::Missing {
                        path: at("/srv/conf"),
                    },
                },
                "the configuration file's directory /srv/conf does not open, so no mount source \
                 can be checked against it: /srv/conf does not exist",
            ),
            (
                GuardRefusal::Source {
                    source: at("/srv/link"),
                    refusal: WalkRefusal::Symlink {
                        path: at("/srv/link"),
                    },
                },
                "mount source /srv/link is refused: /srv/link is a symlink, which the walk does \
                 not follow",
            ),
            (
                GuardRefusal::ThroughDataVolume {
                    what: None,
                    path: at("/System/Volumes/Data/usr"),
                    data_volume: at("/System/Volumes/Data"),
                },
                "mount source /System/Volumes/Data/usr is reached through the data volume \
                 /System/Volumes/Data, where a firmlink gives a directory a second parent that \
                 no walk from / sees; name it by its path outside /System/Volumes/Data",
            ),
            (
                GuardRefusal::ThroughDataVolume {
                    what: Some(InstanceRoot),
                    path: at("/System/Volumes/Data/Users/op/inst"),
                    data_volume: at("/System/Volumes/Data"),
                },
                "the instance root /System/Volumes/Data/Users/op/inst is reached through the data \
                 volume /System/Volumes/Data, where a firmlink gives a directory a second parent \
                 that no walk from / sees; name it by its path outside /System/Volumes/Data",
            ),
            (
                GuardRefusal::DataVolume {
                    refusal: unopenable_volume().expect_err("an error"),
                },
                "the data volume does not open, so a firmlink cannot be ruled out: openat failed \
                 at /System/Volumes/Data (errno 5)",
            ),
        ];
        for (refusal, text) in cases {
            assert_eq!(refusal.to_string(), text);
        }
    }
}
