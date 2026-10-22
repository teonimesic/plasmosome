use std::fmt;
use std::path::{Path, PathBuf};

use plasmosome_backend::CellId;

use crate::nofollow::{DirIdentity, Kind, Reached, Walk, WalkRefusal};
use crate::operator::trusted::{TrustCheck, TrustRefusal, TrustedFile};

/// Who has a Mount source: a cell of this instance that holds it or has a pending operation for
/// it, or the operator file, which declares it. Label every source a cell holds or has pending
/// `Cell`, even when the operator file also declares it. A `Declared` source that does not open
/// never refuses, so a held source labelled `Declared` stops refusing once it is renamed away, and
/// nothing signals it.
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

/// The macOS data volume, its identity, and the identity of "/" and of every directory down to it,
/// the volume's own last. A firmlink such as `/Users` opens one of its directories, so that
/// directory has a second parent, the data volume, that no walk from "/" through the firmlink
/// passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataVolume {
    pub(crate) path: PathBuf,
    pub(crate) identity: DirIdentity,
    pub(crate) chain: Vec<DirIdentity>,
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
        Ok(chain) => Ok(chain.last().copied().map(|identity| DataVolume {
            path: path.to_path_buf(),
            identity,
            chain,
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

    /// Refuses `file` with `inside_mount_source` when a source that opened, whoever has it, is the
    /// file's directory or a directory above it. On macOS a firmlink gives a directory a second
    /// parent, so both chains count: the one from "/" the file was opened through, and the one
    /// through the data volume, found by opening `/System/Volumes/Data` followed by the
    /// directory's path, one component more at a time, until a spelling opens a directory on the
    /// first chain. So `/System`, `/System/Volumes`, `/System/Volumes/Data` and
    /// `/System/Volumes/Data/usr` each hold a file under `/usr/local`, and a rename below the
    /// firmlink after the file was opened does not change that. It also refuses when a source the
    /// cell `resolving_for` holds did not open; when the file is reached through the data volume;
    /// and when the data volume, or a spelling of the directory's path through it, does not open
    /// for any reason but a missing name. A declared source that did not open never refuses.
    ///
    /// Pass the cell the request resolves for, or `None` only for a request that makes a new
    /// cell. `None` for an existing cell skips the sources that cell holds and that no longer
    /// open, and nothing signals it.
    pub(crate) fn check(
        &self,
        file: &TrustedFile,
        resolving_for: Option<&CellId>,
    ) -> Result<(), TrustRefusal> {
        self.check_at(file.path(), file.chain(), resolving_for)
            .map_err(|reason| TrustRefusal {
                path: file.path().to_path_buf(),
                check: TrustCheck::InsideMountSource,
                reason,
            })
    }

    fn check_at(
        &self,
        path: &Path,
        chain: &[DirIdentity],
        resolving_for: Option<&CellId>,
    ) -> Result<(), String> {
        let shown = path.display();
        let second = match &self.data_volume {
            Err(refusal) => return Err(unopened_volume(refusal)),
            Ok(Some(volume)) if chain.contains(&volume.identity) => {
                let volume = volume.path.display();
                return Err(format!(
                    "{shown} is reached through the data volume {volume}, {FIRMLINK}"
                ));
            }
            Ok(Some(volume)) => data_side(volume, path, chain)?,
            Ok(None) => Vec::new(),
        };
        for (source, found) in &self.probed {
            let (spelled, holder) = (source.path.display(), &source.holder);
            match found {
                Ok(dirs)
                    if dirs
                        .last()
                        .is_some_and(|dir| chain.contains(dir) || second.contains(dir)) =>
                {
                    return Err(format!(
                        "{shown} lies inside mount source {spelled}, which {holder}"
                    ));
                }
                Err(refusal) if matches!(holder, Holder::Cell(cell) if Some(cell) == resolving_for) =>
                {
                    return Err(format!(
                        "mount source {spelled}, which {holder}, does not open: {refusal}"
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// The second chain a firmlink gives the directory holding `path`. It opens the data volume
/// followed by the first component of the directory's path, then the first two, and so on, and
/// stops at the first spelling that opens a directory on `chain`: the firmlink's target. It
/// returns every identity that walk passes. The directories below the target are on `chain`
/// already and are never opened by path again, so a rename there after the file was opened cannot
/// hide the second chain. It is empty when a spelling is missing, or when none opens a directory
/// on `chain`, so the directory has no second parent. Any other refusal is `Err`, because a
/// firmlink cannot then be ruled out.
fn data_side(
    volume: &DataVolume,
    path: &Path,
    chain: &[DirIdentity],
) -> Result<Vec<DirIdentity>, String> {
    let dir = path.parent().unwrap_or(path);
    let mut spelled = volume.path.clone();
    for name in dir.iter().skip(1) {
        spelled.push(name);
        match Walk::from_root(&spelled).and_then(|walk| walk.identities()) {
            Ok(found) if found.last().is_some_and(|last| chain.contains(last)) => return Ok(found),
            Ok(_) => {}
            Err(WalkRefusal::Missing { .. }) => return Ok(Vec::new()),
            Err(refusal) => {
                return Err(format!(
                    "the data-side spelling {} of {} does not open, so a firmlink cannot be \
                     ruled out: {refusal}",
                    spelled.display(),
                    dir.display()
                ));
            }
        }
    }
    Ok(Vec::new())
}

/// What a guarded directory of spec 022 §6 is, as a refusal names it. A new guarded directory is
/// one more kind here and one more entry in the list the caller passes to [`Guard::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuardedKind {
    InstanceRoot,
    RegistryRoot,
    ControlSocket,
    Configuration,
    Executable,
    OperatorFile,
    Declaration,
    CellRuntimeArtifact,
}

impl fmt::Display for GuardedKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GuardedKind::InstanceRoot => "the instance root",
            GuardedKind::RegistryRoot => "the registry root",
            GuardedKind::ControlSocket => "the control socket's directory",
            GuardedKind::Configuration => "the configuration file's directory",
            GuardedKind::Executable => "the controller executable's directory",
            GuardedKind::OperatorFile => "the operator file's directory",
            GuardedKind::Declaration => "a listed declaration's directory",
            GuardedKind::CellRuntimeArtifact => "a cell_runtime Artifact's directory",
        })
    }
}

/// Where a refused Mount source lies against a guarded directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Relation {
    Same,
    Inside,
    Above,
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Relation::Same => "the same directory as",
            Relation::Inside => "inside",
            Relation::Above => "above",
        })
    }
}

