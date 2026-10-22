use std::ffi::{CStr, CString};
use std::fmt;
use std::fs::{File, Permissions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use plasmosome_core::private_socket::{PrivateDir, PrivateSocketError};

use super::RemoveOutcome;
use super::artifacts::{ArtifactFault, FileIdentity, Positioned, VerifiedArtifact};
use super::digest::Digest;
use super::path::RecipePath;
use super::recipe::ArtifactRole;

const NEW_COPY_FLAGS: libc::c_int =
    libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
const COPY_MODE: u32 = 0o600;

/// A cell's private directory, the one spec 001 §4.2 puts `writable_root`, `control_path` and
/// `data_path` under, checked by core's `PrivateDir` and kept open.
///
/// Get one from [`PrivateDirectory::open`]. The membrane does not create it: in cell mode it is
/// spec 023's `runtime/`, which `membraned` creates through core. A later rename of the path does
/// not move what is made through this value, because it holds the directory open.
#[derive(Debug)]
pub struct PrivateDirectory {
    path: RecipePath,
    dir: PrivateDir,
}

impl PrivateDirectory {
    /// Opens `path` with core's `PrivateDir::open`: walked from "/" without following a symlink,
    /// every ancestor owned by root or the effective UID and writable by neither group nor other,
    /// and `path` itself a directory owned by the effective UID with no group or other bits and
    /// no ACL. On macOS pass the canonical path, `/private/var/...` rather than `/var/...`.
    pub fn open(path: &RecipePath) -> Result<PrivateDirectory, CellDirectoryRefusal> {
        let dir = PrivateDir::open(path.as_path())
            .map_err(|refusal| refused(path, CellDirectoryFault::Private(refusal)))?;
        Ok(PrivateDirectory {
            path: path.clone(),
            dir,
        })
    }

    /// The path it was opened at.
    pub fn path(&self) -> &RecipePath {
        &self.path
    }

    /// The device and inode of the directory held open.
    pub fn identity(&self) -> FileIdentity {
        let identity = self.dir.identity();
        FileIdentity {
            dev: identity.dev,
            ino: identity.ino,
        }
    }

    /// Walks the path again with the same checks and requires the directory held open. A
    /// directory renamed away and replaced at the path is `Private(Replaced)`. Call it just before
    /// a launch that relies on the path.
    pub fn reconfirm(&self) -> Result<(), CellDirectoryRefusal> {
        self.dir
            .reconfirm()
            .map_err(|refusal| refused(&self.path, CellDirectoryFault::Private(refusal)))
    }
}

/// The per-cell RAW copy of the verified root image that spec 001 §4.2 calls `writable_root`.
///
/// Only [`WritableRoot::create`] makes one, as a new file inside a [`PrivateDirectory`]. It keeps
/// the file and that directory open. [`WritableRoot::remove`] inspects the name in the held
/// directory and unlinks it only if it still holds the file it created. The inspection and the
/// unlink are two steps, so another writer can put an object at the name between them. Dropping
/// it does the same, so a refusal or a panic after the copy removes the copy, unless that removal
/// fails, finds another object at the name, or finds the name gone because the copy was renamed
/// away.
#[derive(Debug)]
pub struct WritableRoot {
    path: RecipePath,
    parent: OwnedFd,
    name: CString,
    file: File,
    identity: FileIdentity,
    removed: bool,
}

impl WritableRoot {
    /// Creates `path` inside `parent` and copies the verified root image into it.
    ///
    /// `source` must be the recipe's root image, and `path`'s parent must be the path `parent` was
    /// opened at, which must still pass [`PrivateDirectory::reconfirm`]; that refusal carries the
    /// directory's path, not `path`. The name is created with `O_CREAT | O_EXCL | O_NOFOLLOW`, so
    /// any entry already there, a symlink included, is `Exists` and is left as it was. The file is
    /// set to mode 0600 whatever the umask. The bytes are read through `source.file()` with
    /// positioned reads, at most one byte past the size it was verified at, and hashed while they
    /// are written; a digest other than `source.digest()` is `CopyMismatch`. The copy is synced,
    /// then `source.reconfirm()` must pass, or the refusal is `Source`.
    ///
    /// A refusal after the new file's `fstat` removes the copy by identity, as
    /// [`WritableRoot::remove`] does, and only as far as that removal can: one that fails, or
    /// finds another object at the name, leaves what is there. If that `fstat` fails, right after
    /// the file is created, the empty file stays at the name, since there is no identity to
    /// remove it by.
    pub fn create(
        path: &RecipePath,
        parent: &PrivateDirectory,
        source: &VerifiedArtifact,
    ) -> Result<WritableRoot, CellDirectoryRefusal> {
        if source.role() != ArtifactRole::RootImage {
            return Err(refused(
                path,
                CellDirectoryFault::WrongSource {
                    role: source.role(),
                },
            ));
        }
        let bytes = Positioned {
            file: source.file(),
            offset: 0,
        }
        .take(source.size().saturating_add(1));
        let root = create_copy(path, parent, bytes, source.digest())?;
        source
            .reconfirm()
            .map_err(|refusal| refused(path, CellDirectoryFault::Source(refusal.fault)))?;
        Ok(root)
    }

    /// The path it was created at.
    pub fn path(&self) -> &RecipePath {
        &self.path
    }

    /// The device and inode of the file it created.
    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    /// Removes the file only while the name in the held directory is still that regular file, by
    /// device and inode, and says what it found. A name that is gone is `AlreadyGone`; another
    /// object at the name is `LeftReplacement` and is not touched. The inspection and the unlink
    /// are two steps, so another writer with authority over the directory, the same UID included,
    /// can put an object at the name between them.
    pub fn remove(mut self) -> RemoveOutcome {
        self.remove_entry()
    }

    #[allow(unsafe_code)]
    fn remove_entry(&mut self) -> RemoveOutcome {
        self.removed = true;
        match stat_at(self.parent.as_fd(), &self.name) {
            Ok(found) if found.regular && found.identity == self.identity => {}
            Ok(_) => return RemoveOutcome::LeftReplacement,
            Err(errno) => return gone_or_failed(errno),
        }
        // SAFETY: `self.name` is a CString this value owns, and `self.parent` keeps the directory
        // descriptor open for the call. unlinkat keeps no pointer to the name.
        if unsafe { libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0) } == 0 {
            RemoveOutcome::Removed
        } else {
            gone_or_failed(
                io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or(libc::EIO),
            )
        }
    }
}

