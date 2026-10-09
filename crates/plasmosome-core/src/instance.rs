use std::ffi::{CStr, CString, OsStr};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use plasmosome_backend::CellId;

use crate::state::{
    CELL_JOURNAL_FILE, CELLS_DIR, CellPathError, cell_ledger_path, validate_cell_id,
};

const LOCK_FILE: &str = "controller.lock";
const PRIVATE_FILE: libc::mode_t = 0o600;
const PRIVATE_DIRECTORY: libc::mode_t = 0o700;
const DIRECTORY_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
const JOURNAL_FLAGS: libc::c_int = libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;

/// An opened instance root directory. The operator supplies and trusts the root, so `open` may
/// reach it through a symlink; every name beneath it is opened relative to this descriptor
/// without following a symlink.
#[derive(Debug)]
pub struct InstanceRoot {
    dir: OwnedFd,
    path: PathBuf,
}

impl InstanceRoot {
    /// Opens the directory at `path`, which must be absolute. A relative path is refused rather
    /// than resolved against the current directory.
    pub fn open(path: &Path) -> Result<InstanceRoot, InstanceRootError> {
        if !path.is_absolute() {
            return Err(InstanceRootError::NotAbsolute {
                path: path.to_path_buf(),
            });
        }
        let dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(path)
            .map_err(|source| InstanceRootError::Open {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(InstanceRoot {
            dir: OwnedFd::from(dir),
            path: path.to_path_buf(),
        })
    }

    /// The path this root was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Takes the instance's one writer lock: a nonblocking exclusive `flock` on
    /// `<root>/controller.lock`, opened without following a symlink and close-on-exec. The file
    /// is created mode 0600 if missing, and the root is synced after creating it. Another holder,
    /// in this process or another, makes this `Busy`; it never waits. The lock lasts until the
    /// returned value is dropped or the process dies, and the file is never unlinked.
    pub fn lock(&self) -> Result<WriterLock, LockError> {
        let path = self.path.join(LOCK_FILE);
        let (fd, created) =
            self.open_lock_file()
                .map_err(|source| match source.raw_os_error() {
                    Some(libc::ELOOP) => LockError::Symlink { path: path.clone() },
                    Some(libc::EISDIR) => LockError::NotRegular { path: path.clone() },
                    _ => LockError::Io {
                        path: path.clone(),
                        source,
                    },
                })?;
        let file = File::from(fd);
        let io_error = |source| LockError::Io {
            path: path.clone(),
            source,
        };
        if !file.metadata().map_err(io_error)?.is_file() {
            return Err(LockError::NotRegular { path });
        }
        if created {
            change_mode(file.as_fd(), PRIVATE_FILE).map_err(io_error)?;
            sync_directory(self.dir.as_fd()).map_err(io_error)?;
        }
        match lock_exclusive_nonblocking(file.as_fd()) {
            Ok(()) => Ok(WriterLock { file, path }),
            Err(source) if source.raw_os_error() == Some(libc::EWOULDBLOCK) => {
                Err(LockError::Busy { path })
            }
            Err(source) => Err(LockError::Io { path, source }),
        }
    }

    /// Lists every entry of `<root>/cells` and classifies it, opening nothing through a symlink.
    /// A missing `cells` is a fresh instance with no entries. A symlinked or non-directory
    /// `cells`, or any failure to list or inspect all of its entries, refuses the whole
    /// discovery: a partial listing is never returned. A bad entry or an unopenable cell is
    /// reported in its own entry while its siblings are still classified. Entries come in
    /// raw-byte order of their names.
    pub fn discover(&self) -> Result<Discovery, DiscoveryError> {
        let cells_path = self.path.join(CELLS_DIR);
        let cells = match self.open_cells() {
            Ok(cells) => cells,
            Err(DirectoryRefusal::Missing(_)) => {
                return Ok(Discovery {
                    cells_dir_present: false,
                    entries: Vec::new(),
                });
            }
            Err(DirectoryRefusal::Symlink) => {
                return Err(DiscoveryError::CellsSymlink { path: cells_path });
            }
            Err(DirectoryRefusal::NotADirectory) => {
                return Err(DiscoveryError::CellsNotADirectory { path: cells_path });
            }
            Err(DirectoryRefusal::Io(source)) => {
                return Err(DiscoveryError::Open {
                    path: cells_path,
                    source,
                });
            }
        };
        let mut names = DirectoryStream::open(cells.as_fd())
            .and_then(collect_names)
            .map_err(|source| DiscoveryError::Listing {
                path: cells_path.clone(),
                source,
            })?;
        names.sort();
        let entries = names
            .into_iter()
            .map(|raw_name| self.classify(cells.as_fd(), &cells_path, raw_name))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Discovery {
            cells_dir_present: true,
            entries,
        })
    }

    /// Opens an existing cell's directory, following no symlink at `cells` or at the cell. An
    /// invalid ID is refused before anything is opened.
    pub fn cell_dir(&self, cell: &CellId) -> Result<CellDir, CellDirError> {
        let journal_path = cell_ledger_path(&self.path, cell).map_err(CellDirError::InvalidCell)?;
        let cells_path = self.path.join(CELLS_DIR);
        let cells = self
            .open_cells()
            .map_err(|refusal| refusal.into_cell_dir_error(cells_path.clone()))?;
        let cell_path = cells_path.join(cell.as_str());
        let name = c_name(cell.as_str().as_bytes()).map_err(|source| CellDirError::Io {
            path: cell_path.clone(),
            source,
        })?;
        let dir = open_at(cells.as_fd(), &name, DIRECTORY_FLAGS, 0).map_err(|error| {
            directory_refusal(cells.as_fd(), &name, error).into_cell_dir_error(cell_path)
        })?;
        Ok(CellDir {
            dir,
            cell: cell.clone(),
            journal_path,
        })
    }

    /// Creates `<root>/cells/<cell>` exclusively with mode 0700, whatever the umask, creating
    /// `cells` the same way when it is missing. Each directory is made from its opened parent,
    /// which is synced afterwards. An existing cell directory is `AlreadyExists` and is never
    /// adopted. An invalid ID is refused before anything is created.
    pub fn create_cell_dir(&self, cell: &CellId) -> Result<CellDir, CellDirError> {
        let journal_path = cell_ledger_path(&self.path, cell).map_err(CellDirError::InvalidCell)?;
        let cells_path = self.path.join(CELLS_DIR);
        let cells = self
            .create_cells()
            .map_err(|refusal| refusal.into_cell_dir_error(cells_path.clone()))?;
        let cell_path = cells_path.join(cell.as_str());
        let io_error = |source| CellDirError::Io {
            path: cell_path.clone(),
            source,
        };
        let name = c_name(cell.as_str().as_bytes()).map_err(io_error)?;
        match make_directory_at(cells.as_fd(), &name) {
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
                return Err(CellDirError::AlreadyExists { path: cell_path });
            }
            made => made.map_err(io_error)?,
        }
        let dir = open_at(cells.as_fd(), &name, DIRECTORY_FLAGS, 0).map_err(|error| {
            directory_refusal(cells.as_fd(), &name, error).into_cell_dir_error(cell_path.clone())
        })?;
        change_mode(dir.as_fd(), PRIVATE_DIRECTORY).map_err(io_error)?;
        sync_directory(cells.as_fd()).map_err(io_error)?;
        Ok(CellDir {
            dir,
            cell: cell.clone(),
            journal_path,
        })
    }