/// Where the directory at the end of `source` lies against the one at the end of `guarded`, both
/// chains from the same start. Only identities are compared.
fn relation(source: &[DirIdentity], guarded: &[DirIdentity]) -> Option<Relation> {
    match (source.last(), guarded.last()) {
        (Some(reached), Some(dir)) if reached == dir => Some(Relation::Same),
        (Some(reached), _) if guarded.contains(reached) => Some(Relation::Above),
        (_, Some(dir)) if source.contains(dir) => Some(Relation::Inside),
        _ => None,
    }
}

/// Every guarded directory of spec 022 §6, with the identity of "/" and of every directory down to
/// it, and the data volume. Build it once per request whose closure yields a Mount, and check each
/// Mount source against it.
#[derive(Debug)]
pub(crate) struct Guard {
    guarded: Vec<(GuardedKind, PathBuf, Vec<DirIdentity>)>,
    data_volume: Option<DataVolume>,
}

impl Guard {
    /// Opens each of `dirs` from "/" with the walk of spec 022 §3 rule 1 alone: no symlink is
    /// followed and no directory is judged, so a directory below a group-writable one stays
    /// guarded. Each of `files` guards the directory it was read from, by its own chain, with no
    /// second walk. A guarded directory that does not open, one reached through the data volume,
    /// and a data volume that does not open each refuse, and then no Mount can be accepted. Pass
    /// real paths, since a symlink in one refuses.
    pub(crate) fn new(
        dirs: &[(GuardedKind, PathBuf)],
        files: &[(GuardedKind, &TrustedFile)],
    ) -> Result<Guard, GuardRefusal> {
        Self::new_with(dirs, files, Walk::from_root, data_volume())
    }

