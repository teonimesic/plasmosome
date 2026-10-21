use std::fmt;
use std::path::{Path, PathBuf};

use plasmosome_backend::CellId;

use crate::nofollow::{DirIdentity, Walk, WalkRefusal};
use crate::operator::trusted::{TrustCheck, TrustRefusal, TrustedFile};

/// Who has a Mount source: a cell of this instance that holds it or has a pending operation for
/// it, or the operator file, which declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Holder {
    Cell(CellId),
    Declared,
}

impl fmt::Display for Holder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Holder::Cell(cell) => write!(f, "{cell} holds"),
            Holder::Declared => f.write_str("the operator file declares"),
        }
    }
}

/// A Mount source spec 022 §3 rule 5 keeps trusted files out of: the path recorded at attach or
/// declared, and who has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Source {
    pub(crate) holder: Holder,
    pub(crate) path: PathBuf,
}

/// The macOS data volume and its identity. A firmlink such as `/Users` opens one of its
/// directories, so that directory has a second parent, the data volume, that no walk from "/"
/// through the firmlink passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataVolume {
    pub(crate) path: PathBuf,
    pub(crate) identity: DirIdentity,
}

#[cfg(target_os = "macos")]
const DATA_VOLUME: Option<&str> = Some("/System/Volumes/Data");

#[cfg(target_os = "linux")]
const DATA_VOLUME: Option<&str> = None;

const FIRMLINK: &str =
    "where a firmlink gives a directory a second parent that no walk from / sees";

/// Opens `/System/Volumes/Data` with the walk of spec 022 §3 rule 1. It is `Ok(None)` on Linux,
/// which has no firmlinks, and when that path is missing. Any other refusal is `Err`, which the
/// caller must refuse on, never read as no data volume.
pub(crate) fn data_volume() -> Result<Option<DataVolume>, WalkRefusal> {
    match DATA_VOLUME {
        Some(path) => volume_at(Path::new(path), Walk::from_root),
        None => Ok(None),
    }
}

fn volume_at(
    path: &Path,
    walk: impl Fn(&Path) -> Result<Walk, WalkRefusal>,
) -> Result<Option<DataVolume>, WalkRefusal> {
    match walk(path).and_then(|walk| walk.identities()) {
        Ok(chain) => Ok(chain.last().map(|&identity| DataVolume {
            path: path.to_path_buf(),
            identity,
        })),
        Err(WalkRefusal::Missing { .. }) => Ok(None),
        Err(refusal) => Err(refusal),
    }
}

fn unopened_volume(refusal: &WalkRefusal) -> String {
    format!("the data volume does not open, so a firmlink cannot be ruled out: {refusal}")
}

/// Every held, pending and declared Mount source, each opened once with the walk of spec 022 §3
/// rule 1 alone, and the data volume. Build it once per resolving request, after the operator file
/// is read, and check every trusted file the request reads against it.
#[derive(Debug)]
pub(crate) struct Probes {
    probed: Vec<(Source, Result<Vec<DirIdentity>, WalkRefusal>)>,
    data_volume: Result<Option<DataVolume>, WalkRefusal>,
}

impl Probes {
    /// Opens each of `sources` from "/" without following a symlink or judging any directory, and
    /// keeps its identity or why it did not open. It reads the data volume once. It never fails:
    /// [`Probes::check`] decides what a source that did not open means.
    pub(crate) fn probe(sources: &[Source]) -> Probes {
        Self::probe_with(sources, Walk::from_root, data_volume())
    }

    fn probe_with(
        sources: &[Source],
        walk: impl Fn(&Path) -> Result<Walk, WalkRefusal>,
        data_volume: Result<Option<DataVolume>, WalkRefusal>,
    ) -> Probes {
        let probed = sources
            .iter()
            .map(|source| {
                let found = walk(&source.path).and_then(|walk| walk.identities());
                (source.clone(), found)
            })
            .collect();
        Probes {
            probed,
            data_volume,
        }
    }

    /// Refuses `file` with `inside_mount_source` when its directory is, or lies below, a source
    /// that opened, whoever has it; when a source the cell `resolving_for` holds did not open; and
    /// when the file is reached through the data volume or the data volume did not open. Pass the
    /// request's cell, or `None` for a new cell. A declared source that did not open never
    /// refuses.
    pub(crate) fn check(
        &self,
        file: &TrustedFile,
        resolving_for: Option<&CellId>,
    ) -> Result<(), TrustRefusal> {
        let shown = file.path().display();
        let refuse = |reason: String| {
            Err(TrustRefusal {
                path: file.path().to_path_buf(),
                check: TrustCheck::InsideMountSource,
                reason,
            })
        };
        match &self.data_volume {
            Err(refusal) => return refuse(unopened_volume(refusal)),
            Ok(Some(volume)) if file.chain().contains(&volume.identity) => {
                let volume = volume.path.display();
                return refuse(format!(
                    "{shown} is reached through the data volume {volume}, {FIRMLINK}"
                ));
            }
            Ok(_) => {}
        }
        for (source, found) in &self.probed {
            let (path, holder) = (source.path.display(), &source.holder);
            match found {
                Ok(chain) if chain.last().is_some_and(|dir| file.chain().contains(dir)) => {
                    return refuse(format!(
                        "{shown} lies inside mount source {path}, which {holder}"
                    ));
                }
                Err(refusal) if matches!(holder, Holder::Cell(cell) if Some(cell) == resolving_for) =>
                {
                    return refuse(format!(
                        "mount source {path}, which {holder}, does not open: {refusal}"
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

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