    fn create_cells(&self) -> Result<OwnedFd, DirectoryRefusal> {
        let name = c_name(CELLS_DIR.as_bytes()).map_err(DirectoryRefusal::Io)?;
        let created = match make_directory_at(self.dir.as_fd(), &name) {
            Ok(()) => true,
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => false,
            Err(error) => return Err(DirectoryRefusal::Io(error)),
        };
        let cells = self.open_cells()?;
        if created {
            change_mode(cells.as_fd(), PRIVATE_DIRECTORY).map_err(DirectoryRefusal::Io)?;
            sync_directory(self.dir.as_fd()).map_err(DirectoryRefusal::Io)?;
        }
        Ok(cells)
    }

    fn open_cells(&self) -> Result<OwnedFd, DirectoryRefusal> {
        let name = c_name(CELLS_DIR.as_bytes()).map_err(DirectoryRefusal::Io)?;
        open_at(self.dir.as_fd(), &name, DIRECTORY_FLAGS, 0)
            .map_err(|error| directory_refusal(self.dir.as_fd(), &name, error))
    }

    fn classify(
        &self,
        cells: BorrowedFd<'_>,
        cells_path: &Path,
        raw_name: Vec<u8>,
    ) -> Result<DiscoveredEntry, DiscoveryError> {
        let path = cells_path.join(OsStr::from_bytes(&raw_name));
        let classify_error = |source| DiscoveryError::Classify {
            path: path.clone(),
            source,
        };
        let name = c_name(&raw_name).map_err(classify_error)?;
        let class = match kind_at(cells, &name).map_err(classify_error)? {
            Kind::Symlink => EntryClass::NotACell(NotACell::Symlink),
            Kind::Other => EntryClass::NotACell(NotACell::NotADirectory),
            Kind::Directory => match self.cell_named(&raw_name) {
                None => EntryClass::NotACell(NotACell::InvalidName),
                Some((cell, journal_path)) => {
                    let journal = match open_at(cells, &name, DIRECTORY_FLAGS, 0) {
                        Ok(dir) => CellDir {
                            dir,
                            cell: cell.clone(),
                            journal_path,
                        }
                        .open_journal(),
                        Err(error) => JournalOpen::Refused(JournalRefusal::CellDirectory(error)),
                    };
                    EntryClass::Cell { cell, journal }
                }
            },
        };
        Ok(DiscoveredEntry {
            raw_name,
            path,
            class,
        })
    }

    fn cell_named(&self, raw_name: &[u8]) -> Option<(CellId, PathBuf)> {
        let cell = parse_cell_name(raw_name)?;
        let journal_path = cell_ledger_path(&self.path, &cell).ok()?;
        Some((cell, journal_path))
    }

    fn open_lock_file(&self) -> io::Result<(OwnedFd, bool)> {
        let name = c_name(LOCK_FILE.as_bytes())?;
        let flags = libc::O_RDWR | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
        match open_at(self.dir.as_fd(), &name, flags, 0) {
            Err(missing) if missing.raw_os_error() == Some(libc::ENOENT) => {
                let exclusive = flags | libc::O_CREAT | libc::O_EXCL;
                match open_at(self.dir.as_fd(), &name, exclusive, PRIVATE_FILE) {
                    Ok(fd) => Ok((fd, true)),
                    Err(raced) if raced.raw_os_error() == Some(libc::EEXIST) => {
                        open_at(self.dir.as_fd(), &name, flags, 0).map(|fd| (fd, false))
                    }
                    Err(error) => Err(error),
                }
            }
            opened => opened.map(|fd| (fd, false)),
        }
    }
}

/// The instance's writer lock. Dropping it releases the lock and leaves `controller.lock` in
/// place; so does the death of the process holding it.
pub struct WriterLock {
    file: File,
    path: PathBuf,
}

impl WriterLock {
    /// The lock file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Debug for WriterLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriterLock")
            .field("fd", &self.file.as_raw_fd())
            .field("path", &self.path)
            .finish()
    }
}

/// One opened cell directory under `<root>/cells`, reached without following a symlink. Its
/// journal is opened relative to it, never by path.
#[derive(Debug)]
pub struct CellDir {
    dir: OwnedFd,
    cell: CellId,
    journal_path: PathBuf,
}

impl CellDir {
    /// The cell this directory belongs to.
    pub fn cell(&self) -> &CellId {
        &self.cell
    }

    /// The journal's path, as [`cell_ledger_path`] names it, for messages and reports.
    pub fn journal_path(&self) -> &Path {
        &self.journal_path
    }

    /// Opens the journal for reading without following a symlink and without blocking on a
    /// FIFO. A missing journal is `Missing`; anything but a regular file is refused and closed
    /// unread. The returned file is in blocking mode.
    pub fn open_journal(&self) -> JournalOpen {
        match open_journal_at(self.dir.as_fd(), libc::O_RDONLY, 0) {
            Ok(file) => JournalOpen::Regular(file),
            Err(JournalRefusal::Io(error)) if error.raw_os_error() == Some(libc::ENOENT) => {
                JournalOpen::Missing
            }
            Err(refusal) => JournalOpen::Refused(refusal),
        }
    }