    fn new_with(
        dirs: &[(GuardedKind, PathBuf)],
        files: &[(GuardedKind, &TrustedFile)],
        walk: impl Fn(&Path) -> Result<Walk, WalkRefusal>,
        data_volume: Result<Option<DataVolume>, WalkRefusal>,
    ) -> Result<Guard, GuardRefusal> {
        let data_volume = data_volume.map_err(|refusal| GuardRefusal::DataVolume { refusal })?;
        let mut guarded = Vec::with_capacity(dirs.len() + files.len());
        for (what, path) in dirs {
            let chain = walk(path)
                .and_then(|walk| walk.identities())
                .map_err(|refusal| GuardRefusal::Guarded {
                    what: *what,
                    path: path.clone(),
                    refusal,
                })?;
            guarded.push((*what, path.clone(), chain));
        }
        for (what, file) in files {
            let dir = file.path().parent().unwrap_or(file.path());
            guarded.push((*what, dir.to_path_buf(), file.chain().to_vec()));
        }
        if let Some(volume) = &data_volume
            && let Some((what, path, _)) = guarded
                .iter()
                .find(|(_, _, chain)| chain.contains(&volume.identity))
        {
            return Err(GuardRefusal::ThroughDataVolume {
                what: Some(*what),
                path: path.clone(),
                data_volume: volume.path.clone(),
            });
        }
        Ok(Guard {
            guarded,
            data_volume,
        })
    }

    /// Opens `source` from "/" without following a symlink, holds every directory above it to
    /// spec 022 §3 rule 2 and requires a directory. It then refuses a source that is the data
    /// volume, lies inside it or lies above it, such as `/System`, since such a source holds a
    /// firmlinked directory under a second spelling. Last it refuses a source that is the same
    /// directory as, inside or above a guarded directory, naming the first such directory in the
    /// order given.
    pub(crate) fn check(&self, source: &Path) -> Result<(), GuardRefusal> {
        self.check_with(source, Walk::from_root)
    }

    fn check_with(
        &self,
        source: &Path,
        walk: impl Fn(&Path) -> Result<Walk, WalkRefusal>,
    ) -> Result<(), GuardRefusal> {
        let chain = walk(source)
            .and_then(|walk| walk.open_directory(only_a_directory))
            .map_err(|refusal| GuardRefusal::Source {
                source: source.to_path_buf(),
                refusal,
            })?
            .chain;
        if let Some(volume) = &self.data_volume {
            match relation(&chain, &volume.chain) {
                Some(Relation::Above) => {
                    return Err(GuardRefusal::AboveDataVolume {
                        source: source.to_path_buf(),
                        data_volume: volume.path.clone(),
                    });
                }
                Some(Relation::Same | Relation::Inside) => {
                    return Err(GuardRefusal::ThroughDataVolume {
                        what: None,
                        path: source.to_path_buf(),
                        data_volume: volume.path.clone(),
                    });
                }
                None => {}
            }
        }
        for (what, guarded, dirs) in &self.guarded {
            if let Some(relation) = relation(&chain, dirs) {
                return Err(GuardRefusal::Overlap {
                    source: source.to_path_buf(),
                    relation,
                    what: *what,
                    guarded: guarded.clone(),
                });
            }
        }
        Ok(())
    }
}

fn only_a_directory(reached: &Reached<'_>) -> Result<(), WalkRefusal> {
    if reached.facts.kind == Kind::Directory {
        Ok(())
    } else {
        Err(WalkRefusal::NotDirectory {
            path: reached.path.to_path_buf(),
        })
    }
}