impl Drop for WritableRoot {
    fn drop(&mut self) {
        if !self.removed {
            let _ = self.remove_entry();
        }
    }
}

fn create_copy(
    path: &RecipePath,
    parent: &PrivateDirectory,
    reader: impl Read,
    expected: &Digest,
) -> Result<WritableRoot, CellDirectoryRefusal> {
    let refuse = |fault| refused(path, fault);
    let (Some(directory), Ok(name)) = (path.parent(), CString::new(path.file_name())) else {
        return Err(refuse(CellDirectoryFault::WrongParent));
    };
    if directory != parent.path {
        return Err(refuse(CellDirectoryFault::WrongParent));
    }
    parent.reconfirm()?;
    let held = parent
        .dir
        .as_fd()
        .try_clone_to_owned()
        .map_err(|error| refuse(io_fault("dup", &error)))?;
    let file = match open_new(held.as_fd(), &name) {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
            return Err(refuse(CellDirectoryFault::Exists));
        }
        Err(error) => return Err(refuse(io_fault("openat", &error))),
    };
    let metadata = file
        .metadata()
        .map_err(|error| refuse(io_fault("fstat", &error)))?;
    let root = WritableRoot {
        path: path.clone(),
        parent: held,
        name,
        file,
        identity: FileIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
        },
        removed: false,
    };
    root.file
        .set_permissions(Permissions::from_mode(COPY_MODE))
        .map_err(|error| refuse(io_fault("fchmod", &error)))?;
    copy_verified(reader, &root.file, expected).map_err(refuse)?;
    root.file
        .sync_all()
        .map_err(|error| refuse(io_fault("fsync", &error)))?;
    Ok(root)
}

fn copy_verified(
    reader: impl Read,
    writer: &File,
    expected: &Digest,
) -> Result<(), CellDirectoryFault> {
    let mut tee = Tee {
        reader,
        writer,
        write_failed: false,
    };
    let actual = Digest::of_reader(&mut tee)
        .map_err(|error| io_fault(if tee.write_failed { "write" } else { "pread" }, &error))?;
    if actual == *expected {
        Ok(())
    } else {
        Err(CellDirectoryFault::CopyMismatch {
            expected: *expected,
            actual,
        })
    }
}

struct Tee<'a, R> {
    reader: R,
    writer: &'a File,
    write_failed: bool,
}

impl<R: Read> Read for Tee<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.reader.read(buffer)?;
        self.writer.write_all(&buffer[..read]).inspect_err(|_| {
            self.write_failed = true;
        })?;
        Ok(read)
    }
}

#[allow(unsafe_code)]
fn open_new(dir: BorrowedFd<'_>, name: &CStr) -> io::Result<File> {
    // SAFETY: `name` is a CStr that outlives the call, and the BorrowedFd keeps `dir` open for it.
    // openat keeps no pointer to the name.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            NEW_COPY_FLAGS,
            COPY_MODE as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a non-negative descriptor just above, which nothing else owns, so
    // this File becomes its only owner.
    Ok(unsafe { File::from_raw_fd(fd) })
}

struct Found {
    regular: bool,
    identity: FileIdentity,
}