    /// Opens the journal for reading and appending, without following a symlink, blocking on a
    /// FIFO or truncating. An existing regular journal returns `created: false`. A missing one
    /// is created exclusively with mode 0600 and returns `created: true`; this call does not
    /// sync it or its directory. An existing inode is never replaced.
    pub fn open_journal_for_append(&self) -> Result<JournalAppend, JournalRefusal> {
        let access = libc::O_RDWR | libc::O_APPEND;
        match open_journal_at(self.dir.as_fd(), access, 0) {
            Ok(file) => Ok(JournalAppend {
                file,
                created: false,
            }),
            Err(JournalRefusal::Io(error)) if error.raw_os_error() == Some(libc::ENOENT) => {
                let exclusive = access | libc::O_CREAT | libc::O_EXCL;
                let file = open_journal_at(self.dir.as_fd(), exclusive, PRIVATE_FILE)?;
                change_mode(file.as_fd(), PRIVATE_FILE).map_err(JournalRefusal::Io)?;
                Ok(JournalAppend {
                    file,
                    created: true,
                })
            }
            Err(refusal) => Err(refusal),
        }
    }

    /// Syncs this cell directory, making entries created in it durable.
    pub fn sync(&self) -> io::Result<()> {
        sync_directory(self.dir.as_fd())
    }
}

/// A journal opened for reading and appending through one descriptor. `created` is true when
/// this open created the file: its directory entry is not yet durable until the caller syncs
/// the file and then the cell directory with [`CellDir::sync`].
#[derive(Debug)]
pub struct JournalAppend {
    pub file: File,
    pub created: bool,
}

/// The result of opening a cell's journal for reading.
#[derive(Debug)]
pub enum JournalOpen {
    Missing,
    Regular(File),
    Refused(JournalRefusal),
}

/// Why a cell's journal was not opened. `CellDirectory` means the cell directory itself could
/// not be opened.
#[derive(Debug)]
pub enum JournalRefusal {
    CellDirectory(io::Error),
    Symlink,
    NotRegular,
    Io(io::Error),
}

/// Every entry found under `<root>/cells`, in raw-byte order of the names.
/// `cells_dir_present` is false for a fresh instance with no `cells` directory.
#[derive(Debug)]
pub struct Discovery {
    pub cells_dir_present: bool,
    pub entries: Vec<DiscoveredEntry>,
}

/// One entry of `<root>/cells`. `raw_name` is the name's exact bytes and `path` is built from
/// them, never from a lossy string.
#[derive(Debug)]
pub struct DiscoveredEntry {
    pub raw_name: Vec<u8>,
    pub path: PathBuf,
    pub class: EntryClass,
}

/// A discovered entry is a valid cell with its journal opened, or not a cell at all.
#[derive(Debug)]
pub enum EntryClass {
    Cell { cell: CellId, journal: JournalOpen },
    NotACell(NotACell),
}

/// Why an entry is not a cell. Nothing is opened through such an entry. A symlink is reported
/// as a symlink even when its name is also invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotACell {
    InvalidName,
    Symlink,
    NotADirectory,
}

/// Why discovery could not account for every entry under `<root>/cells`.
#[derive(Debug)]
pub enum DiscoveryError {
    CellsSymlink { path: PathBuf },
    CellsNotADirectory { path: PathBuf },
    Open { path: PathBuf, source: io::Error },
    Listing { path: PathBuf, source: io::Error },
    Classify { path: PathBuf, source: io::Error },
}