/// Why spec 022 §6's guard refuses. `Display` is the one sentence a 108's `detail` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GuardRefusal {
    /// The source does not open with the walk, or is not a directory.
    Source {
        source: PathBuf,
        refusal: WalkRefusal,
    },
    /// The source is the same directory as, inside or above a guarded directory.
    Overlap {
        source: PathBuf,
        relation: Relation,
        what: GuardedKind,
        guarded: PathBuf,
    },
    /// A guarded directory does not open, so no source can be checked.
    Guarded {
        what: GuardedKind,
        path: PathBuf,
        refusal: WalkRefusal,
    },
    /// A guarded directory (`what` its kind) or a source (`what` none) is the data volume or lies
    /// inside it.
    ThroughDataVolume {
        what: Option<GuardedKind>,
        path: PathBuf,
        data_volume: PathBuf,
    },
    /// The source lies above the data volume.
    AboveDataVolume {
        source: PathBuf,
        data_volume: PathBuf,
    },
    /// The data volume does not open for a reason other than a missing name.
    DataVolume { refusal: WalkRefusal },
}

impl fmt::Display for GuardRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuardRefusal::Source { source, refusal } => {
                write!(f, "mount source {} is refused: {refusal}", source.display())
            }
            GuardRefusal::Overlap {
                source,
                relation,
                what,
                guarded,
            } => write!(
                f,
                "mount source {} is {relation} {what} {}",
                source.display(),
                guarded.display()
            ),
            GuardRefusal::Guarded {
                what,
                path,
                refusal,
            } => write!(
                f,
                "{what} {} does not open, so no mount source can be checked against it: \
                 {refusal}",
                path.display()
            ),
            GuardRefusal::ThroughDataVolume {
                what,
                path,
                data_volume,
            } => {
                match what {
                    Some(what) => write!(f, "{what} ")?,
                    None => f.write_str("mount source ")?,
                }
                let volume = data_volume.display();
                write!(
                    f,
                    "{} is reached through the data volume {volume}, {FIRMLINK}; name it by its \
                     path outside {volume}",
                    path.display()
                )
            }
            GuardRefusal::AboveDataVolume {
                source,
                data_volume,
            } => write!(
                f,
                "mount source {} is above the data volume {}, {FIRMLINK}",
                source.display(),
                data_volume.display()
            ),
            GuardRefusal::DataVolume { refusal } => f.write_str(&unopened_volume(refusal)),
        }
    }
}

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

    fn chain_to(start: &Path, relative: &str) -> Vec<DirIdentity> {
        let mut at = start.to_path_buf();
        let mut chain = vec![identity_of(&at)];
        for name in Path::new(relative).iter() {
            at.push(name);
            chain.push(identity_of(&at));
        }
        chain
    }

    fn volume(root: &TestRoot, relative: &str) -> Result<Option<DataVolume>, WalkRefusal> {
        Ok(Some(DataVolume {
            path: root.join(relative),
            identity: identity_of(&root.join(relative)),
            chain: chain_to(root.path(), relative),
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

    fn through(path: PathBuf, data_volume: &Path) -> Result<(), GuardRefusal> {
        Err(GuardRefusal::ThroughDataVolume {
            what: None,
            path,
            data_volume: data_volume.to_path_buf(),
        })
    }

    fn above(source: PathBuf, data_volume: &Path) -> Result<(), GuardRefusal> {
        Err(GuardRefusal::AboveDataVolume {
            source,
            data_volume: data_volume.to_path_buf(),
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
            assert_eq!(
                refusal.reason,
                format!(
                    "{} lies inside mount source {}, which the operator file declares",
                    root.join("S/op.json").display(),
                    root.join("S").display()
                )
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
        let link = root.join("link").display().to_string();
        assert_eq!(
            refusal.reason,
            format!(
                "mount source {link}, which cell-1 holds, does not open: {link} is a symlink, \
                 which the walk does not follow"
            )
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
        assert_eq!(
            refusal.reason,
            format!(
                "{} is reached through the data volume {}, where a firmlink gives a directory a \
                 second parent that no walk from / sees",
                root.join("data/x/op.json").display(),
                root.join("data").display()
            )
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
        let probes = Probes::probe(&[]);
        assert_eq!(probes.check(&firmlinked, None), Ok(()));
        let spelled = data_side(&root.join("op.json"));
        let mut spellings = vec![spelled.clone()];
        if fs::symlink_metadata("/system/volumes/data").is_ok() {
            let below = spelled
                .strip_prefix("/System/Volumes/Data")
                .expect("a data-side spelling");
            spellings.push(Path::new("/system/volumes/data").join(below));
        } else {
            eprintln!(
                "skipped the lower-case spelling: this Mac's system volume is case-sensitive"
            );
        }
        for path in spellings {
            let through =
                TrustedFile::open(&path).expect("the data-side spelling meets rules 1 to 4");
            assert_eq!(
                probes.check(&through, None),
                Err(TrustRefusal {
                    path: path.clone(),
                    check: TrustCheck::InsideMountSource,
                    reason: format!(
                        "{} is reached through the data volume /System/Volumes/Data, where a \
                         firmlink gives a directory a second parent that no walk from / sees",
                        path.display()
                    ),
                })
            );
        }
    }

    #[cfg(target_os = "macos")]
    fn on_host(path: &str) -> Source {
        Source {
            holder: Holder::Declared,
            path: PathBuf::from(path),
        }
    }

    #[cfg(target_os = "macos")]
    fn inside(file: &Path, source: &str) -> String {
        format!(
            "{} lies inside mount source {source}, which the operator file declares",
            file.display()
        )
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_source_at_or_above_the_data_volume_holds_a_firmlinked_file() {
        let root = TestRoot::walkable_from_root();
        write_file(&root.join("op.json"), b"{}", 0o600);
        let file = TrustedFile::open(&root.join("op.json")).expect("the file opens");
        let spelled = data_side(root.path());
        let spelled = spelled.to_str().expect("a UTF-8 temp path");
        let mut rows = vec![
            ("/System", true),
            ("/System/Volumes", true),
            ("/System/Volumes/Data", true),
            ("/System/Volumes/Data/private", true),
            (spelled, true),
            ("/System/Volumes/Data/usr", false),
            ("/System/Volumes/Data/Users", false),
        ];
        if fs::symlink_metadata("/system/volumes").is_ok() {
            rows.push(("/system/volumes", true));
        } else {
            eprintln!("skipped /system/volumes: this Mac's system volume is case-sensitive");
        }
        for (source, holds) in rows {
            let expected = if holds {
                Err(TrustRefusal {
                    path: root.join("op.json"),
                    check: TrustCheck::InsideMountSource,
                    reason: inside(&root.join("op.json"), source),
                })
            } else {
                Ok(())
            };
            let probes = Probes::probe(&[on_host(source)]);
            assert_eq!(probes.check(&file, None), expected, "{source}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_data_side_directory_above_usr_local_holds_it() {
        let local =
            match Walk::from_root(Path::new("/usr/local")).and_then(|walk| walk.identities()) {
                Ok(chain) => chain,
                Err(WalkRefusal::Missing { .. }) => {
                    eprintln!("skipped: this Mac has no /usr/local");
                    return;
                }
                Err(refusal) => panic!("/usr/local does not open: {refusal}"),
            };
        let file = Path::new("/usr/local/op.json");
        for (source, holds) in [
            ("/System/Volumes/Data/usr", true),
            ("/System/Volumes/Data", true),
            ("/System/Volumes/Data/private", false),
        ] {
            let expected = if holds {
                Err(inside(file, source))
            } else {
                Ok(())
            };
            let probes = Probes::probe(&[on_host(source)]);
            assert_eq!(probes.check_at(file, &local, None), expected, "{source}");
        }
    }

    #[test]
    fn a_source_above_the_data_volume_holds_a_directory_a_firmlink_opens() {
        let root = TestRoot::new();
        dirs(
            &root,
            &["x", "x/data", "x/data/firmlinked", "x/data/other", "w"],
        );
        symlink("firmlinked", root.join("x/data/link")).expect("symlink");
        let slash = identity_of(Path::new("/"));
        let firmlinked = [slash, identity_of(&root.join("x/data/firmlinked"))];
        let elsewhere = [slash, identity_of(&root.join("w"))];
        let probe = |sources: &[Source]| {
            Probes::probe_with(sources, walker(&root), volume(&root, "x/data"))
        };
        let shown = Path::new("/firmlinked/op.json");
        for (relative, holds) in [
            ("x", true),
            ("x/data", true),
            ("x/data/firmlinked", true),
            ("x/data/other", false),
            ("w", false),
        ] {
            let expected = if holds {
                Err(format!(
                    "/firmlinked/op.json lies inside mount source {}, which the operator file \
                     declares",
                    root.join(relative).display()
                ))
            } else {
                Ok(())
            };
            let probes = probe(&[declared(&root, relative)]);
            assert_eq!(
                probes.check_at(shown, &firmlinked, None),
                expected,
                "{relative}"
            );
        }
        let probes = probe(&[declared(&root, "x")]);
        for spelled in ["/absent/op.json", "/other/op.json"] {
            assert_eq!(
                probes.check_at(Path::new(spelled), &elsewhere, None),
                Ok(()),
                "{spelled}"
            );
        }
        let data = root.join("x/data").display().to_string();
        assert_eq!(
            probe(&[]).check_at(Path::new("/link/op.json"), &elsewhere, None),
            Err(format!(
                "the data-side spelling {data}/link of /link does not open, so a firmlink cannot \
                 be ruled out: {data}/link is a symlink, which the walk does not follow"
            ))
        );
        write_file(&root.join("x/data/plain"), b"", 0o600);
        assert_eq!(
            probe(&[]).check_at(Path::new("/plain/op.json"), &elsewhere, None),
            Err(format!(
                "the data-side spelling {data}/plain of /plain does not open, so a firmlink \
                 cannot be ruled out: {data}/plain is not a directory"
            ))
        );
    }

    #[test]
    fn a_rename_below_the_firmlink_after_the_open_keeps_the_second_chain() {
        let root = TestRoot::new();
        dirs(
            &root,
            &["x", "x/data", "x/data/firmlinked", "x/data/firmlinked/ops"],
        );
        let chain = [
            identity_of(Path::new("/")),
            identity_of(&root.join("x/data/firmlinked")),
            identity_of(&root.join("x/data/firmlinked/ops")),
        ];
        let probes = Probes::probe_with(
            &[declared(&root, "x")],
            walker(&root),
            volume(&root, "x/data"),
        );
        let shown = Path::new("/firmlinked/ops/op.json");
        let expected = format!(
            "/firmlinked/ops/op.json lies inside mount source {}, which the operator file declares",
            root.join("x").display()
        );
        fs::rename(
            root.join("x/data/firmlinked/ops"),
            root.join("x/data/firmlinked/moved"),
        )
        .expect("rename");
        assert_eq!(
            probes.check_at(shown, &chain, None),
            Err(expected.clone()),
            "renamed away"
        );
        dirs(&root, &["x/data/firmlinked/ops"]);
        assert_eq!(
            probes.check_at(shown, &chain, None),
            Err(expected),
            "another directory under the name"
        );
    }

    #[test]
    fn probe_opens_each_source_from_the_root() {
        let root = TestRoot::new();
        dirs(&root, &["S", "T"]);
        let probes = Probes::probe(&[held("cell-1", &root, "S"), held("cell-2", &root, "gone")]);
        let file = trusted(&root, "S/op.json");
        assert_eq!(
            refused(&probes, &file, Some("cell-3")).reason,
            format!(
                "{} lies inside mount source {}, which cell-1 holds",
                root.join("S/op.json").display(),
                root.join("S").display()
            )
        );
        let beside = trusted(&root, "T/op.json");
        assert_eq!(checked(&probes, &beside, Some("cell-1")), Ok(()));
        let gone = root.join("gone").display().to_string();
        assert_eq!(
            refused(&probes, &beside, Some("cell-2")).reason,
            format!(
                "mount source {gone}, which cell-2 holds, does not open: {gone} does not exist"
            )
        );
    }

    #[test]
    fn a_source_spelled_in_another_case_holds_the_same_directory() {
        let root = TestRoot::new();
        dirs(&root, &["S"]);
        if fs::symlink_metadata(root.join("s")).is_err() {
            eprintln!("skipped: the temp directory is case-sensitive, so s and S are two names");
            return;
        }
        let probes = probe(&root, &[held("cell-1", &root, "s")]);
        let file = trusted(&root, "S/op.json");
        assert_eq!(
            refused(&probes, &file, Some("cell-2")).reason,
            format!(
                "{} lies inside mount source {}, which cell-1 holds",
                root.join("S/op.json").display(),
                root.join("s").display()
            )
        );
    }

    #[test]
    fn every_source_is_checked_not_only_the_first() {
        let root = TestRoot::new();
        dirs(&root, &["A", "S"]);
        let file = trusted(&root, "S/op.json");
        let expected = format!(
            "{} lies inside mount source {}, which the operator file declares",
            root.join("S/op.json").display(),
            root.join("S").display()
        );
        for first in [
            declared(&root, "A"),
            declared(&root, "missing"),
            held("cell-1", &root, "missing"),
        ] {
            let probes = probe(&root, &[first, declared(&root, "S")]);
            assert_eq!(refused(&probes, &file, Some("cell-2")).reason, expected);
        }
    }

    #[test]
    fn a_held_source_its_owner_cannot_list_still_holds_its_files() {
        let root = TestRoot::new();
        dirs(&root, &["S"]);
        write_file(&root.join("S/op.json"), b"{}", 0o600);
        set_mode(&root.join("S"), 0o300);
        let file = TrustedFile::from_walk(root.walk("S/op.json")).expect("a 0300 directory opens");
        let probes = probe(&root, &[held("cell-1", &root, "S")]);
        assert_eq!(
            refused(&probes, &file, Some("cell-2")).reason,
            format!(
                "{} lies inside mount source {}, which cell-1 holds",
                root.join("S/op.json").display(),
                root.join("S").display()
            )
        );
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
            chain: chain_to(Path::new("/"), "System/Volumes/Data"),
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
        write_file(&root.join("unreadable"), b"", 0o000);
        let guard = guard(&root, &[(InstanceRoot, "inst")]).expect("opens");
        let at = |relative: &str| root.join(relative);
        for (relative, refusal) in [
            ("link", WalkRefusal::Symlink { path: at("link") }),
            ("alias/x", WalkRefusal::Symlink { path: at("alias") }),
            ("file", WalkRefusal::NotDirectory { path: at("file") }),
            (
                "unreadable",
                WalkRefusal::NotDirectory {
                    path: at("unreadable"),
                },
            ),
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
            eprintln!("skipped the unsearchable case: root searches every directory");
            return;
        }
        dirs(&root, &["locked", "locked/inst"]);
        set_mode(&root.join("locked"), 0o000);
        let outcome = guard(&root, &[(InstanceRoot, "locked/inst")]).map(|_| ());
        set_mode(&root.join("locked"), 0o700);
        let found = match outcome {
            Err(GuardRefusal::Guarded {
                what,
                path,
                refusal: WalkRefusal::Io { errno, .. },
            }) => Some((what, path, errno)),
            _ => None,
        };
        assert_eq!(
            found,
            Some((InstanceRoot, root.join("locked/inst"), libc::EACCES))
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
        if fs::symlink_metadata(root.join("INST")).is_err() {
            eprintln!("skipped: this volume is case-sensitive, so INST is another name");
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
                through(root.join(relative), &data)
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

    #[test]
    fn a_source_at_or_above_an_injected_data_volume_is_refused() {
        let root = TestRoot::new();
        dirs(
            &root,
            &[
                "sys",
                "sys/vol",
                "sys/vol/data",
                "sys/vol/data/usr",
                "inst",
                "z",
            ],
        );
        let data = root.join("sys/vol/data");
        let inst = [(InstanceRoot, root.join("inst"))];
        let guard = Guard::new_with(&inst, &[], walker(&root), volume(&root, "sys/vol/data"))
            .expect("opens");
        for relative in ["sys", "sys/vol"] {
            assert_eq!(
                source(&guard, &root, relative),
                above(root.join(relative), &data),
                "{relative}"
            );
        }
        for relative in ["sys/vol/data", "sys/vol/data/usr"] {
            assert_eq!(
                source(&guard, &root, relative),
                through(root.join(relative), &data),
                "{relative}"
            );
        }
        assert_eq!(source(&guard, &root, "z"), Ok(()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_volumes_data_is_refused_over_a_firmlinked_guarded_directory() {
        let root = TestRoot::walkable_from_root();
        for relative in ["inst", "src", "ops"] {
            make_dir(&root.join(relative), 0o700);
        }
        write_file(&root.join("ops/operator.json"), b"{}", 0o600);
        let file = TrustedFile::open(&root.join("ops/operator.json")).expect("the file opens");
        for relative in ["inst", "ops"] {
            assert_eq!(
                identity_of(&data_side(&root.join(relative))),
                identity_of(&root.join(relative)),
                "a firmlink opens {relative}"
            );
        }
        let volume = PathBuf::from("/System/Volumes/Data");
        let guard = Guard::new(
            &[(InstanceRoot, root.join("inst"))],
            &[(OperatorFile, &file)],
        )
        .expect("opens");
        let mut over = vec!["/", "/System", "/System/Volumes"];
        if fs::symlink_metadata("/system/volumes").is_ok() {
            over.push("/system/volumes");
        } else {
            eprintln!("skipped /system/volumes: this Mac's system volume is case-sensitive");
        }
        for path in over {
            assert_eq!(
                guard.check(Path::new(path)),
                above(PathBuf::from(path), &volume),
                "{path}"
            );
        }
        let spelled = data_side(&root.join("src"));
        for path in [
            volume.clone(),
            volume.join("usr"),
            volume.join("private"),
            spelled,
        ] {
            assert_eq!(guard.check(&path), through(path.clone(), &volume));
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
    fn the_guard_walks_every_path_from_the_root() {
        let root = TestRoot::walkable_from_root();
        for relative in ["inst", "inst/sub", "src", "ops"] {
            make_dir(&root.join(relative), 0o700);
        }
        write_file(&root.join("ops/operator.json"), b"{}", 0o600);
        let file = TrustedFile::open(&root.join("ops/operator.json")).expect("the file opens");
        let guard = Guard::new(
            &[(InstanceRoot, root.join("inst"))],
            &[(OperatorFile, &file)],
        )
        .expect("opens");
        assert_eq!(guard.check(&root.join("src")), Ok(()));
        assert_eq!(
            guard.check(&root.join("inst/sub")),
            overlap(&root, "inst/sub", Inside, InstanceRoot, "inst")
        );
        assert_eq!(
            guard.check(&root.join("ops")),
            overlap(&root, "ops", Same, OperatorFile, "ops")
        );
        assert_eq!(
            Guard::new(&[(InstanceRoot, root.join("absent"))], &[]).map(|_| ()),
            Err(GuardRefusal::Guarded {
                what: InstanceRoot,
                path: root.join("absent"),
                refusal: WalkRefusal::Missing {
                    path: root.join("absent")
                }
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
                GuardRefusal::AboveDataVolume {
                    source: at("/System/Volumes"),
                    data_volume: at("/System/Volumes/Data"),
                },
                "mount source /System/Volumes is above the data volume /System/Volumes/Data, \
                 where a firmlink gives a directory a second parent that no walk from / sees",
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