#[allow(unsafe_code)]
fn stat_at(dir: BorrowedFd<'_>, name: &CStr) -> Result<Found, i32> {
    // SAFETY: libc::stat is a plain C struct, for which all zeroes is a valid value.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `name` is a CStr that outlives the call, the BorrowedFd keeps `dir` open for it, and
    // `stat` is a local libc::stat that fstatat may write.
    let outcome = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if outcome != 0 {
        return Err(io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EIO));
    }
    Ok(Found {
        regular: stat.st_mode & libc::S_IFMT == libc::S_IFREG,
        identity: FileIdentity {
            dev: device(&stat),
            ino: stat.st_ino,
        },
    })
}

#[cfg(target_os = "macos")]
fn device(stat: &libc::stat) -> u64 {
    stat.st_dev as u64
}

#[cfg(not(target_os = "macos"))]
fn device(stat: &libc::stat) -> u64 {
    stat.st_dev
}

fn gone_or_failed(errno: i32) -> RemoveOutcome {
    if errno == libc::ENOENT {
        RemoveOutcome::AlreadyGone
    } else {
        RemoveOutcome::Failed { errno }
    }
}

fn io_fault(op: &'static str, error: &io::Error) -> CellDirectoryFault {
    CellDirectoryFault::Io {
        op,
        errno: error.raw_os_error().unwrap_or(libc::EIO),
    }
}

fn refused(path: &RecipePath, fault: CellDirectoryFault) -> CellDirectoryRefusal {
    CellDirectoryRefusal {
        path: path.clone(),
        fault,
    }
}

/// Why a cell directory or a writable root was refused: a path and the first rule it broke. The
/// path is the one asked for, except that a private directory that no longer reconfirms inside
/// [`WritableRoot::create`] gives the directory's path. A refusal from `create` leaves no file it
/// made, with three exceptions: the empty new file stays if its `fstat` fails; the copy stays if
/// removing it fails or finds another object at the name; and a copy renamed away stays under its
/// new name, because its removal then finds the name gone, or unlinks an object put at the name
/// between the inspection and the unlink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellDirectoryRefusal {
    pub path: RecipePath,
    pub fault: CellDirectoryFault,
}

/// The first rule a cell directory or a writable root broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellDirectoryFault {
    /// Core's private directory refused. Its error names the rule and the component that broke
    /// it: a symlink on the way, a replaceable ancestor, a directory that is not private, one
    /// that carries an ACL, or one that no longer names the directory held open (`Replaced`).
    Private(PrivateSocketError),
    /// The writable root's parent is not the path the [`PrivateDirectory`] was opened at.
    WrongParent,
    /// The source is this artifact of the recipe, not its root image.
    WrongSource { role: ArtifactRole },
    /// An entry is already at the name: a file, a directory, a socket or a symlink, dangling or
    /// not. It was left as it was.
    Exists,
    /// The bytes copied hash to `actual`, not the verified `expected`.
    CopyMismatch { expected: Digest, actual: Digest },
    /// After the copy, the root image's `reconfirm` refused, and this is the fault it returned:
    /// `Replaced` when its path names another file, `ChangedSinceVerification` when its size or
    /// times moved since it was verified, or any other fault of the checks `reconfirm` repeats,
    /// such as a walk, file type, owner, mode, ACL, immutability or `Io` fault.
    Source(ArtifactFault),
    /// A call failed with this errno.
    Io { op: &'static str, errno: i32 },
}

impl fmt::Display for CellDirectoryRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.as_str(), self.fault)
    }
}

impl std::error::Error for CellDirectoryRefusal {}