/// Why a cell directory could not be opened or created.
#[derive(Debug)]
pub enum CellDirError {
    InvalidCell(CellPathError),
    Symlink { path: PathBuf },
    NotADirectory { path: PathBuf },
    AlreadyExists { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
}

/// Why an instance root could not be opened.
#[derive(Debug)]
pub enum InstanceRootError {
    NotAbsolute { path: PathBuf },
    Open { path: PathBuf, source: io::Error },
}

/// Why the writer lock was not taken. `Busy` means another open of the lock file holds it.
#[derive(Debug)]
pub enum LockError {
    Busy { path: PathBuf },
    Symlink { path: PathBuf },
    NotRegular { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
}

fn c_name(name: &[u8]) -> io::Result<CString> {
    CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

fn open_at(
    dir: BorrowedFd<'_>,
    name: &CStr,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> io::Result<OwnedFd> {
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, mode as libc::c_uint) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn parse_cell_name(raw: &[u8]) -> Option<CellId> {
    let cell = CellId::from(std::str::from_utf8(raw).ok()?);
    validate_cell_id(&cell).ok()?;
    Some(cell)
}

fn collect_names(names: impl Iterator<Item = io::Result<Vec<u8>>>) -> io::Result<Vec<Vec<u8>>> {
    names.collect()
}

enum DirectoryRefusal {
    Missing(io::Error),
    Symlink,
    NotADirectory,
    Io(io::Error),
}

impl DirectoryRefusal {
    fn into_cell_dir_error(self, path: PathBuf) -> CellDirError {
        match self {
            DirectoryRefusal::Symlink => CellDirError::Symlink { path },
            DirectoryRefusal::NotADirectory => CellDirError::NotADirectory { path },
            DirectoryRefusal::Missing(source) | DirectoryRefusal::Io(source) => {
                CellDirError::Io { path, source }
            }
        }
    }
}

fn directory_refusal(parent: BorrowedFd<'_>, name: &CStr, error: io::Error) -> DirectoryRefusal {
    match error.raw_os_error() {
        Some(libc::ENOENT) => DirectoryRefusal::Missing(error),
        Some(libc::ELOOP) => DirectoryRefusal::Symlink,
        Some(libc::ENOTDIR) => match kind_at(parent, name) {
            Ok(Kind::Symlink) => DirectoryRefusal::Symlink,
            _ => DirectoryRefusal::NotADirectory,
        },
        _ => DirectoryRefusal::Io(error),
    }
}

fn open_journal_at(
    cell: BorrowedFd<'_>,
    access: libc::c_int,
    mode: libc::mode_t,
) -> Result<File, JournalRefusal> {
    let name = c_name(CELL_JOURNAL_FILE.as_bytes()).map_err(JournalRefusal::Io)?;
    let fd = open_at(cell, &name, access | JOURNAL_FLAGS, mode).map_err(|error| {
        match error.raw_os_error() {
            Some(libc::ELOOP) => JournalRefusal::Symlink,
            Some(libc::EISDIR) => JournalRefusal::NotRegular,
            _ => JournalRefusal::Io(error),
        }
    })?;
    let file = File::from(fd);
    if !file.metadata().map_err(JournalRefusal::Io)?.is_file() {
        return Err(JournalRefusal::NotRegular);
    }
    clear_nonblocking(file.as_fd()).map_err(JournalRefusal::Io)?;
    Ok(file)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Directory,
    Symlink,
    Other,
}

fn kind_at(dir: BorrowedFd<'_>, name: &CStr) -> io::Result<Kind> {
    let mut status = MaybeUninit::<libc::stat>::uninit();
    let found = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            status.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if found != 0 {
        return Err(io::Error::last_os_error());
    }
    let format = unsafe { status.assume_init() }.st_mode & libc::S_IFMT;
    Ok(match format {
        libc::S_IFDIR => Kind::Directory,
        libc::S_IFLNK => Kind::Symlink,
        _ => Kind::Other,
    })
}

fn clear_nonblocking(file: BorrowedFd<'_>) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let set = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) };
    if set < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct DirectoryStream(NonNull<libc::DIR>);

impl DirectoryStream {
    fn open(dir: BorrowedFd<'_>) -> io::Result<DirectoryStream> {
        let owned = dir.try_clone_to_owned()?;
        let stream = unsafe { libc::fdopendir(owned.as_raw_fd()) };
        let stream = NonNull::new(stream).ok_or_else(io::Error::last_os_error)?;
        let _owned_by_stream = owned.into_raw_fd();
        Ok(DirectoryStream(stream))
    }
}

impl Iterator for DirectoryStream {
    type Item = io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            clear_errno();
            let entry = unsafe { libc::readdir(self.0.as_ptr()) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                return (error.raw_os_error() != Some(0)).then_some(Err(error));
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name != b"." && name != b".." {
                return Some(Ok(name.to_vec()));
            }
        }
    }
}

impl Drop for DirectoryStream {
    fn drop(&mut self) {
        unsafe { libc::closedir(self.0.as_ptr()) };
    }
}

#[cfg(target_os = "macos")]
fn clear_errno() {
    unsafe { *libc::__error() = 0 };
}

#[cfg(target_os = "linux")]
fn clear_errno() {
    unsafe { *libc::__errno_location() = 0 };
}

fn make_directory_at(parent: BorrowedFd<'_>, name: &CStr) -> io::Result<()> {
    let made = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), PRIVATE_DIRECTORY) };
    if made != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn change_mode(fd: BorrowedFd<'_>, mode: libc::mode_t) -> io::Result<()> {
    let changed = unsafe { libc::fchmod(fd.as_raw_fd(), mode) };
    if changed != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn lock_exclusive_nonblocking(file: BorrowedFd<'_>) -> io::Result<()> {
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if locked != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn sync_directory(dir: BorrowedFd<'_>) -> io::Result<()> {
    File::from(dir.try_clone_to_owned()?).sync_all()
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::fs;
    use std::io::{self, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::thread;
    use std::time::{Duration, SystemTime};

    use tempfile::TempDir;

    use super::*;

    const DEADLINE: Duration = Duration::from_secs(5);

    fn temp_root() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir
            .path()
            .canonicalize()
            .expect("a canonical temporary path");
        (dir, root)
    }

    fn open_root(root: &Path) -> InstanceRoot {
        InstanceRoot::open(root).expect("the instance root opens")
    }

    fn within<T: Send + 'static>(what: &str, work: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(work());
        });
        match receiver.recv_timeout(DEADLINE) {
            Ok(value) => value,
            Err(RecvTimeoutError::Timeout) => panic!("{what} did not return within {DEADLINE:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("{what} panicked"),
        }
    }

    fn a_separate_open_can_lock(path: &Path) -> bool {
        let file = fs::File::open(path).expect("the file opens for a lock attempt");
        let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        taken == 0
    }

    fn make_fifo(path: &Path) {
        let name = CString::new(path.as_os_str().as_bytes()).expect("a path without NUL");
        let made = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
        assert_eq!(
            made,
            0,
            "mkfifo {}: {}",
            path.display(),
            io::Error::last_os_error()
        );
    }

    fn make_cell(root: &Path, name: &str, journal: Option<&[u8]>) -> PathBuf {
        let cell = root.join("cells").join(name);
        fs::create_dir_all(&cell).expect("the cell directory is made");
        if let Some(bytes) = journal {
            fs::write(cell.join("ledger.ndjson"), bytes).expect("the journal is written");
        }
        cell
    }

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("the mode is set");
    }

    fn assert_not_root() {
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "this test needs a user the kernel refuses for mode 000; root bypasses it"
        );
    }

    fn describe(class: &EntryClass) -> String {
        match class {
            EntryClass::Cell { cell, journal } => {
                format!("cell {cell}: {}", describe_journal(journal))
            }
            EntryClass::NotACell(reason) => format!("not a cell: {reason:?}"),
        }
    }

    fn describe_journal(journal: &JournalOpen) -> &'static str {
        match journal {
            JournalOpen::Missing => "missing",
            JournalOpen::Regular(_) => "regular",
            JournalOpen::Refused(JournalRefusal::CellDirectory(_)) => "refused: cell directory",
            JournalOpen::Refused(JournalRefusal::Symlink) => "refused: symlink",
            JournalOpen::Refused(JournalRefusal::NotRegular) => "refused: not regular",
            JournalOpen::Refused(JournalRefusal::Io(_)) => "refused: io",
        }
    }

    fn discover(root: &Path) -> Discovery {
        open_root(root).discover().expect("discovery succeeds")
    }

    fn described(discovery: &Discovery) -> Vec<String> {
        discovery
            .entries
            .iter()
            .map(|entry| describe(&entry.class))
            .collect()
    }

    #[test]
    fn a_relative_root_is_refused() {
        match InstanceRoot::open(Path::new("relative/root")) {
            Err(InstanceRootError::NotAbsolute { path }) => {
                assert_eq!(path, PathBuf::from("relative/root"))
            }
            other => panic!("expected NotAbsolute, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_root_is_an_open_error_naming_it() {
        let (_dir, root) = temp_root();
        let missing = root.join("missing");
        match InstanceRoot::open(&missing) {
            Err(InstanceRootError::Open { path, source }) => {
                assert_eq!(path, missing);
                assert_eq!(source.kind(), io::ErrorKind::NotFound);
            }
            other => panic!("expected Open, got {other:?}"),
        }
    }

    #[test]
    fn lock_creates_a_private_regular_file_and_holds_it() {
        let (_dir, root) = temp_root();
        let instance = open_root(&root);
        assert_eq!(instance.path(), root);
        let lock = instance.lock().expect("the first lock is taken");
        let path = root.join("controller.lock");
        assert_eq!(lock.path(), path);
        let metadata = fs::symlink_metadata(&path).expect("the lock file exists");
        assert!(metadata.file_type().is_file(), "{metadata:?}");
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert!(
            !a_separate_open_can_lock(&path),
            "the lock is held while the WriterLock lives"
        );
        assert!(format!("{lock:?}").contains("controller.lock"), "{lock:?}");
    }

    #[test]
    fn the_root_and_lock_descriptors_are_close_on_exec() {
        let (_dir, root) = temp_root();
        let instance = open_root(&root);
        let lock = instance.lock().expect("the first lock is taken");
        for (what, fd) in [
            ("root", instance.dir.as_raw_fd()),
            ("lock", lock.file.as_raw_fd()),
        ] {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            assert!(
                flags >= 0 && flags & libc::FD_CLOEXEC != 0,
                "the {what} descriptor would leak into a spawned process: flags {flags}"
            );
        }
    }

    #[test]
    fn a_second_lock_is_busy_while_the_first_is_held() {
        let (_dir, root) = temp_root();
        let _held = open_root(&root).lock().expect("the first lock is taken");
        let second = open_root(&root);
        match within("a contended lock", move || second.lock()) {
            Err(LockError::Busy { path }) => assert_eq!(path, root.join("controller.lock")),
            other => panic!("expected Busy, got {other:?}"),
        }
    }

    #[test]
    fn dropping_the_lock_releases_it_and_keeps_the_file() {
        let (_dir, root) = temp_root();
        let instance = open_root(&root);
        let path = root.join("controller.lock");
        drop(instance.lock().expect("the first lock is taken"));
        let inode = fs::symlink_metadata(&path)
            .expect("the lock file outlives its lock")
            .ino();
        assert!(
            a_separate_open_can_lock(&path),
            "dropping the WriterLock releases the lock"
        );
        let again = instance.lock().expect("the lock is free again");
        assert_eq!(again.path(), path);
        assert_eq!(
            fs::symlink_metadata(&path).expect("the lock file").ino(),
            inode,
            "the lock file is never replaced"
        );
    }

    #[test]
    fn a_symlinked_lock_file_is_refused_without_touching_the_target() {
        let (_dir, root) = temp_root();
        let target = root.join("elsewhere");
        fs::write(&target, b"keep").expect("the target is written");
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        fs::File::options()
            .write(true)
            .open(&target)
            .and_then(|file| file.set_modified(old))
            .expect("the target's mtime is set");
        symlink(&target, root.join("controller.lock")).expect("the link is made");
        match open_root(&root).lock() {
            Err(LockError::Symlink { path }) => assert_eq!(path, root.join("controller.lock")),
            other => panic!("expected Symlink, got {other:?}"),
        }
        assert_eq!(fs::read(&target).expect("the target reads"), b"keep");
        assert_eq!(
            fs::metadata(&target)
                .and_then(|metadata| metadata.modified())
                .expect("the target's mtime"),
            old
        );
        assert!(
            a_separate_open_can_lock(&target),
            "no lock was taken through the link"
        );
    }

    #[test]
    fn a_dangling_symlinked_lock_file_is_refused_and_creates_nothing() {
        let (_dir, root) = temp_root();
        let target = root.join("never-created");
        symlink(&target, root.join("controller.lock")).expect("the link is made");
        match open_root(&root).lock() {
            Err(LockError::Symlink { path }) => assert_eq!(path, root.join("controller.lock")),
            other => panic!("expected Symlink, got {other:?}"),
        }
        assert!(
            fs::symlink_metadata(&target).is_err(),
            "the link target was created"
        );
    }

    #[test]
    fn a_directory_at_the_lock_path_is_refused() {
        let (_dir, root) = temp_root();
        fs::create_dir(root.join("controller.lock")).expect("the directory is made");
        match open_root(&root).lock() {
            Err(LockError::NotRegular { path }) => assert_eq!(path, root.join("controller.lock")),
            other => panic!("expected NotRegular, got {other:?}"),
        }
    }

    #[test]
    fn a_fifo_at_the_lock_path_is_refused_as_not_regular() {
        let (_dir, root) = temp_root();
        make_fifo(&root.join("controller.lock"));
        let instance = open_root(&root);
        match within("locking a FIFO", move || instance.lock()) {
            Err(LockError::NotRegular { path }) => assert_eq!(path, root.join("controller.lock")),
            other => panic!("expected NotRegular, got {other:?}"),
        }
    }

    #[test]
    fn a_socket_at_the_lock_path_is_refused_as_not_regular() {
        let (_dir, root) = temp_root();
        let _socket = UnixListener::bind(root.join("controller.lock")).expect("the socket binds");
        match open_root(&root).lock() {
            Err(LockError::NotRegular { path }) => assert_eq!(path, root.join("controller.lock")),
            other => panic!("expected NotRegular, got {other:?}"),
        }
    }

    #[test]
    fn a_lock_file_that_cannot_be_created_is_an_io_error() {
        assert_not_root();
        let (_dir, root) = temp_root();
        set_mode(&root, 0o500);
        let result = open_root(&root).lock();
        set_mode(&root, 0o700);
        match result {
            Err(LockError::Io { path, source }) => {
                assert_eq!(path, root.join("controller.lock"));
                assert_eq!(source.raw_os_error(), Some(libc::EACCES));
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_cells_directory_is_a_fresh_instance() {
        let (_dir, root) = temp_root();
        let discovery = discover(&root);
        assert!(!discovery.cells_dir_present);
        assert!(discovery.entries.is_empty(), "{discovery:?}");
    }

    #[test]
    fn an_empty_cells_directory_lists_no_dot_entries() {
        let (_dir, root) = temp_root();
        fs::create_dir(root.join("cells")).expect("cells is made");
        let discovery = discover(&root);
        assert!(discovery.cells_dir_present);
        assert!(discovery.entries.is_empty(), "{discovery:?}");
    }

    #[test]
    fn a_symlinked_cells_directory_aborts_discovery() {
        let (_dir, root) = temp_root();
        make_cell(&root.join("elsewhere"), "cell-1", Some(b"{}\n"));
        symlink(root.join("elsewhere/cells"), root.join("cells")).expect("the link is made");
        match open_root(&root).discover() {
            Err(DiscoveryError::CellsSymlink { path }) => assert_eq!(path, root.join("cells")),
            other => panic!("expected CellsSymlink, got {other:?}"),
        }
    }

    #[test]
    fn a_regular_file_at_cells_aborts_discovery() {
        let (_dir, root) = temp_root();
        fs::write(root.join("cells"), b"").expect("the file is made");
        match open_root(&root).discover() {
            Err(DiscoveryError::CellsNotADirectory { path }) => {
                assert_eq!(path, root.join("cells"))
            }
            other => panic!("expected CellsNotADirectory, got {other:?}"),
        }
    }

    #[test]
    fn an_unreadable_cells_directory_aborts_discovery() {
        assert_not_root();
        let (_dir, root) = temp_root();
        make_cell(&root, "cell-1", None);
        set_mode(&root.join("cells"), 0o000);
        let result = open_root(&root).discover();
        set_mode(&root.join("cells"), 0o700);
        match result {
            Err(DiscoveryError::Open { path, source }) => {
                assert_eq!(path, root.join("cells"));
                assert_eq!(source.raw_os_error(), Some(libc::EACCES));
            }
            other => panic!("expected Open, got {other:?}"),
        }
    }

    #[test]
    fn a_listing_error_after_some_entries_aborts() {
        let names = vec![
            Ok(b"a".to_vec()),
            Ok(b"b".to_vec()),
            Err(io::Error::from_raw_os_error(libc::EIO)),
        ];
        let error = collect_names(names.into_iter()).expect_err("a partial listing is refused");
        assert_eq!(error.raw_os_error(), Some(libc::EIO));
        assert_eq!(
            collect_names(vec![Ok(b"b".to_vec()), Ok(b"a".to_vec())].into_iter())
                .expect("a complete listing"),
            vec![b"b".to_vec(), b"a".to_vec()]
        );
    }

    #[test]
    fn entries_are_classified_and_good_siblings_survive() {
        assert_not_root();
        let (_dir, root) = temp_root();
        make_cell(&root, "a-journal", Some(b"{}\n"));
        make_cell(&root, "b-empty", None);
        let locked = root.join("locked");
        fs::create_dir(&locked).expect("the link target is made");
        set_mode(&locked, 0o000);
        symlink(&locked, root.join("cells/c-link")).expect("the link is made");
        fs::write(root.join("cells/d-file"), b"not a cell").expect("the file is made");
        fs::create_dir(root.join("cells/e\\bad")).expect("the bad name is made");

        let mut discovery = discover(&root);
        set_mode(&locked, 0o700);

        assert!(discovery.cells_dir_present);
        assert_eq!(
            described(&discovery),
            [
                "cell a-journal: regular",
                "cell b-empty: missing",
                "not a cell: Symlink",
                "not a cell: NotADirectory",
                "not a cell: InvalidName",
            ]
        );
        let names = ["a-journal", "b-empty", "c-link", "d-file", "e\\bad"];
        for (entry, name) in discovery.entries.iter().zip(names) {
            assert_eq!(entry.raw_name, name.as_bytes());
            assert_eq!(entry.path, root.join("cells").join(name));
        }
        let EntryClass::Cell {
            journal: JournalOpen::Regular(file),
            ..
        } = &mut discovery.entries[0].class
        else {
            panic!("the first cell's journal opens");
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).expect("the journal reads");
        assert_eq!(bytes, b"{}\n");
    }

    #[test]
    fn a_symlink_with_an_invalid_name_is_reported_as_a_symlink() {
        let (_dir, root) = temp_root();
        make_cell(&root, "good", None);
        symlink(root.join("cells/good"), root.join("cells/bad\\link")).expect("the link is made");
        fs::write(root.join("cells/bad\\file"), b"").expect("the file is made");
        assert_eq!(
            described(&discover(&root)),
            [
                "not a cell: NotADirectory",
                "not a cell: Symlink",
                "cell good: missing",
            ]
        );
    }

    #[test]
    fn parse_cell_name_refuses_non_utf8_bytes_without_loss() {
        assert_eq!(parse_cell_name(&[0xff, b'a']), None);
        assert_eq!(parse_cell_name(b".."), None);
        assert_eq!(parse_cell_name(b"a\\b"), None);
        assert_eq!(parse_cell_name(b"cell-1"), Some(CellId::from("cell-1")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_non_utf8_entry_keeps_its_raw_name_and_path() {
        use std::ffi::OsStr;

        let (_dir, root) = temp_root();
        let raw = [0xff, b'a'];
        let path = root.join("cells").join(OsStr::from_bytes(&raw));
        fs::create_dir_all(&path).expect("the non-UTF-8 entry is made");
        let discovery = discover(&root);
        assert_eq!(described(&discovery), ["not a cell: InvalidName"]);
        assert_eq!(discovery.entries[0].raw_name, raw);
        assert_eq!(discovery.entries[0].path, path);
        assert!(
            discovery.entries[0]
                .path
                .as_os_str()
                .as_bytes()
                .ends_with(&raw)
        );
    }

    #[test]
    fn a_symlinked_journal_is_refused_and_its_target_is_not_opened() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let cell = make_cell(&root, "cell-1", None);
        let target = root.join("secret");
        fs::write(&target, b"secret").expect("the target is made");
        set_mode(&target, 0o000);
        symlink(&target, cell.join("ledger.ndjson")).expect("the link is made");
        assert_eq!(
            described(&discover(&root)),
            ["cell cell-1: refused: symlink"]
        );
    }

    #[test]
    fn a_fifo_journal_is_refused_without_blocking() {
        let (_dir, root) = temp_root();
        let cell = make_cell(&root, "cell-1", None);
        make_fifo(&cell.join("ledger.ndjson"));
        let instance = open_root(&root);
        let discovery = within("discovering a FIFO journal", move || instance.discover())
            .expect("discovery succeeds");
        assert_eq!(described(&discovery), ["cell cell-1: refused: not regular"]);
    }

    #[test]
    fn a_directory_journal_is_refused_as_not_regular() {
        let (_dir, root) = temp_root();
        let cell = make_cell(&root, "cell-1", None);
        fs::create_dir(cell.join("ledger.ndjson")).expect("the directory is made");
        assert_eq!(
            described(&discover(&root)),
            ["cell cell-1: refused: not regular"]
        );
    }

    #[test]
    fn a_socket_journal_is_refused_as_not_regular() {
        let (_dir, root) = temp_root();
        let cell = make_cell(&root, "cell-1", None);
        let _socket = UnixListener::bind(cell.join("ledger.ndjson")).expect("the socket binds");
        assert_eq!(
            described(&discover(&root)),
            ["cell cell-1: refused: not regular"]
        );
    }

    #[test]
    fn an_unreadable_cell_directory_refuses_only_that_cell() {
        assert_not_root();
        let (_dir, root) = temp_root();
        make_cell(&root, "cell-a", Some(b"{}\n"));
        let locked = make_cell(&root, "cell-b", None);
        set_mode(&locked, 0o000);
        let discovery = discover(&root);
        set_mode(&locked, 0o700);
        assert_eq!(
            described(&discovery),
            [
                "cell cell-a: regular",
                "cell cell-b: refused: cell directory"
            ]
        );
    }

    #[test]
    fn a_discovered_journal_is_left_blocking() {
        let (_dir, root) = temp_root();
        make_cell(&root, "cell-1", Some(b"{}\n"));
        let discovery = discover(&root);
        let EntryClass::Cell {
            journal: JournalOpen::Regular(file),
            ..
        } = &discovery.entries[0].class
        else {
            panic!("the journal opens: {discovery:?}");
        };
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0, "F_GETFL: {}", io::Error::last_os_error());
        assert_eq!(flags & libc::O_NONBLOCK, 0, "O_NONBLOCK was left set");
    }

    fn cell(name: &str) -> CellId {
        CellId::from(name)
    }

    fn mode_of(path: &Path) -> u32 {
        fs::symlink_metadata(path).expect("the entry exists").mode() & 0o7777
    }

    fn identity(metadata: &fs::Metadata) -> (u64, u64) {
        (metadata.dev(), metadata.ino())
    }

    fn appended(dir: &CellDir) -> JournalAppend {
        dir.open_journal_for_append()
            .expect("the journal opens for appending")
    }

    #[test]
    fn create_cell_dir_creates_private_directories() {
        let (_dir, root) = temp_root();
        let created = open_root(&root)
            .create_cell_dir(&cell("cell-1"))
            .expect("the cell directory is created");
        assert_eq!(created.cell(), &cell("cell-1"));
        assert_eq!(
            created.journal_path(),
            root.join("cells/cell-1/ledger.ndjson")
        );
        for path in [root.join("cells"), root.join("cells/cell-1")] {
            assert!(
                fs::symlink_metadata(&path)
                    .expect("the directory exists")
                    .is_dir()
            );
            assert_eq!(mode_of(&path), 0o700, "{}", path.display());
        }
        created.sync().expect("the cell directory syncs");
    }

    #[test]
    fn create_cell_dir_refuses_an_existing_cell() {
        let (_dir, root) = temp_root();
        let existing = make_cell(&root, "cell-1", None);
        set_mode(&existing, 0o755);
        match open_root(&root).create_cell_dir(&cell("cell-1")) {
            Err(CellDirError::AlreadyExists { path }) => assert_eq!(path, existing),
            other => panic!("expected AlreadyExists, got {other:?}"),
        }
        assert_eq!(mode_of(&existing), 0o755, "an existing cell is not adopted");
    }

    #[test]
    fn create_cell_dir_refuses_a_symlinked_cells_directory() {
        let (_dir, root) = temp_root();
        let elsewhere = root.join("elsewhere");
        fs::create_dir(&elsewhere).expect("the link target is made");
        symlink(&elsewhere, root.join("cells")).expect("the link is made");
        match open_root(&root).create_cell_dir(&cell("cell-1")) {
            Err(CellDirError::Symlink { path }) => assert_eq!(path, root.join("cells")),
            other => panic!("expected Symlink, got {other:?}"),
        }
        assert_eq!(
            fs::read_dir(&elsewhere).expect("the target lists").count(),
            0,
            "nothing was created through the link"
        );
    }

    #[test]
    fn create_cell_dir_refuses_a_regular_file_at_cells() {
        let (_dir, root) = temp_root();
        fs::write(root.join("cells"), b"").expect("the file is made");
        match open_root(&root).create_cell_dir(&cell("cell-1")) {
            Err(CellDirError::NotADirectory { path }) => assert_eq!(path, root.join("cells")),
            other => panic!("expected NotADirectory, got {other:?}"),
        }
    }

    #[test]
    fn create_cell_dir_reports_a_parent_it_cannot_write() {
        assert_not_root();
        let (_dir, root) = temp_root();
        set_mode(&root, 0o500);
        let without_cells = open_root(&root).create_cell_dir(&cell("cell-1"));
        set_mode(&root, 0o700);
        match without_cells {
            Err(CellDirError::Io { path, source }) => {
                assert_eq!(path, root.join("cells"));
                assert_eq!(source.raw_os_error(), Some(libc::EACCES));
            }
            other => panic!("expected Io for cells, got {other:?}"),
        }

        fs::create_dir(root.join("cells")).expect("cells is made");
        set_mode(&root.join("cells"), 0o500);
        let without_cell = open_root(&root).create_cell_dir(&cell("cell-1"));
        set_mode(&root.join("cells"), 0o700);
        match without_cell {
            Err(CellDirError::Io { path, source }) => {
                assert_eq!(path, root.join("cells/cell-1"));
                assert_eq!(source.raw_os_error(), Some(libc::EACCES));
            }
            other => panic!("expected Io for the cell, got {other:?}"),
        }
    }

    #[test]
    fn create_cell_dir_refuses_invalid_ids_before_creating_anything() {
        let (_dir, root) = temp_root();
        match open_root(&root).create_cell_dir(&cell("..")) {
            Err(CellDirError::InvalidCell(CellPathError::NotACellName(text))) => {
                assert_eq!(text, "..")
            }
            other => panic!("expected InvalidCell, got {other:?}"),
        }
        assert!(
            fs::symlink_metadata(root.join("cells")).is_err(),
            "cells was created for an invalid ID"
        );
    }

    #[test]
    fn cell_dir_opens_an_existing_cell_and_its_journal() {
        let (_dir, root) = temp_root();
        make_cell(&root, "cell-1", Some(b"{}\n"));
        let opened = open_root(&root)
            .cell_dir(&cell("cell-1"))
            .expect("the cell opens");
        assert_eq!(opened.cell(), &cell("cell-1"));
        assert_eq!(
            opened.journal_path(),
            cell_ledger_path(&root, &cell("cell-1")).expect("a valid cell")
        );
        assert_eq!(describe_journal(&opened.open_journal()), "regular");
    }

    #[test]
    fn cell_dir_refuses_invalid_ids_before_any_open() {
        let (_dir, root) = temp_root();
        fs::create_dir(root.join("cells")).expect("cells is made");
        match open_root(&root).cell_dir(&cell("..")) {
            Err(CellDirError::InvalidCell(CellPathError::NotACellName(text))) => {
                assert_eq!(text, "..")
            }
            other => panic!("expected InvalidCell, got {other:?}"),
        }
    }

    #[test]
    fn cell_dir_refuses_symlinks_and_non_directories() {
        let (_dir, root) = temp_root();
        make_cell(&root, "real", None);
        symlink(root.join("cells/real"), root.join("cells/linked")).expect("the link is made");
        fs::write(root.join("cells/file"), b"").expect("the file is made");
        let instance = open_root(&root);
        match instance.cell_dir(&cell("linked")) {
            Err(CellDirError::Symlink { path }) => assert_eq!(path, root.join("cells/linked")),
            other => panic!("expected Symlink, got {other:?}"),
        }
        match instance.cell_dir(&cell("file")) {
            Err(CellDirError::NotADirectory { path }) => {
                assert_eq!(path, root.join("cells/file"))
            }
            other => panic!("expected NotADirectory, got {other:?}"),
        }
        match instance.cell_dir(&cell("missing")) {
            Err(CellDirError::Io { path, source }) => {
                assert_eq!(path, root.join("cells/missing"));
                assert_eq!(source.kind(), io::ErrorKind::NotFound);
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn cell_dir_refuses_a_symlinked_cells_directory() {
        let (_dir, root) = temp_root();
        make_cell(&root.join("elsewhere"), "cell-1", None);
        symlink(root.join("elsewhere/cells"), root.join("cells")).expect("the link is made");
        match open_root(&root).cell_dir(&cell("cell-1")) {
            Err(CellDirError::Symlink { path }) => assert_eq!(path, root.join("cells")),
            other => panic!("expected Symlink, got {other:?}"),
        }
    }

    #[test]
    fn open_journal_for_append_creates_once_and_never_truncates() {
        let (_dir, root) = temp_root();
        let instance = open_root(&root);
        let created = instance
            .create_cell_dir(&cell("cell-1"))
            .expect("the cell directory is created");
        let journal = root.join("cells/cell-1/ledger.ndjson");

        let mut first = appended(&created);
        assert!(first.created);
        assert_eq!(mode_of(&journal), 0o600);
        first.file.write_all(b"one\n").expect("the first record");
        let before = identity(&fs::symlink_metadata(&journal).expect("the journal"));

        let mut second = appended(&instance.cell_dir(&cell("cell-1")).expect("the cell"));
        assert!(!second.created);
        let mut existing = Vec::new();
        second
            .file
            .read_to_end(&mut existing)
            .expect("the existing records read through the append handle");
        assert_eq!(existing, b"one\n");
        second.file.write_all(b"two\n").expect("the second record");
        assert_eq!(fs::read(&journal).expect("the journal"), b"one\ntwo\n");
        assert_eq!(
            identity(&fs::symlink_metadata(&journal).expect("the journal")),
            before,
            "the journal inode was replaced"
        );
    }

    #[test]
    fn an_append_handle_writes_after_the_existing_records() {
        let (_dir, root) = temp_root();
        make_cell(&root, "cell-1", Some(b"one\n"));
        let opened = open_root(&root)
            .cell_dir(&cell("cell-1"))
            .expect("the cell");
        let mut handle = appended(&opened);
        assert!(!handle.created);
        handle
            .file
            .write_all(b"two\n")
            .expect("the record is written");
        assert_eq!(
            fs::read(root.join("cells/cell-1/ledger.ndjson")).expect("the journal"),
            b"one\ntwo\n"
        );
    }

    #[test]
    fn the_append_handle_and_the_constructor_name_the_same_file() {
        let (_dir, root) = temp_root();
        let created = open_root(&root)
            .create_cell_dir(&cell("cell-1"))
            .expect("the cell directory is created");
        let handle = appended(&created);
        let constructed = cell_ledger_path(&root, &cell("cell-1")).expect("a valid cell");
        assert_eq!(
            identity(&handle.file.metadata().expect("the handle's fstat")),
            identity(&fs::metadata(&constructed).expect("the constructed path")),
        );
        assert_eq!(created.journal_path(), constructed);
    }

    #[test]
    fn open_journal_for_append_refuses_a_symlinked_journal() {
        let (_dir, root) = temp_root();
        let cell_path = make_cell(&root, "cell-1", None);
        let target = root.join("elsewhere");
        fs::write(&target, b"keep").expect("the target is made");
        symlink(&target, cell_path.join("ledger.ndjson")).expect("the link is made");
        let opened = open_root(&root)
            .cell_dir(&cell("cell-1"))
            .expect("the cell");
        assert!(
            matches!(
                opened.open_journal_for_append(),
                Err(JournalRefusal::Symlink)
            ),
            "a symlinked journal is refused"
        );
        assert_eq!(fs::read(&target).expect("the target"), b"keep");
    }

    #[test]
    fn open_journal_for_append_refuses_a_directory_fifo_or_socket_journal() {
        let (_dir, root) = temp_root();
        fs::create_dir_all(root.join("cells/dir/ledger.ndjson")).expect("the directory is made");
        make_cell(&root, "fifo", None);
        make_fifo(&root.join("cells/fifo/ledger.ndjson"));
        let socket = make_cell(&root, "socket", None);
        let _socket = UnixListener::bind(socket.join("ledger.ndjson")).expect("the socket binds");
        let instance = open_root(&root);
        for name in ["dir", "fifo", "socket"] {
            let opened = instance.cell_dir(&cell(name)).expect("the cell");
            let result = within("opening a journal for appending", move || {
                opened.open_journal_for_append()
            });
            assert!(
                matches!(result, Err(JournalRefusal::NotRegular)),
                "{name}: {result:?}"
            );
        }
    }

    #[test]
    fn the_cell_and_journal_descriptors_are_close_on_exec() {
        let (_dir, root) = temp_root();
        let created = open_root(&root)
            .create_cell_dir(&cell("cell-1"))
            .expect("the cell directory is created");
        let handle = appended(&created);
        for (what, fd) in [
            ("cell", created.dir.as_raw_fd()),
            ("journal", handle.file.as_raw_fd()),
        ] {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            assert!(
                flags >= 0 && flags & libc::FD_CLOEXEC != 0,
                "the {what} descriptor would leak into a spawned process: flags {flags}"
            );
        }
    }
}