impl fmt::Display for CellDirectoryFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CellDirectoryFault::Private(refusal) => write!(f, "{refusal}"),
            CellDirectoryFault::WrongParent => {
                f.write_str("its parent is not the private directory it was asked to go in")
            }
            CellDirectoryFault::WrongSource { role } => {
                write!(f, "the source is the {role}, not the root image")
            }
            CellDirectoryFault::Exists => {
                f.write_str("an entry already exists and was left as it was")
            }
            CellDirectoryFault::CopyMismatch { expected, actual } => {
                write!(
                    f,
                    "the copy hashes to {actual}, not the verified {expected}"
                )
            }
            CellDirectoryFault::Source(fault) => {
                write!(
                    f,
                    "the root image did not pass its reconfirm after the copy: {fault}"
                )
            }
            CellDirectoryFault::Io { op, errno } => write!(f, "{op} failed with errno {errno}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_fixtures::private_root;
    use crate::runtime::test_fixtures::tree::set_mode;
    use plasmosome_core::private_socket::effective_uid;
    use std::fs;
    use std::io::{self, Seek, SeekFrom};
    use std::os::unix::fs::{MetadataExt, symlink};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn recipe_path(path: &Path) -> RecipePath {
        RecipePath::parse(path.to_str().expect("test paths are UTF-8"))
            .expect("test paths are canonical")
    }

    fn make_dir(path: &Path, mode: u32) -> PathBuf {
        fs::create_dir(path).expect("mkdir");
        set_mode(path, mode);
        path.to_path_buf()
    }

    fn open_at(path: &Path) -> Result<PrivateDirectory, CellDirectoryRefusal> {
        PrivateDirectory::open(&recipe_path(path))
    }

    fn opened(path: &Path) -> PrivateDirectory {
        open_at(path).expect("the private directory opens")
    }

    fn fault_of<T: fmt::Debug>(outcome: Result<T, CellDirectoryRefusal>) -> CellDirectoryFault {
        outcome.expect_err("the call is refused").fault
    }

    fn private(refusal: PrivateSocketError) -> CellDirectoryFault {
        CellDirectoryFault::Private(refusal)
    }

    fn identity_of(path: &Path) -> FileIdentity {
        let metadata = fs::symlink_metadata(path).expect("lstat");
        FileIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }

    fn absent(path: &Path) -> bool {
        matches!(fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
    }

    fn copy_of(
        path: &Path,
        parent: &PrivateDirectory,
        bytes: &[u8],
    ) -> Result<WritableRoot, CellDirectoryRefusal> {
        create_copy(&recipe_path(path), parent, bytes, &Digest::of(bytes))
    }

    struct Cell {
        _root: tempfile::TempDir,
        base: PathBuf,
        dir: PathBuf,
    }

    impl Cell {
        fn new() -> Cell {
            let root = private_root();
            let base = root.path().to_path_buf();
            let dir = make_dir(&base.join("cell"), 0o700);
            Cell {
                _root: root,
                base,
                dir,
            }
        }

        fn root_img(&self) -> PathBuf {
            self.dir.join("root.img")
        }
    }

    #[test]
    fn a_private_directory_is_owned_by_the_effective_uid_with_no_group_or_other_bit() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        assert_eq!(dir.path().as_path(), cell.dir);
        assert_eq!(dir.identity(), identity_of(&cell.dir));
        for (name, mode) in [("group", 0o750), ("other", 0o701), ("shared", 0o770)] {
            let path = make_dir(&cell.base.join(name), mode);
            assert_eq!(
                fault_of(open_at(&path)),
                private(PrivateSocketError::NotPrivate { path, mode }),
                "{name}"
            );
        }
        let file = cell.base.join("file");
        fs::write(&file, b"").expect("write");
        set_mode(&file, 0o600);
        assert_eq!(
            fault_of(open_at(&file)),
            private(PrivateSocketError::NotDirectory { path: file })
        );
    }

    #[test]
    fn a_world_readable_directory_or_one_reached_through_a_symlink_is_refused() {
        let cell = Cell::new();
        let open = make_dir(&cell.base.join("open"), 0o755);
        assert_eq!(
            fault_of(open_at(&open)),
            private(PrivateSocketError::NotPrivate {
                path: open,
                mode: 0o755
            })
        );
        let link = cell.base.join("link");
        symlink(&cell.dir, &link).expect("symlink");
        assert_eq!(
            fault_of(open_at(&link)),
            private(PrivateSocketError::SymlinkInPath { path: link.clone() })
        );
        make_dir(&cell.dir.join("inner"), 0o700);
        assert_eq!(
            fault_of(open_at(&link.join("inner"))),
            private(PrivateSocketError::SymlinkInPath { path: link })
        );
    }

    #[test]
    fn reconfirm_refuses_a_directory_renamed_away_and_replaced() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        assert_eq!(dir.reconfirm(), Ok(()));
        fs::rename(&cell.dir, cell.base.join("moved")).expect("rename");
        make_dir(&cell.dir, 0o700);
        let refusal = dir
            .reconfirm()
            .expect_err("the path names another directory");
        assert_eq!(refusal.path.as_path(), cell.dir);
        assert_eq!(
            refusal.fault,
            private(PrivateSocketError::Replaced {
                path: cell.dir.clone()
            })
        );
    }

    #[test]
    fn a_copy_whose_bytes_match_is_a_new_private_file_holding_them() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let bytes: Vec<u8> = (0..(3 << 20) + 17).map(|at: u32| at as u8).collect();
        let root = copy_of(&cell.root_img(), &dir, &bytes).expect("the copy is made");
        assert_eq!(root.path().as_path(), cell.root_img());
        assert_eq!(root.identity(), identity_of(&cell.root_img()));
        let metadata = fs::symlink_metadata(cell.root_img()).expect("lstat");
        assert!(metadata.is_file());
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(metadata.nlink(), 1);
        assert_eq!(metadata.uid(), effective_uid());
        assert_eq!(fs::read(cell.root_img()).expect("read"), bytes);
    }

    const UMASK_PROBE: &str = "PLASMOSOME_CELL_DIRECTORY_UMASK_PROBE";
    const PROBE_DEADLINE: Duration = Duration::from_secs(30);

    #[test]
    fn the_copy_is_0600_under_a_umask_that_clears_the_owner_bits() {
        let probe = "runtime::cell_directory::tests::probe_a_copy_under_umask_0377";
        let output = tempfile::tempfile().expect("the probe has a file for its output");
        let mut command = Command::new(std::env::current_exe().expect("the test binary's path"));
        command
            .args(["--exact", probe, "--ignored", "--test-threads=1"])
            .env(UMASK_PROBE, "1")
            .stdin(Stdio::null())
            .stdout(output.try_clone().expect("the output file is shared"))
            .stderr(output.try_clone().expect("the output file is shared"));
        let spawned = {
            let _no_descriptor_mid_creation = crate::spawn_lock::hold_fork_side();
            command.spawn()
        };
        let mut child = spawned.expect("the probe starts");
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().expect("the probe's status") {
                break Some(status);
            }
            if started.elapsed() >= PROBE_DEADLINE {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut said = String::new();
        let mut output = output;
        let _ = output
            .seek(SeekFrom::Start(0))
            .and_then(|_| output.read_to_string(&mut said));
        assert_eq!(status.map(|status| status.success()), Some(true), "{said}");
        assert!(said.contains("1 passed"), "{said}");
    }

    #[test]
    #[ignore = "helper process for the_copy_is_0600_under_a_umask_that_clears_the_owner_bits"]
    #[allow(unsafe_code)]
    fn probe_a_copy_under_umask_0377() {
        if std::env::var_os(UMASK_PROBE).is_none() {
            return;
        }
        // SAFETY: umask takes a mode and returns the old one. This probe process runs only this
        // test, so no other thread creates a file under the changed mask.
        unsafe { libc::umask(0o377) };
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let _root = copy_of(&cell.root_img(), &dir, b"bytes").expect("the copy is made");
        let mode = fs::symlink_metadata(cell.root_img()).expect("lstat").mode();
        assert_eq!(mode & 0o7777, 0o600);
    }

    #[test]
    fn a_copy_whose_bytes_miss_the_digest_is_refused_and_leaves_no_file() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let expected = Digest::of(b"the root image");
        let outcome = create_copy(
            &recipe_path(&cell.root_img()),
            &dir,
            &b"other bytes"[..],
            &expected,
        );
        assert_eq!(
            fault_of(outcome),
            CellDirectoryFault::CopyMismatch {
                expected,
                actual: Digest::of(b"other bytes"),
            }
        );
        assert!(absent(&cell.root_img()));
    }

    struct FailsAfter(usize);

    impl Read for FailsAfter {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.0 == 0 {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            let read = self.0.min(buffer.len());
            buffer[..read].fill(7);
            self.0 -= read;
            Ok(read)
        }
    }

    #[test]
    fn a_copy_whose_read_fails_is_refused_and_leaves_no_file() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let outcome = create_copy(
            &recipe_path(&cell.root_img()),
            &dir,
            FailsAfter(100_000),
            &Digest::of(b"the root image"),
        );
        assert_eq!(
            fault_of(outcome),
            CellDirectoryFault::Io {
                op: "pread",
                errno: libc::EIO
            }
        );
        assert!(absent(&cell.root_img()));
    }

    #[test]
    fn a_copy_whose_write_fails_is_an_io_refusal_named_write() {
        let cell = Cell::new();
        let path = cell.dir.join("read-only");
        fs::write(&path, b"").expect("write");
        let read_only = File::open(&path).expect("open");
        assert_eq!(
            copy_verified(&b"bytes"[..], &read_only, &Digest::of(b"bytes")),
            Err(CellDirectoryFault::Io {
                op: "write",
                errno: libc::EBADF
            })
        );
    }

    #[test]
    fn an_entry_already_at_the_name_is_refused_as_exists_and_left_as_it_was() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let name = cell.root_img();
        let victim = cell.base.join("victim");
        fs::write(&victim, b"victim bytes").expect("write");
        set_mode(&victim, 0o644);
        let victim_identity = identity_of(&victim);

        fs::write(&name, b"old bytes").expect("write");
        let before = identity_of(&name);
        assert_eq!(
            fault_of(copy_of(&name, &dir, b"new")),
            CellDirectoryFault::Exists
        );
        assert_eq!(fs::read(&name).expect("read"), b"old bytes");
        assert_eq!(identity_of(&name), before);
        fs::remove_file(&name).expect("rm");

        let nowhere = cell.base.join("nowhere");
        symlink(&nowhere, &name).expect("symlink");
        assert_eq!(
            fault_of(copy_of(&name, &dir, b"new")),
            CellDirectoryFault::Exists
        );
        assert!(absent(&nowhere));
        assert_eq!(fs::read_link(&name).expect("readlink"), nowhere);
        fs::remove_file(&name).expect("rm");

        symlink(&victim, &name).expect("symlink");
        assert_eq!(
            fault_of(copy_of(&name, &dir, b"new")),
            CellDirectoryFault::Exists
        );
        assert_eq!(fs::read(&victim).expect("read"), b"victim bytes");
        assert_eq!(identity_of(&victim), victim_identity);
        let victim_mode = fs::metadata(&victim).expect("stat").mode() & 0o7777;
        assert_eq!(victim_mode, 0o644);
        fs::remove_file(&name).expect("rm");

        make_dir(&name, 0o700);
        assert_eq!(
            fault_of(copy_of(&name, &dir, b"new")),
            CellDirectoryFault::Exists
        );
        assert!(fs::symlink_metadata(&name).expect("lstat").is_dir());
    }

    #[test]
    fn a_writable_root_outside_the_private_directory_is_refused_as_wrong_parent() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let other = make_dir(&cell.base.join("other"), 0o700);
        let nested = make_dir(&cell.dir.join("nested"), 0o700);
        for path in [
            other.join("root.img"),
            nested.join("root.img"),
            cell.base.join("root.img"),
        ] {
            assert_eq!(
                fault_of(copy_of(&path, &dir, b"bytes")),
                CellDirectoryFault::WrongParent,
                "{}",
                path.display()
            );
            assert!(absent(&path), "{}", path.display());
        }
        assert!(absent(&cell.root_img()));
        assert_eq!(
            fault_of(copy_of(Path::new("/root.img"), &dir, b"bytes")),
            CellDirectoryFault::WrongParent
        );
    }

    #[test]
    fn a_private_directory_replaced_before_the_copy_is_refused_and_nothing_is_created() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let moved = cell.base.join("moved");
        fs::rename(&cell.dir, &moved).expect("rename");
        make_dir(&cell.dir, 0o700);
        let refusal = copy_of(&cell.root_img(), &dir, b"bytes").expect_err("the copy is refused");
        assert_eq!(refusal.path.as_path(), cell.dir);
        assert_eq!(
            refusal.fault,
            private(PrivateSocketError::Replaced {
                path: cell.dir.clone()
            })
        );
        assert!(absent(&cell.root_img()));
        assert!(absent(&moved.join("root.img")));
    }

    #[test]
    fn a_directory_that_refuses_the_create_is_an_io_refusal() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        set_mode(&cell.dir, 0o500);
        let outcome = copy_of(&cell.root_img(), &dir, b"bytes");
        set_mode(&cell.dir, 0o700);
        assert_eq!(
            fault_of(outcome),
            CellDirectoryFault::Io {
                op: "openat",
                errno: libc::EACCES
            }
        );
        assert!(absent(&cell.root_img()));
    }

    #[test]
    fn remove_reports_removed_left_replacement_or_already_gone() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let name = cell.root_img();

        let root = copy_of(&name, &dir, b"first").expect("the copy is made");
        assert_eq!(root.remove(), RemoveOutcome::Removed);
        assert!(absent(&name));

        let root = copy_of(&name, &dir, b"second").expect("the copy is made");
        let moved = cell.dir.join("moved.img");
        fs::rename(&name, &moved).expect("rename");
        fs::write(&name, b"replacement").expect("write");
        assert_eq!(root.remove(), RemoveOutcome::LeftReplacement);
        assert_eq!(fs::read(&moved).expect("read"), b"second");
        assert_eq!(fs::read(&name).expect("read"), b"replacement");
        fs::remove_file(&name).expect("rm");

        let root = copy_of(&name, &dir, b"third").expect("the copy is made");
        fs::remove_file(&name).expect("rm");
        assert_eq!(root.remove(), RemoveOutcome::AlreadyGone);
    }

    #[test]
    fn a_symlink_to_the_moved_copy_is_left_as_a_replacement() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let name = cell.root_img();
        let root = copy_of(&name, &dir, b"bytes").expect("the copy is made");
        let moved = cell.dir.join("moved.img");
        fs::rename(&name, &moved).expect("rename");
        symlink(&moved, &name).expect("symlink");
        assert_eq!(root.remove(), RemoveOutcome::LeftReplacement);
        assert_eq!(fs::read_link(&name).expect("the symlink stays"), moved);
        assert_eq!(fs::read(&moved).expect("the copy stays"), b"bytes");
    }

    #[test]
    fn remove_reports_a_failed_inspection_or_unlink_and_leaves_the_copy() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let name = cell.root_img();
        let failed = RemoveOutcome::Failed {
            errno: libc::EACCES,
        };

        let root = copy_of(&name, &dir, b"bytes").expect("the copy is made");
        set_mode(&cell.dir, 0o500);
        let outcome = root.remove();
        set_mode(&cell.dir, 0o700);
        assert_eq!(outcome, failed, "no write permission: the unlink fails");
        assert_eq!(fs::read(&name).expect("read"), b"bytes");
        fs::remove_file(&name).expect("rm");

        let root = copy_of(&name, &dir, b"bytes").expect("the copy is made");
        set_mode(&cell.dir, 0o000);
        let outcome = root.remove();
        set_mode(&cell.dir, 0o700);
        assert_eq!(
            outcome, failed,
            "no search permission: the inspection fails"
        );
        assert_eq!(fs::read(&name).expect("read"), b"bytes");
    }

    #[test]
    fn dropping_a_writable_root_removes_its_copy_and_leaves_a_replacement() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let name = cell.root_img();
        drop(copy_of(&name, &dir, b"bytes").expect("the copy is made"));
        assert!(absent(&name));

        let root = copy_of(&name, &dir, b"bytes").expect("the copy is made");
        fs::rename(&name, cell.dir.join("moved.img")).expect("rename");
        fs::write(&name, b"replacement").expect("write");
        drop(root);
        assert_eq!(fs::read(&name).expect("read"), b"replacement");
        assert_eq!(
            fs::read(cell.dir.join("moved.img")).expect("read"),
            b"bytes"
        );
    }

    #[test]
    fn remove_reaches_the_copy_in_a_renamed_directory_and_leaves_the_old_path_alone() {
        let cell = Cell::new();
        let dir = opened(&cell.dir);
        let root = copy_of(&cell.root_img(), &dir, b"bytes").expect("the copy is made");
        let moved = cell.base.join("moved");
        fs::rename(&cell.dir, &moved).expect("rename");
        make_dir(&cell.dir, 0o700);
        fs::write(cell.root_img(), b"decoy").expect("write");
        assert_eq!(root.remove(), RemoveOutcome::Removed);
        assert!(absent(&moved.join("root.img")));
        assert_eq!(fs::read(cell.root_img()).expect("read"), b"decoy");
    }

    #[test]
    fn each_fault_says_what_was_found() {
        use CellDirectoryFault::*;
        let (expected, actual) = (Digest::of(b"a"), Digest::of(b"b"));
        let mismatch = format!("{actual}, not the verified {expected}");
        let rows = [
            (
                Private(PrivateSocketError::NotPrivate {
                    path: "/a/cell".into(),
                    mode: 0o750,
                }),
                "/a/cell has mode 0750",
            ),
            (WrongParent, "not the private directory"),
            (
                WrongSource {
                    role: ArtifactRole::Kernel,
                },
                "kernel, not the root image",
            ),
            (Exists, "already exists and was left"),
            (CopyMismatch { expected, actual }, &mismatch),
            (
                Source(ArtifactFault::ChangedSinceVerification),
                "the root image did not pass its reconfirm after the copy: its size or times moved",
            ),
            (
                Source(ArtifactFault::Io {
                    op: "fstat",
                    errno: 5,
                }),
                "the root image did not pass its reconfirm after the copy: fstat failed with errno 5",
            ),
            (
                Io {
                    op: "fsync",
                    errno: 5,
                },
                "fsync failed with errno 5",
            ),
        ];
        for (fault, part) in rows {
            let shown = fault.to_string();
            assert!(shown.contains(part), "{shown:?} names {part}");
        }
        let refusal = CellDirectoryRefusal {
            path: RecipePath::parse("/a/cell/root.img").expect("canonical"),
            fault: Exists,
        };
        assert!(refusal.to_string().starts_with("/a/cell/root.img: "));
    }

    #[cfg(target_os = "macos")]
    mod macos {
        use super::*;
        use crate::runtime::artifacts::verify_artifact;
        use crate::runtime::recipe::Artifact;
        use crate::runtime::strict_json;
        use crate::runtime::test_fixtures::tree::{ArtifactTree, set_flags};
        use std::io::Seek;
        use std::os::unix::fs::FileExt;

        fn verified(role: ArtifactRole, path: &Path, bytes: &[u8]) -> VerifiedArtifact {
            let text = serde_json::json!({
                "path": path.to_str().expect("test paths are UTF-8"),
                "sha256": Digest::of(bytes).hex(),
            })
            .to_string();
            let value = strict_json::parse_value(text.as_bytes()).expect("the artifact is JSON");
            let artifact =
                Artifact::from_value(&value, "root_image").expect("the artifact is valid");
            verify_artifact(role, &artifact).expect("the artifact verifies")
        }

        struct Bundle {
            tree: ArtifactTree,
            cell: PathBuf,
            image: PathBuf,
        }

        impl Bundle {
            fn new() -> Bundle {
                let tree = ArtifactTree::new();
                tree.dir("base", 0o700);
                let cell = tree.dir("cell", 0o700);
                let image = tree.join("base/rootfs.raw");
                Bundle { tree, cell, image }
            }

            fn root_img(&self) -> PathBuf {
                self.cell.join("root.img")
            }

            fn create(
                &self,
                source: &VerifiedArtifact,
            ) -> Result<WritableRoot, CellDirectoryRefusal> {
                WritableRoot::create(&recipe_path(&self.root_img()), &opened(&self.cell), source)
            }
        }

        fn image_bytes() -> Vec<u8> {
            (0..(2 << 20) + 5).map(|at: u32| (at % 251) as u8).collect()
        }

        #[test]
        fn the_copy_holds_exactly_the_verified_bytes_in_a_new_private_file() {
            let bundle = Bundle::new();
            let bytes = image_bytes();
            bundle.tree.immutable_file("base/rootfs.raw", &bytes);
            let source = verified(ArtifactRole::RootImage, &bundle.image, &bytes);
            let root = bundle.create(&source).expect("the copy is made");
            assert_eq!(root.path().as_path(), bundle.root_img());
            assert_eq!(root.identity(), identity_of(&bundle.root_img()));
            assert_ne!(root.identity(), source.identity());
            let metadata = fs::symlink_metadata(bundle.root_img()).expect("lstat");
            assert!(metadata.is_file());
            assert_eq!(metadata.mode() & 0o7777, 0o600);
            assert_eq!(metadata.nlink(), 1);
            assert_eq!(metadata.uid(), effective_uid());
            assert_eq!(fs::read(bundle.root_img()).expect("read"), bytes);
            let mut file = source.file();
            assert_eq!(file.stream_position().expect("the offset"), 0);
        }

        #[test]
        fn the_copy_reads_the_verified_descriptor_not_the_path() {
            let bundle = Bundle::new();
            let bytes = image_bytes();
            bundle.tree.immutable_file("base/rootfs.raw", &bytes);
            let source = verified(ArtifactRole::RootImage, &bundle.image, &bytes);
            set_flags(&bundle.image, 0);
            fs::rename(&bundle.image, bundle.tree.join("base/rootfs.old")).expect("rename");
            bundle
                .tree
                .immutable_file("base/rootfs.raw", b"another image");
            assert_eq!(
                fault_of(bundle.create(&source)),
                CellDirectoryFault::Source(ArtifactFault::Replaced {
                    verified: source.identity(),
                    found: identity_of(&bundle.image),
                })
            );
            assert!(absent(&bundle.root_img()));
        }

        #[test]
        fn a_root_image_a_held_writer_rewrote_after_verification_is_refused_and_leaves_no_copy() {
            let bundle = Bundle::new();
            let bytes = image_bytes();
            let path = bundle.tree.file("base/rootfs.raw", &bytes, 0o644);
            let writer = fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("a writer opened before the lock");
            set_mode(&path, 0o444);
            set_flags(&path, libc::UF_IMMUTABLE);
            let source = verified(ArtifactRole::RootImage, &path, &bytes);
            writer
                .write_all_at(&bytes[..16], 0)
                .expect("the held writer still writes");
            assert_eq!(
                fault_of(bundle.create(&source)),
                CellDirectoryFault::Source(ArtifactFault::ChangedSinceVerification)
            );
            assert!(absent(&bundle.root_img()));
        }

        #[test]
        fn a_root_image_a_held_writer_extended_is_copied_only_to_one_byte_past_its_size() {
            let bundle = Bundle::new();
            let bytes = image_bytes();
            let path = bundle.tree.file("base/rootfs.raw", &bytes, 0o644);
            let writer = fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("a writer opened before the lock");
            set_mode(&path, 0o444);
            set_flags(&path, libc::UF_IMMUTABLE);
            let source = verified(ArtifactRole::RootImage, &path, &bytes);
            writer
                .write_all_at(&vec![9; 4 << 20], bytes.len() as u64)
                .expect("the held writer extends the image");
            let mut first = bytes.clone();
            first.push(9);
            assert_eq!(
                fault_of(bundle.create(&source)),
                CellDirectoryFault::CopyMismatch {
                    expected: Digest::of(&bytes),
                    actual: Digest::of(&first),
                }
            );
            assert!(absent(&bundle.root_img()));
        }

        #[test]
        fn a_source_other_than_the_root_image_is_refused_as_wrong_source() {
            let bundle = Bundle::new();
            let kernel = bundle.tree.immutable_file("base/kernel", b"kernel");
            let source = verified(ArtifactRole::Kernel, &kernel, b"kernel");
            assert_eq!(
                fault_of(bundle.create(&source)),
                CellDirectoryFault::WrongSource {
                    role: ArtifactRole::Kernel
                }
            );
            assert!(absent(&bundle.root_img()));
        }

        #[test]
        fn create_refuses_an_entry_already_at_the_name_and_leaves_it() {
            let bundle = Bundle::new();
            let bytes = image_bytes();
            bundle.tree.immutable_file("base/rootfs.raw", &bytes);
            let source = verified(ArtifactRole::RootImage, &bundle.image, &bytes);
            fs::write(bundle.root_img(), b"old bytes").expect("write");
            assert_eq!(fault_of(bundle.create(&source)), CellDirectoryFault::Exists);
            assert_eq!(fs::read(bundle.root_img()).expect("read"), b"old bytes");
        }
    }
}
