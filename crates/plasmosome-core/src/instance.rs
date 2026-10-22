use std::ffi::{CStr, CString, OsStr};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{Mutex, PoisonError};

use plasmosome_backend::{BootSession, CellId};
use serde_json::Value;

use crate::nofollow::{Walk, WalkRefusal};
use crate::runtime::cell_id::{MAX_CELL_NUMBER, format_cell_id, parse_cell_id};
use crate::runtime::cell_paths::RUNTIME_DIR;
use crate::runtime::line::{LineFault, read_line};
use crate::state::{
    CELL_JOURNAL_FILE, CELLS_DIR, CellPathError, cell_ledger_path, validate_cell_id,
};
use crate::supervisor_check::LockFact;

const LOCK_FILE: &str = "controller.lock";
const PRIVATE_FILE: libc::mode_t = 0o600;
const PRIVATE_DIRECTORY: libc::mode_t = 0o700;
const DIRECTORY_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
const FILE_FLAGS: libc::c_int = libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
const NEW_FILE_FLAGS: libc::c_int =
    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
const LAUNCHED_FILE: &CStr = c"launched";
const LAUNCHED_TEMPORARY: &CStr = c"launched.tmp";
const CREATED_FILE: &CStr = c"created";
const RETIRED_DIR: &CStr = c"retired";
const CELLS_NAME: &CStr = c"cells";
const SUPERVISOR_LOCK_FILE: &CStr = c"supervisor.lock";
const SUPERVISOR_LOCK_TEMPORARY: &CStr = c"supervisor.lock.tmp";
const SUPERVISOR_LOG_FILE: &CStr = c"membraned.log";
const ALLOCATION_ATTEMPTS: usize = 64;
const CELL_ID_PREFIX: &[u8] = b"cell-";
const MARKER_KEYS: [&str; 3] = ["cell", "boot_session", "stopped"];
pub(crate) const MAX_MARKER_BYTES: usize = 4096;
const MARKER_READ_LIMIT: u64 = MAX_MARKER_BYTES as u64 + 1;

type SyncDirectory = fn(BorrowedFd<'_>) -> io::Result<()>;
type Identity = (u64, u64);

/// An opened instance root directory. `open` walks the root's path from `/` without following a
/// symlink. `/`, every directory below it and the root itself must pass the ancestor rule that
/// `membraned`'s socket check applies to them (spec 001 §4.1): owned by root or the effective
/// UID, writable by neither group nor other with no sticky-bit exception, and on macOS carrying
/// no ACL allow entry that lets anyone replace its entries and on no volume mounted with
/// ownership ignored. So a root under which every membrane would refuse its socket is refused
/// when it is opened, before anything is made in it. Every name beneath the root is opened
/// relative to this descriptor without following a symlink.
///
/// Private modes (0700 for directories, 0600 for files) are set with `fchmod`, whatever the
/// umask. A directory this user cannot read, such as one a umask of 0477 left at 0300, is first
/// set to 0700 by name with `fchmodat` and `AT_SYMLINK_NOFOLLOW`, then opened. That chmod never
/// adds group or other bits. It runs on any `EACCES` from opening a directory, not only one the
/// mode causes. So on macOS a deny-list ACL on `cells` at 0500 or 0300 makes `create_cell_dir`
/// fail with `Io` and `EACCES` and still leaves `cells` at 0700; `retired` and `runtime` go
/// through the same steps.
///
/// On Linux the C library decides whether `AT_SYMLINK_NOFOLLOW` works. glibc before 2.32
/// refuses it even with `/proc`. Later glibc needs `/proc` or, from glibc 2.39, the `fchmodat2`
/// call of Linux 6.6. When glibc refuses it, such a directory is an `Io` error. musl also needs
/// `/proc` or `fchmodat2`, but with neither it fails with `ENOENT`: `create_cell_dir` and
/// `CellDir::create_runtime_dir` then report a directory that exists as `Missing`, and
/// `retire` reports `Io` with `ENOENT`.
///
/// On macOS private modes do not restrict access when the root or `cells` carries an
/// inheritable ACL: the new entry inherits it and the mode does not show it. `open` refuses an
/// allow entry on the root or above it that lets anyone replace entries. Other entries, and
/// entries on what the root contains, are not checked, and nothing here removes an ACL.
#[derive(Debug)]
pub struct InstanceRoot {
    dir: OwnedFd,
    path: PathBuf,
    identity: Identity,
    sync: SyncDirectory,
}

impl InstanceRoot {
    /// Opens the directory at `path`, which must be absolute and normal: no empty, `.` or `..`
    /// component and no trailing `/`. A relative path is `NotAbsolute`, never resolved against
    /// the current directory. A path the rule above refuses is `Untrusted`, naming the root and
    /// the component that broke the rule. A component that is missing or cannot be opened is
    /// `Open`, naming that component. It creates nothing.
    pub fn open(path: &Path) -> Result<InstanceRoot, InstanceRootError> {
        if !path.is_absolute() {
            return Err(InstanceRootError::NotAbsolute {
                path: path.to_path_buf(),
            });
        }
        let opened = Walk::from_root(path)
            .and_then(|walk| walk.open_trusted_directory())
            .map_err(|refusal| root_refusal(path, refusal))?;
        Ok(InstanceRoot {
            dir: opened.fd,
            path: path.to_path_buf(),
            identity: (opened.facts.dev, opened.facts.ino),
            sync: sync_directory,
        })
    }

    /// The path this root was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Takes the instance's one writer lock: a nonblocking exclusive `flock` on
    /// `<root>/controller.lock`, opened without following a symlink and close-on-exec. The file
    /// is created if missing. Once the lock is held, every call sets the file to mode 0600 and
    /// syncs the root, so a retry after a failed call finishes what the failed call began.
    /// Another holder, in this process or another, makes this `Busy`; it never waits. The file is
    /// never unlinked.
    ///
    /// The lock lasts until the returned value is dropped or the process dies, provided no
    /// process forked from this one still has the lock file open without having called `exec`.
    /// Such a child keeps the lock alive after this process dies, and an unlock from it releases
    /// this process's lock. Close-on-exec covers a child only once it has called `exec`.
    ///
    /// The lock assumes a local filesystem and does not check for one. On NFS and similar
    /// filesystems `flock` may be emulated, lost silently or held past a crash; spec 008 makes
    /// it a local writer exclusion, not a cross-host lock.
    pub fn lock(&self) -> Result<WriterLock, LockError> {
        let path = self.path.join(LOCK_FILE);
        let io_error = |source| LockError::Io {
            path: path.clone(),
            source,
        };
        let name = c_name(LOCK_FILE.as_bytes()).map_err(io_error)?;
        let fd = self.open_lock_file(&name).map_err(|source| {
            match file_refusal(self.dir.as_fd(), &name, source) {
                FileRefusal::Symlink => LockError::Symlink { path: path.clone() },
                FileRefusal::NotRegular => LockError::NotRegular { path: path.clone() },
                FileRefusal::Io(source) => io_error(source),
            }
        })?;
        let file = File::from(fd);
        if !file.metadata().map_err(io_error)?.is_file() {
            return Err(LockError::NotRegular { path });
        }
        match lock_exclusive_nonblocking(file.as_fd()) {
            Err(source) if source.raw_os_error() == Some(libc::EWOULDBLOCK) => {
                return Err(LockError::Busy { path });
            }
            locked => locked.map_err(io_error)?,
        }
        change_mode(file.as_fd(), PRIVATE_FILE).map_err(io_error)?;
        (self.sync)(self.dir.as_fd()).map_err(|source| LockError::Io {
            path: self.path.clone(),
            source,
        })?;
        Ok(WriterLock {
            file,
            path,
            root: self.identity,
            allocation: Mutex::new(()),
        })
    }

    /// Lists every entry of `<root>/cells` and classifies it, opening nothing through a symlink.
    /// It needs this root's writer lock; a lock taken on another root is `ForeignLock`. A
    /// missing `cells` is a fresh instance with no entries. A symlinked or non-directory
    /// `cells`, or any failure to list or inspect all of its entries, refuses the whole
    /// discovery: a partial listing is never returned. Running out of file descriptors
    /// (`EMFILE`, `ENFILE`) is a fault of the process, not of a cell, and also refuses the whole
    /// discovery. A bad entry or an unopenable cell is reported in its own entry while its
    /// siblings are still classified. Entries come in raw-byte order of their names.
    pub fn discover(&self, lock: &WriterLock) -> Result<Discovery, DiscoveryError> {
        if lock.root != self.identity {
            return Err(DiscoveryError::ForeignLock {
                path: lock.path.clone(),
            });
        }
        let cells_path = self.path.join(CELLS_DIR);
        let cells = match self.open_cells() {
            Ok(cells) => cells,
            Err(DirectoryRefusal::Missing) => {
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
            Err(DirectoryRefusal::Io(source)) if out_of_descriptors(&source) => {
                return Err(DiscoveryError::OutOfDescriptors {
                    path: cells_path,
                    source,
                });
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
            .map_err(|source| {
                let path = cells_path.clone();
                if out_of_descriptors(&source) {
                    DiscoveryError::OutOfDescriptors { path, source }
                } else {
                    DiscoveryError::Listing { path, source }
                }
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
    /// invalid ID is refused before anything is opened, and a missing `cells` or cell is
    /// `Missing`. The name on disk must equal the ID byte for byte: on a filesystem that folds
    /// case or Unicode normalization, another spelling of an existing cell is refused as `Alias`.
    /// The ID must also still name the directory that was opened, so a cell retired or replaced
    /// while it is opened is `Missing`. Opening needs no lock; appending to the journal does.
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
            directory_refusal(cells.as_fd(), &name, error).into_cell_dir_error(cell_path.clone())
        })?;
        let named =
            naming(cells.as_fd(), &name, dir.as_fd()).map_err(|source| CellDirError::Io {
                path: cells_path.clone(),
                source,
            })?;
        match named {
            Naming::Exact => Ok(self.cell_dir_from(dir, cell.clone(), journal_path)),
            Naming::Alias => Err(CellDirError::Alias { path: cell_path }),
            Naming::Gone => Err(CellDirError::Missing { path: cell_path }),
        }
    }

    fn cell_dir_from(&self, dir: OwnedFd, cell: CellId, journal_path: PathBuf) -> CellDir {
        CellDir {
            dir,
            cell,
            journal_path,
            root: self.identity,
            sync: self.sync,
        }
    }

    /// Creates `<root>/cells/<cell>` exclusively with mode 0700, whatever the umask, creating
    /// `cells` when it is missing. It needs this root's writer lock; a lock taken on another root
    /// is `ForeignLock` and nothing is created. Each directory is made from its opened parent.
    /// Every call sets `cells` to mode 0700 and syncs the root and then `cells`, including a call
    /// that finds the cell already there, so a retry after a failed call makes durable what the
    /// failed call created. A failed sync names the directory that was being synced. An existing
    /// cell directory is `AlreadyExists` and is never adopted. An invalid ID is refused before
    /// anything is created.
    pub fn create_cell_dir(
        &self,
        lock: &WriterLock,
        cell: &CellId,
    ) -> Result<CellDir, CellDirError> {
        if lock.root != self.identity {
            return Err(CellDirError::ForeignLock {
                path: lock.path.clone(),
            });
        }
        let journal_path = cell_ledger_path(&self.path, cell).map_err(CellDirError::InvalidCell)?;
        let cells_path = self.path.join(CELLS_DIR);
        let cells = self.create_cells(&cells_path)?;
        let cell_path = cells_path.join(cell.as_str());
        let io_error = |source| CellDirError::Io {
            path: cell_path.clone(),
            source,
        };
        let sync_cells = || {
            (self.sync)(cells.as_fd()).map_err(|source| CellDirError::Io {
                path: cells_path.clone(),
                source,
            })
        };
        let name = c_name(cell.as_str().as_bytes()).map_err(io_error)?;
        match make_directory_at(cells.as_fd(), &name) {
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
                sync_cells()?;
                return Err(CellDirError::AlreadyExists { path: cell_path });
            }
            made => made.map_err(io_error)?,
        }
        let dir = open_private_directory(cells.as_fd(), &name)
            .map_err(|refusal| refusal.into_cell_dir_error(cell_path.clone()))?;
        sync_cells()?;
        Ok(self.cell_dir_from(dir, cell.clone(), journal_path))
    }

    /// Allocates the next cell ID under spec 023 §2 and creates its directory as
    /// [`InstanceRoot::create_cell_dir`] does: exclusive, mode 0700 and synced. The ID is
    /// `cell-<n>`, where `n` is one more than the largest number named in `cells` or `retired`,
    /// or 1 when there is none. Only names count, so an entry of any kind named `cell-<n>` keeps
    /// that ID from being allocated again, and a name that `parse_cell_id` refuses is ignored. A
    /// name whose `cell-` prefix differs only in ASCII case, such as `CELL-3`, counts as that ID,
    /// because a case-insensitive filesystem treats the two as one name. Listing creates neither
    /// directory. Each one found is set to mode 0700, as `create_cell_dir` does for `cells`. A
    /// symlink at either name is `Symlink` and anything else but a directory is `NotADirectory`:
    /// neither is followed, and nothing is created.
    ///
    /// It needs this root's writer lock; a lock taken on another root is `ForeignLock` and
    /// nothing is listed. Allocations under one writer lock run one at a time, from the listing
    /// until the directory exists, through every open of the root that uses that lock.
    /// [`InstanceRoot::retire`] does not take that mutex, so it can move a cell while an
    /// allocation lists. `cells` is listed before `retired` so that a cell it moves then is seen in
    /// one of the two listings, and its ID is not allocated again.
    ///
    /// If the chosen name already exists when the directory is made, the call lists again and
    /// tries the next number: the larger of the new listing's and one above the number that was
    /// taken. After 64 attempts it is `Contended`. When no number up to `cell-9999999999` is left
    /// it is `Exhausted`, naming the directory that holds the largest one. A failure from
    /// `create_cell_dir` is `Create`, which carries the ID it was creating. Its source alone does
    /// not tell whether a directory with that name exists. If the sync of `cells` failed after
    /// the `mkdir`, it does; but an `Io` error naming `cells` also comes from making or opening
    /// `cells` itself, and then no cell directory was made. A caller that must know opens it
    /// with [`InstanceRoot::cell_dir`]. The new directory has no `supervisor.lock` yet.
    pub fn allocate_cell(&self, lock: &WriterLock) -> Result<CellDir, AllocateError> {
        if lock.root != self.identity {
            return Err(AllocateError::ForeignLock {
                path: lock.path.clone(),
            });
        }
        let _allocating = lock
            .allocation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut lowest = 1;
        for _ in 0..ALLOCATION_ATTEMPTS {
            let next = self.next_cell_number(lowest)?;
            let cell = format_cell_id(next);
            match self.create_cell_dir(lock, &cell) {
                Err(CellDirError::AlreadyExists { .. }) => lowest = next + 1,
                created => return created.map_err(|source| AllocateError::Create { cell, source }),
            }
        }
        Err(AllocateError::Contended {
            path: self.path.join(CELLS_DIR),
        })
    }

    /// Retires a cell (spec 023 §7 step 5, and §8): renames `cells/<cell>` to `retired/<cell>`
    /// and unlinks nothing, so the journal, the markers, a stale `membrane.uds` and `runtime/`
    /// all move with it. It needs this root's writer lock; a lock taken on another root is
    /// `ForeignLock` and nothing is created or moved. The caller must also hold the cell's
    /// `supervisor.lock`, as spec 023 requires, which shows that no `membraned` still runs in
    /// the directory; this call does not take or check that lock. A caller holds it when it has
    /// a [`SupervisorLock`] for this cell that it has not handed over, from
    /// [`CellDir::create_supervisor_lock`], a `Free` [`CellDir::probe_supervisor_lock`] or a
    /// `Free` [`LockTest`].
    ///
    /// `retired` is created when it is missing. Every call sets it to mode 0700 and syncs the
    /// root, as [`InstanceRoot::create_cell_dir`] does for `cells`; if that sync fails, nothing
    /// is moved. A symlink or anything but a directory there is `RetiredNotADirectory`. The name
    /// `cells/<cell>` must still be the directory `cell` holds, by device and inode, or the call
    /// is `Moved`. The rename never replaces: an existing `retired/<cell>` is `AlreadyRetired`.
    /// On Linux that needs a filesystem with `RENAME_NOREPLACE`; on one without it the rename is
    /// an `Io` error and nothing is replaced. After the rename, `cells` and then `retired` are
    /// synced; if either sync fails, the cell is already under `retired` and the error names the
    /// directory that failed. The identity check and the rename are two steps, so this relies on
    /// the controller being the only writer of `cells`.
    pub fn retire(&self, lock: &WriterLock, cell: CellDir) -> Result<(), RetireError> {
        if lock.root != self.identity {
            return Err(RetireError::ForeignLock {
                path: lock.path.clone(),
            });
        }
        let cells_path = self.path.join(CELLS_DIR);
        let cell_path = cells_path.join(cell.cell.as_str());
        let retired_path = self.path.join(name_of(RETIRED_DIR));
        let cells = self.open_cells().map_err(|refusal| match refusal {
            DirectoryRefusal::Io(source) => retire_io(&cells_path)(source),
            DirectoryRefusal::Missing
            | DirectoryRefusal::Symlink
            | DirectoryRefusal::NotADirectory => RetireError::Moved {
                path: cell_path.clone(),
            },
        })?;
        let retired = self.open_retired(&retired_path)?;
        let name = c_name(cell.cell.as_str().as_bytes()).map_err(retire_io(&cell_path))?;
        match naming(cells.as_fd(), &name, cell.dir.as_fd()).map_err(retire_io(&cells_path))? {
            Naming::Exact => {}
            Naming::Alias | Naming::Gone => return Err(RetireError::Moved { path: cell_path }),
        }
        rename_no_replace(cells.as_fd(), &name, retired.as_fd(), &name).map_err(|source| {
            if source.raw_os_error() == Some(libc::EEXIST) {
                RetireError::AlreadyRetired {
                    path: retired_path.join(cell.cell.as_str()),
                }
            } else {
                retire_io(&cell_path)(source)
            }
        })?;
        (self.sync)(cells.as_fd()).map_err(retire_io(&cells_path))?;
        (self.sync)(retired.as_fd()).map_err(retire_io(&retired_path))
    }

    fn open_retired(&self, retired_path: &Path) -> Result<OwnedFd, RetireError> {
        match make_directory_at(self.dir.as_fd(), RETIRED_DIR) {
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {}
            made => made.map_err(retire_io(retired_path))?,
        }
        let retired =
            open_private_directory(self.dir.as_fd(), RETIRED_DIR).map_err(
                |refusal| match refusal {
                    DirectoryRefusal::Symlink | DirectoryRefusal::NotADirectory => {
                        RetireError::RetiredNotADirectory {
                            path: retired_path.to_path_buf(),
                        }
                    }
                    DirectoryRefusal::Missing => {
                        retire_io(retired_path)(io::Error::from_raw_os_error(libc::ENOENT))
                    }
                    DirectoryRefusal::Io(source) => retire_io(retired_path)(source),
                },
            )?;
        (self.sync)(self.dir.as_fd()).map_err(retire_io(&self.path))?;
        Ok(retired)
    }

    fn create_cells(&self, cells_path: &Path) -> Result<OwnedFd, CellDirError> {
        let io_error = |source| CellDirError::Io {
            path: cells_path.to_path_buf(),
            source,
        };
        let name = c_name(CELLS_DIR.as_bytes()).map_err(io_error)?;
        match make_directory_at(self.dir.as_fd(), &name) {
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {}
            made => made.map_err(io_error)?,
        }
        let cells = open_private_directory(self.dir.as_fd(), &name)
            .map_err(|refusal| refusal.into_cell_dir_error(cells_path.to_path_buf()))?;
        (self.sync)(self.dir.as_fd()).map_err(|source| CellDirError::Io {
            path: self.path.clone(),
            source,
        })?;
        Ok(cells)
    }

    fn next_cell_number(&self, lowest: u64) -> Result<u64, AllocateError> {
        let in_cells = self.largest_cell_number(CELLS_NAME)?;
        #[cfg(test)]
        tests::between_listings();
        let in_retired = self.largest_cell_number(RETIRED_DIR)?;
        let (largest, holder) = if in_retired > in_cells {
            (in_retired, name_of(RETIRED_DIR))
        } else {
            (in_cells, OsStr::new(CELLS_DIR))
        };
        let above = largest + 1;
        if lowest.max(above) > MAX_CELL_NUMBER {
            let holder = if lowest > above {
                OsStr::new(CELLS_DIR)
            } else {
                holder
            };
            return Err(AllocateError::Exhausted {
                path: self.path.join(holder),
            });
        }
        Ok(lowest.max(above))
    }

    fn largest_cell_number(&self, name: &CStr) -> Result<u64, AllocateError> {
        let path = self.path.join(name_of(name));
        let dir = match open_private_directory(self.dir.as_fd(), name) {
            Ok(dir) => dir,
            Err(DirectoryRefusal::Missing) => return Ok(0),
            Err(DirectoryRefusal::Symlink) => return Err(AllocateError::Symlink { path }),
            Err(DirectoryRefusal::NotADirectory) => {
                return Err(AllocateError::NotADirectory { path });
            }
            Err(DirectoryRefusal::Io(source)) => {
                return Err(AllocateError::Listing { path, source });
            }
        };
        let listing = |source| AllocateError::Listing {
            path: path.clone(),
            source,
        };
        let mut largest = 0;
        for listed in DirectoryStream::open(dir.as_fd()).map_err(listing)? {
            let listed = listed.map_err(listing)?;
            largest = largest.max(listed_cell_number(&listed).unwrap_or(0));
        }
        Ok(largest)
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
        let out_of_descriptors_error = |source| DiscoveryError::OutOfDescriptors {
            path: path.clone(),
            source,
        };
        let name = c_name(&raw_name).map_err(classify_error)?;
        let not_a_cell = |kind| match kind {
            Kind::Symlink => EntryClass::NotACell(NotACell::Symlink),
            Kind::Regular | Kind::Other => EntryClass::NotACell(NotACell::NotADirectory),
            Kind::Directory => EntryClass::NotACell(NotACell::InvalidName),
        };
        let class = match self.cell_named(&raw_name) {
            None => not_a_cell(kind_at(cells, &name).map_err(classify_error)?),
            Some((cell, journal_path)) => match open_at(cells, &name, DIRECTORY_FLAGS, 0) {
                Ok(dir) => match self
                    .cell_dir_from(dir, cell.clone(), journal_path)
                    .open_journal()
                {
                    JournalOpen::Refused(JournalRefusal::Io(source))
                        if out_of_descriptors(&source) =>
                    {
                        return Err(out_of_descriptors_error(source));
                    }
                    journal => EntryClass::Cell { cell, journal },
                },
                Err(source) if out_of_descriptors(&source) => {
                    return Err(out_of_descriptors_error(source));
                }
                Err(error) => match kind_at(cells, &name).map_err(classify_error)? {
                    Kind::Directory => EntryClass::Cell {
                        cell,
                        journal: JournalOpen::Refused(JournalRefusal::CellDirectory(error)),
                    },
                    kind => not_a_cell(kind),
                },
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

    fn open_lock_file(&self, name: &CStr) -> io::Result<OwnedFd> {
        let flags = libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        match open_at(self.dir.as_fd(), name, flags, 0) {
            Err(missing) if missing.raw_os_error() == Some(libc::ENOENT) => {
                let exclusive = flags | libc::O_CREAT | libc::O_EXCL;
                match open_at(self.dir.as_fd(), name, exclusive, PRIVATE_FILE) {
                    Err(raced) if raced.raw_os_error() == Some(libc::EEXIST) => {
                        open_at(self.dir.as_fd(), name, flags, 0)
                    }
                    created => created,
                }
            }
            opened => opened,
        }
    }
}

/// The instance's writer lock. Dropping it releases the lock and leaves `controller.lock` in
/// place; so does the death of the process holding it, unless a forked child that has not
/// called `exec` still holds the file (see [`InstanceRoot::lock`]). Discovery, cell creation and
/// journal appends take it as proof that the caller holds the lock of the same root. It also
/// holds the instance's one allocation mutex, so [`InstanceRoot::allocate_cell`] calls that use
/// it run one at a time.
#[must_use = "dropping the WriterLock releases the instance lock"]
pub struct WriterLock {
    file: File,
    path: PathBuf,
    root: Identity,
    allocation: Mutex<()>,
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
    root: Identity,
    sync: SyncDirectory,
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
    /// FIFO or truncating. It needs the writer lock of the root this directory was opened from;
    /// another root's lock is `ForeignLock`. An existing regular journal returns
    /// `created: false`. A missing one is created exclusively and returns `created: true`. Either
    /// way the journal is set to mode 0600. This call does not sync the journal or its directory.
    /// An existing inode is never replaced.
    pub fn open_journal_for_append(
        &self,
        lock: &WriterLock,
    ) -> Result<JournalAppend, JournalRefusal> {
        if lock.root != self.root {
            return Err(JournalRefusal::ForeignLock {
                path: lock.path.clone(),
            });
        }
        let access = libc::O_RDWR | libc::O_APPEND;
        let (file, created) = match open_journal_at(self.dir.as_fd(), access, 0) {
            Ok(file) => (file, false),
            Err(JournalRefusal::Io(error)) if error.raw_os_error() == Some(libc::ENOENT) => {
                let exclusive = access | libc::O_CREAT | libc::O_EXCL;
                let file = open_journal_at(self.dir.as_fd(), exclusive, PRIVATE_FILE)?;
                (file, true)
            }
            Err(refusal) => return Err(refusal),
        };
        change_mode(file.as_fd(), PRIVATE_FILE).map_err(JournalRefusal::Io)?;
        Ok(JournalAppend { file, created })
    }

    /// Syncs this cell directory, making entries created in it durable.
    pub fn sync(&self) -> io::Result<()> {
        (self.sync)(self.dir.as_fd())
    }

    /// Creates spec 023 §3's `launched` marker, which records this cell and `boot_session` and
    /// no clean stop. The record is written to `launched.tmp`, created exclusively without
    /// following a symlink, with mode 0600 whatever the umask, and synced. It is then renamed to
    /// `launched` without replacing anything, and this directory is synced. A reader sees no
    /// marker or the whole record, never part of it. Any `launched.tmp` left by an earlier call
    /// is removed first. An existing entry named `launched`, a symlink included, is
    /// `AlreadyExists`: it is left as it was, and the temporary is removed. A cell ID that is not
    /// spec 023's `cell-<n>` is `InvalidCell` and nothing is written, since
    /// [`CellDir::read_launched`] would refuse its record.
    ///
    /// A failure before the rename leaves no `launched`, so `read_launched` reports `Absent`; it
    /// may leave `launched.tmp`, which the next call removes. If the directory sync fails, the
    /// marker is already in place: `read_launched` reports it as `Present`, a retry is
    /// `AlreadyExists`, and [`CellDir::sync`] completes the sync.
    ///
    /// The caller must be the only writer of this cell's `launched`, as `membraned` is while it
    /// holds the cell's `supervisor.lock`; this call does not check it. A second writer can
    /// replace the temporary before the rename, and a reader can then see an empty marker.
    pub fn create_launched(&self, boot_session: BootSession) -> Result<(), CellDirError> {
        if parse_cell_id(self.cell.as_str()).is_none() {
            return Err(CellDirError::InvalidCell(CellPathError::NotACellName(
                self.cell.as_str().to_owned(),
            )));
        }
        let marker = LaunchedMarker {
            cell: self.cell.clone(),
            boot_session,
            stopped: false,
        };
        let path = self.entry_path(LAUNCHED_FILE);
        self.write_temporary(&marker.encode())?;
        match rename_no_replace(
            self.dir.as_fd(),
            LAUNCHED_TEMPORARY,
            self.dir.as_fd(),
            LAUNCHED_FILE,
        ) {
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
                remove_at(self.dir.as_fd(), LAUNCHED_TEMPORARY)
                    .map_err(cell_dir_io(&self.entry_path(LAUNCHED_TEMPORARY)))?;
                return Err(CellDirError::AlreadyExists { path });
            }
            renamed => renamed.map_err(cell_dir_io(&path))?,
        }
        self.sync_dir()
    }

    /// Reads the `launched` marker without following a symlink and without blocking on a FIFO.
    /// It is `Absent` only when no entry named `launched` exists. A symlink, a directory or any
    /// other entry that is not a regular file, a record that is not one line of strict JSON
    /// with a `cell-<n>` cell, a lowercase hyphenated boot session and, if any,
    /// `"stopped": true`, in 4,096 bytes or fewer with its newline, a marker naming another
    /// cell, and any error opening or reading it are `Unreadable`.
    pub fn read_launched(&self) -> MarkerRead {
        let fd = match open_at(
            self.dir.as_fd(),
            LAUNCHED_FILE,
            libc::O_RDONLY | FILE_FLAGS,
            0,
        ) {
            Ok(fd) => fd,
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => return MarkerRead::Absent,
            Err(error) => {
                return MarkerRead::Unreadable(
                    match file_refusal(self.dir.as_fd(), LAUNCHED_FILE, error) {
                        FileRefusal::Symlink => MarkerFault::Symlink,
                        FileRefusal::NotRegular => MarkerFault::NotRegular,
                        FileRefusal::Io(error) => MarkerFault::Io(error),
                    },
                );
            }
        };
        let file = File::from(fd);
        match file.metadata() {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return MarkerRead::Unreadable(MarkerFault::NotRegular),
            Err(error) => return MarkerRead::Unreadable(MarkerFault::Io(error)),
        }
        let mut bytes = Vec::new();
        if let Err(error) = file.take(MARKER_READ_LIMIT).read_to_end(&mut bytes) {
            return MarkerRead::Unreadable(MarkerFault::Io(error));
        }
        match LaunchedMarker::decode(&bytes) {
            Ok(marker) if marker.cell == self.cell => MarkerRead::Present(marker),
            Ok(marker) => MarkerRead::Unreadable(MarkerFault::WrongCell { found: marker.cell }),
            Err(fault) => MarkerRead::Unreadable(fault),
        }
    }

    /// Replaces the marker with the same record and `"stopped": true` (spec 023 §5, step 9).
    /// Any `launched.tmp` left by an earlier call is removed. The new record is written to
    /// `launched.tmp`, created exclusively with mode 0600, which is synced and renamed over
    /// `launched`; then this directory is synced. A reader sees the old record or the new one,
    /// never part of either. It needs this cell's marker: with none it is `Missing`, and an
    /// unreadable one is `MarkerUnreadable`; either way nothing is written. A stopped marker is
    /// written again unchanged.
    ///
    /// A failure before the rename leaves the old record, and may leave `launched.tmp`, which the
    /// next call removes. If the directory sync fails, the new record is already in place, and a
    /// retry writes it again and syncs.
    ///
    /// The caller must be the only writer of this cell's `launched`, as `membraned` is while it
    /// holds the cell's `supervisor.lock`; this call does not check it. A second writer can
    /// replace the temporary before the rename, and a reader can then see an empty marker.
    pub fn mark_launched_stopped(&self) -> Result<(), CellDirError> {
        let path = self.entry_path(LAUNCHED_FILE);
        let marker = match self.read_launched() {
            MarkerRead::Present(marker) => marker,
            MarkerRead::Absent => return Err(CellDirError::Missing { path }),
            MarkerRead::Unreadable(fault) => {
                return Err(CellDirError::MarkerUnreadable { path, fault });
            }
        };
        let stopped = LaunchedMarker {
            stopped: true,
            ..marker
        };
        self.write_temporary(&stopped.encode())?;
        rename_at(self.dir.as_fd(), LAUNCHED_TEMPORARY, LAUNCHED_FILE)
            .map_err(cell_dir_io(&path))?;
        self.sync_dir()
    }

    /// Removes the entry named `launched` and syncs this directory. A missing entry is not an
    /// error, and the directory is still synced, so a retry after a failed sync completes. If the
    /// sync fails, the marker is already gone and the error names this directory. A directory
    /// named `launched` is not removed and is an `Io` error.
    pub fn remove_launched(&self) -> Result<(), CellDirError> {
        match remove_at(self.dir.as_fd(), LAUNCHED_FILE) {
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {}
            removed => removed.map_err(cell_dir_io(&self.entry_path(LAUNCHED_FILE)))?,
        }
        self.sync_dir()
    }

    /// Creates `runtime/` exclusively with mode 0700 whatever the umask, opens it without
    /// following a symlink, syncs this directory and returns the opened directory, close-on-exec.
    /// Any existing entry named `runtime` is `AlreadyExists`: a runtime directory is never
    /// reused. If the sync fails, `runtime/` is already in place with mode 0700 and the error
    /// names this directory; a retry is `AlreadyExists`, and [`CellDir::sync`] completes the sync.
    pub fn create_runtime_dir(&self) -> Result<OwnedFd, CellDirError> {
        let path = self.entry_path(RUNTIME_DIR);
        match make_directory_at(self.dir.as_fd(), RUNTIME_DIR) {
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
                return Err(CellDirError::AlreadyExists { path });
            }
            made => made.map_err(cell_dir_io(&path))?,
        }
        let runtime = open_private_directory(self.dir.as_fd(), RUNTIME_DIR)
            .map_err(|refusal| refusal.into_cell_dir_error(path.clone()))?;
        self.sync_dir()?;
        Ok(runtime)
    }

    /// Whether the empty `created` file exists, read without following a symlink. An entry that
    /// is not a regular file is `NotARegularFile`, never `true`.
    pub fn has_created(&self) -> Result<bool, CellDirError> {
        let path = self.entry_path(CREATED_FILE);
        match kind_at(self.dir.as_fd(), CREATED_FILE) {
            Ok(Kind::Regular) => Ok(true),
            Ok(_) => Err(CellDirError::NotARegularFile { path }),
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => Ok(false),
            Err(error) => Err(cell_dir_io(&path)(error)),
        }
    }

    /// Creates the empty `created` file once `cell.new` has completed (spec 023 §4, step 8): it
    /// is created exclusively, without following a symlink, with mode 0600 whatever the umask;
    /// then the file is synced, and then this directory. It needs the writer lock of the root
    /// this directory was opened from; another root's lock is `ForeignLock` and nothing is
    /// created. Any existing entry named `created` is `AlreadyExists`. If either sync fails,
    /// `created` is already in place: [`CellDir::has_created`] reports `true`, and a retry is
    /// `AlreadyExists`.
    pub fn create_created(&self, lock: &WriterLock) -> Result<(), CellDirError> {
        if lock.root != self.root {
            return Err(CellDirError::ForeignLock {
                path: lock.path.clone(),
            });
        }
        self.write_new_file(CREATED_FILE, b"")?;
        self.sync_dir()
    }

    /// Creates spec 023 §3's `supervisor.lock` already locked (spec 023 §4, step 4), so it is
    /// held from the moment it has its name. The file is created as `supervisor.lock.tmp`,
    /// exclusively, without following a symlink, with mode 0600 whatever the umask, and
    /// close-on-exec. `flock(LOCK_EX | LOCK_NB)` is taken on it, it is renamed to
    /// `supervisor.lock` without replacing anything, and this directory is synced. With one call
    /// at a time, a probe sees no entry and then a held lock, never a free one. The returned
    /// [`SupervisorLock`] holds the lock. On Linux the rename needs a filesystem with
    /// `RENAME_NOREPLACE`; on one without it the call is an `Io` error.
    ///
    /// Any `supervisor.lock.tmp` left by an earlier call is removed first; a symlink there is
    /// removed, not followed. Every failure before the rename removes the temporary. An existing
    /// entry named `supervisor.lock` is left as it was: a symlink is `Symlink`, a directory or
    /// other non-regular file is `NotARegularFile`, and a file is `AlreadyExists`. If the sync
    /// fails, the error is `Io` and the lock file stays in place: a retry is `AlreadyExists`, and
    /// once nothing else holds it [`CellDir::probe_supervisor_lock`] reports it `Free`.
    ///
    /// Two calls for one cell at once can get in each other's way: the second call's first step
    /// removes the first call's temporary as a leftover, and the first call's rename then moves
    /// the second call's file into place. So after the rename the call reads the entry named
    /// `supervisor.lock`, without following a symlink, and returns the lock only when that entry
    /// is the file it holds. Otherwise it is `LockNotNamed`, naming `supervisor.lock` in this
    /// cell, no lock is returned, and the entry is left as it is. If the name is gone by then, the
    /// error is `Io` naming `supervisor.lock`. `Ok` therefore always means that `supervisor.lock`
    /// names the held file. When calls race, any of them can fail, and a file no call returned can
    /// be left at `supervisor.lock`; a probe reads it `Free` once nothing holds it.
    ///
    /// A directory at `supervisor.lock.tmp` cannot be unlinked, so every call is then an `Io`
    /// error naming it, while a probe reads `Missing`. Only a writer under the root can plant
    /// one, and such a writer can already plant one at `supervisor.lock`.
    pub fn create_supervisor_lock(&self) -> Result<SupervisorLock, CellDirError> {
        let path = self.entry_path(SUPERVISOR_LOCK_FILE);
        self.remove_if_present(SUPERVISOR_LOCK_TEMPORARY)?;
        let file = match self.lock_and_name_supervisor_lock(&path) {
            Ok(file) => file,
            Err(error) => {
                self.remove_if_present(SUPERVISOR_LOCK_TEMPORARY)?;
                return Err(error);
            }
        };
        #[cfg(test)]
        tests::after_naming_the_lock();
        self.check_lock_named(&file, &path)?;
        self.sync_dir()?;
        Ok(SupervisorLock { file, path })
    }

    fn check_lock_named(&self, file: &File, path: &Path) -> Result<(), CellDirError> {
        let named = stat_at(self.dir.as_fd(), SUPERVISOR_LOCK_FILE).map_err(cell_dir_io(path))?;
        let held = stat_of(file.as_fd()).map_err(cell_dir_io(path))?;
        if (named.st_dev, named.st_ino) == (held.st_dev, held.st_ino) {
            Ok(())
        } else {
            Err(CellDirError::LockNotNamed {
                path: path.to_path_buf(),
            })
        }
    }

    fn lock_and_name_supervisor_lock(&self, path: &Path) -> Result<File, CellDirError> {
        let file = self.create_private_file(SUPERVISOR_LOCK_TEMPORARY)?;
        lock_exclusive_nonblocking(file.as_fd())
            .map_err(cell_dir_io(&self.entry_path(SUPERVISOR_LOCK_TEMPORARY)))?;
        #[cfg(test)]
        tests::before_naming_the_lock();
        match rename_no_replace(
            self.dir.as_fd(),
            SUPERVISOR_LOCK_TEMPORARY,
            self.dir.as_fd(),
            SUPERVISOR_LOCK_FILE,
        ) {
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
                let path = path.to_path_buf();
                Err(match kind_at(self.dir.as_fd(), SUPERVISOR_LOCK_FILE) {
                    Ok(Kind::Symlink) => CellDirError::Symlink { path },
                    Ok(Kind::Directory | Kind::Other) => CellDirError::NotARegularFile { path },
                    Ok(Kind::Regular) | Err(_) => CellDirError::AlreadyExists { path },
                })
            }
            renamed => renamed.map(|()| file).map_err(cell_dir_io(path)),
        }
    }

    /// Tries `supervisor.lock` on a fresh descriptor (spec 023 §4 step 6, §7 step 4 and §8),
    /// opened read-only, close-on-exec, without following a symlink and without blocking. `Held`
    /// means another open of the file holds its `flock`, in this process or another, and the
    /// probe's descriptor is closed. `Free` means the probe took the lock, which the returned
    /// [`SupervisorLock`] keeps until it is dropped. `Missing` means no entry named
    /// `supervisor.lock` exists; nothing is created. A symlink there is `Symlink`, and anything
    /// but a regular file is `NotARegularFile`; neither is locked. An `flock` that fails for
    /// another reason than another holder is `Io`. The probe never calls `LOCK_UN`.
    pub fn probe_supervisor_lock(&self) -> Result<LockProbe, CellDirError> {
        let path = self.entry_path(SUPERVISOR_LOCK_FILE);
        let Some(file) = self.open_supervisor_lock()? else {
            return Ok(LockProbe::Missing);
        };
        let locked = lock_exclusive_nonblocking(file.as_fd());
        #[cfg(test)]
        let locked = tests::probe_lock_result(locked);
        match locked {
            Ok(()) => Ok(LockProbe::Free(SupervisorLock { file, path })),
            Err(error) if error.raw_os_error() == Some(libc::EWOULDBLOCK) => Ok(LockProbe::Held),
            Err(error) => Err(cell_dir_io(&path)(error)),
        }
    }

    /// Tests `supervisor.lock` as spec 023 §8's check does: it probes the lock, and creates and
    /// locks a missing one as [`CellDir::create_supervisor_lock`] does. Only `Free` holds the
    /// lock, until its [`SupervisorLock`] is dropped. Every error of the probe or the create is
    /// `Untestable`, which keeps the error for logs and holds nothing. Use
    /// [`CellDir::probe_supervisor_lock`] where a missing lock must not be created, as in spec 023
    /// §4 step 6. Test one cell's lock one call at a time, as spec 023 §6 has one cell's mutations,
    /// the check of §8 among them, "run one at a time": two tests at once can both come back
    /// `Untestable` and set a healthy cell aside.
    pub fn test_supervisor_lock(&self) -> LockTest {
        self.probe_supervisor_lock()
            .and_then(|probe| match probe {
                LockProbe::Held => Ok(LockTest::Held),
                LockProbe::Free(lock) => Ok(LockTest::Free(lock)),
                LockProbe::Missing => self.create_supervisor_lock().map(LockTest::Free),
            })
            .unwrap_or_else(LockTest::Untestable)
    }

    /// The device and inode of the file named `supervisor.lock`, opened as
    /// [`CellDir::probe_supervisor_lock`] opens it, for `membraned` to check that the descriptor
    /// it was handed is this cell's lock (spec 023 §5, step 2). It takes no lock. With no entry
    /// there it is `Missing`; a symlink or anything but a regular file is refused as the probe
    /// refuses it.
    pub fn supervisor_lock_identity(&self) -> Result<FileIdentity, CellDirError> {
        let path = self.entry_path(SUPERVISOR_LOCK_FILE);
        match self.open_supervisor_lock()? {
            Some(file) => file_identity(&file).map_err(cell_dir_io(&path)),
            None => Err(CellDirError::Missing { path }),
        }
    }

    /// Opens `membraned.log`, `membraned`'s stderr (spec 023 §4), for appending. A missing log is
    /// created; an existing one is kept, never truncated. Either way it is set to mode 0600
    /// whatever the umask. It is opened without following a symlink and without blocking on a
    /// FIFO, and close-on-exec; the returned file is in blocking mode. A symlink is `Symlink` and
    /// anything but a regular file is `NotARegularFile`; neither is written.
    pub fn open_supervisor_log(&self) -> Result<File, CellDirError> {
        let path = self.entry_path(SUPERVISOR_LOG_FILE);
        let access = libc::O_WRONLY | libc::O_APPEND | libc::O_CREAT;
        let file = open_regular_at(self.dir.as_fd(), SUPERVISOR_LOG_FILE, access, PRIVATE_FILE)
            .map_err(|refusal| refusal.into_cell_dir_error(path.clone()))?;
        change_mode(file.as_fd(), PRIVATE_FILE).map_err(cell_dir_io(&path))?;
        clear_nonblocking(file.as_fd()).map_err(cell_dir_io(&path))?;
        Ok(file)
    }

    fn open_supervisor_lock(&self) -> Result<Option<File>, CellDirError> {
        match open_regular_at(self.dir.as_fd(), SUPERVISOR_LOCK_FILE, libc::O_RDONLY, 0) {
            Ok(file) => Ok(Some(file)),
            Err(FileRefusal::Io(error)) if error.raw_os_error() == Some(libc::ENOENT) => Ok(None),
            Err(refusal) => Err(refusal.into_cell_dir_error(self.entry_path(SUPERVISOR_LOCK_FILE))),
        }
    }

    fn entry_path(&self, name: &CStr) -> PathBuf {
        self.journal_path.with_file_name(name_of(name))
    }

    fn sync_dir(&self) -> Result<(), CellDirError> {
        let path = self
            .journal_path
            .parent()
            .map_or_else(PathBuf::new, Path::to_path_buf);
        self.sync().map_err(cell_dir_io(&path))
    }

    fn remove_if_present(&self, name: &CStr) -> Result<(), CellDirError> {
        match remove_at(self.dir.as_fd(), name) {
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => Ok(()),
            removed => removed.map_err(cell_dir_io(&self.entry_path(name))),
        }
    }

    fn write_temporary(&self, contents: &[u8]) -> Result<(), CellDirError> {
        match remove_at(self.dir.as_fd(), LAUNCHED_TEMPORARY) {
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {}
            removed => removed.map_err(cell_dir_io(&self.entry_path(LAUNCHED_TEMPORARY)))?,
        }
        self.write_new_file(LAUNCHED_TEMPORARY, contents)
    }

    fn write_new_file(&self, name: &CStr, contents: &[u8]) -> Result<(), CellDirError> {
        let path = self.entry_path(name);
        let mut file = self.create_private_file(name)?;
        file.write_all(contents).map_err(cell_dir_io(&path))?;
        (self.sync)(file.as_fd()).map_err(cell_dir_io(&path))
    }

    fn create_private_file(&self, name: &CStr) -> Result<File, CellDirError> {
        let path = self.entry_path(name);
        let fd = match open_at(self.dir.as_fd(), name, NEW_FILE_FLAGS, PRIVATE_FILE) {
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
                return Err(CellDirError::AlreadyExists { path });
            }
            created => created.map_err(cell_dir_io(&path))?,
        };
        change_mode(fd.as_fd(), PRIVATE_FILE).map_err(cell_dir_io(&path))?;
        Ok(File::from(fd))
    }
}

/// A cell's `supervisor.lock`, open and holding its `flock`. The lock belongs to the open file,
/// not to this value: dropping it closes this descriptor and releases the lock only once no
/// other descriptor of the same open file remains, such as the copy handed to `membraned`.
/// Nothing here calls `LOCK_UN`. A process forked from this one that has not called `exec`
/// keeps the lock alive, as it does the [`WriterLock`]. The descriptor is close-on-exec.
#[must_use = "dropping the SupervisorLock releases the cell's supervisor lock"]
pub struct SupervisorLock {
    file: File,
    path: PathBuf,
}

impl SupervisorLock {
    /// The device and inode of the locked file, from `fstat` on the held descriptor.
    pub fn identity(&self) -> io::Result<FileIdentity> {
        file_identity(&self.file)
    }

    /// Gives up this value for its descriptor, which holds the lock until it and every copy of it
    /// are closed.
    pub fn into_owned_fd(self) -> OwnedFd {
        OwnedFd::from(self.file)
    }
}

impl AsFd for SupervisorLock {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl fmt::Debug for SupervisorLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SupervisorLock")
            .field("fd", &self.file.as_raw_fd())
            .field("path", &self.path)
            .finish()
    }
}

/// What [`CellDir::probe_supervisor_lock`] found. Only `Free` holds the lock.
#[derive(Debug)]
#[must_use = "a Free probe holds the supervisor lock until it is dropped"]
pub enum LockProbe {
    Held,
    Free(SupervisorLock),
    Missing,
}

/// What [`CellDir::test_supervisor_lock`] found. Only `Free` holds the lock. `Untestable` keeps
/// the error that made the lock untestable.
#[derive(Debug)]
#[must_use = "a Free lock test holds the supervisor lock until it is dropped"]
pub enum LockTest {
    Held,
    Free(SupervisorLock),
    Untestable(CellDirError),
}

impl LockTest {
    /// The fact spec 023 §8's table decides on.
    pub fn fact(&self) -> LockFact {
        match self {
            Self::Held => LockFact::Held,
            Self::Free(_) => LockFact::Free,
            Self::Untestable(_) => LockFact::Untestable,
        }
    }
}

/// A file's device and inode, as `fstat` reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
}

/// Why no cell was allocated. `Symlink` names a `cells` or `retired` that is a symlink, and
/// `NotADirectory` one that is anything else but a directory. `Exhausted` means no ID up to
/// `cell-9999999999` is left, and names the directory that holds the largest one. `Contended`
/// means each of 64 attempts found the name it chose already taken. `Create` carries the cell
/// ID that [`InstanceRoot::create_cell_dir`] failed to create.
#[derive(Debug)]
pub enum AllocateError {
    ForeignLock { path: PathBuf },
    Symlink { path: PathBuf },
    NotADirectory { path: PathBuf },
    Listing { path: PathBuf, source: io::Error },
    Exhausted { path: PathBuf },
    Contended { path: PathBuf },
    Create { cell: CellId, source: CellDirError },
}

/// Spec 023 §3's `launched` marker: the cell it belongs to and the host boot session its
/// supervisor started in. `stopped` is true once the supervisor recorded a clean stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchedMarker {
    pub cell: CellId,
    pub boot_session: BootSession,
    pub stopped: bool,
}

impl LaunchedMarker {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let cell = Value::from(self.cell.as_str());
        let stopped = if self.stopped {
            ",\"stopped\":true"
        } else {
            ""
        };
        format!(
            "{{\"cell\":{cell},\"boot_session\":\"{}\"{stopped}}}\n",
            self.boot_session
        )
        .into_bytes()
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<LaunchedMarker, MarkerFault> {
        if bytes.len() > MAX_MARKER_BYTES {
            return Err(MarkerFault::TooLarge);
        }
        let malformed = |reason: &str| MarkerFault::Malformed(reason.to_owned());
        let value = read_line(bytes).map_err(|fault| match fault {
            LineFault::NotOneLine => malformed("the record is not one line ending in a newline"),
            LineFault::TooLarge { .. } => MarkerFault::TooLarge,
            LineFault::NotUtf8 => malformed("the record is not UTF-8"),
            LineFault::NotJson { detail } => MarkerFault::Malformed(format!("not JSON: {detail}")),
            LineFault::DuplicateKey { field } => {
                MarkerFault::Malformed(format!("the key {field:?} is repeated"))
            }
        })?;
        let Value::Object(entries) = value else {
            return Err(malformed("the record is not a JSON object"));
        };
        if let Some(key) = entries
            .keys()
            .find(|key| !MARKER_KEYS.contains(&key.as_str()))
        {
            return Err(MarkerFault::Malformed(format!("an unknown key {key:?}")));
        }
        let cell = match entries.get("cell") {
            Some(Value::String(text)) if parse_cell_id(text).is_some() => {
                CellId::from(text.as_str())
            }
            Some(_) => return Err(malformed("`cell` is not a cell ID")),
            None => return Err(malformed("`cell` is missing")),
        };
        let boot_session = match entries.get("boot_session") {
            Some(Value::String(text)) => BootSession::parse(text)
                .filter(|session| session.to_string() == *text)
                .ok_or_else(|| malformed("`boot_session` is not a lowercase hyphenated UUID"))?,
            Some(_) => return Err(malformed("`boot_session` is not a string")),
            None => return Err(malformed("`boot_session` is missing")),
        };
        let stopped = match entries.get("stopped") {
            None => false,
            Some(Value::Bool(true)) => true,
            Some(_) => return Err(malformed("`stopped` is present but not true")),
        };
        Ok(LaunchedMarker {
            cell,
            boot_session,
            stopped,
        })
    }
}

/// What [`CellDir::read_launched`] found. `Absent` means only that no entry named `launched`
/// exists.
#[derive(Debug)]
pub enum MarkerRead {
    Absent,
    Present(LaunchedMarker),
    Unreadable(MarkerFault),
}

/// Why an entry named `launched` is not its cell's marker. `WrongCell` is a well-formed marker
/// that names `found` instead.
#[derive(Debug)]
pub enum MarkerFault {
    Symlink,
    NotRegular,
    TooLarge,
    Io(io::Error),
    Malformed(String),
    WrongCell { found: CellId },
}

/// Why a cell was not retired.
#[derive(Debug)]
pub enum RetireError {
    ForeignLock { path: PathBuf },
    RetiredNotADirectory { path: PathBuf },
    AlreadyRetired { path: PathBuf },
    Moved { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
}

/// A journal opened for reading and appending through one descriptor. `created` is true when
/// this open created the file: its directory entry is not yet durable until the caller syncs
/// the file and then the cell directory with [`CellDir::sync`]. `created: false` does not prove
/// the entry is durable: an earlier open may have created the file and failed before that sync,
/// so a caller resuming an existing journal also syncs the cell directory once before it acts
/// on the journal.
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
/// not be opened. It was then never validated, so no path under it may be used, including its
/// supervisor socket.
#[derive(Debug)]
pub enum JournalRefusal {
    CellDirectory(io::Error),
    Symlink,
    NotRegular,
    Io(io::Error),
    ForeignLock { path: PathBuf },
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
    OutOfDescriptors { path: PathBuf, source: io::Error },
    ForeignLock { path: PathBuf },
}

/// Why a cell directory could not be opened or created.
#[derive(Debug)]
pub enum CellDirError {
    InvalidCell(CellPathError),
    Missing { path: PathBuf },
    Alias { path: PathBuf },
    Symlink { path: PathBuf },
    NotADirectory { path: PathBuf },
    AlreadyExists { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
    ForeignLock { path: PathBuf },
    NotARegularFile { path: PathBuf },
    MarkerUnreadable { path: PathBuf, fault: MarkerFault },
    LockNotNamed { path: PathBuf },
}

/// Why an instance root could not be opened.
#[derive(Debug)]
pub enum InstanceRootError {
    NotAbsolute {
        path: PathBuf,
    },
    /// The walk refused `root`'s path. `refusal` names the component that broke the rule.
    Untrusted {
        root: PathBuf,
        refusal: WalkRefusal,
    },
    Open {
        /// The root, or the directory above it that is missing or this user cannot search. On
        /// Linux the walk opens a directory this user cannot search, so this names the entry
        /// below it.
        path: PathBuf,
        source: io::Error,
    },
}

/// Why the writer lock was not taken. `Busy` means another open of the lock file holds it.
#[derive(Debug)]
pub enum LockError {
    Busy { path: PathBuf },
    Symlink { path: PathBuf },
    NotRegular { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
}

fn symlink_text(path: &Path) -> String {
    format!("{} is a symlink and was not followed", path.display())
}

fn foreign_lock_text(path: &Path) -> String {
    format!(
        "the writer lock {} belongs to another instance root",
        path.display()
    )
}

impl fmt::Display for JournalRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalRefusal::CellDirectory(source) => {
                write!(f, "the cell directory could not be opened: {source}")
            }
            JournalRefusal::Symlink => f.write_str("the journal is a symlink and was not followed"),
            JournalRefusal::NotRegular => f.write_str("the journal is not a regular file"),
            JournalRefusal::Io(source) => write!(f, "I/O failed on the journal: {source}"),
            JournalRefusal::ForeignLock { path } => f.write_str(&foreign_lock_text(path)),
        }
    }
}

impl std::error::Error for JournalRefusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            JournalRefusal::CellDirectory(source) | JournalRefusal::Io(source) => Some(source),
            JournalRefusal::Symlink
            | JournalRefusal::NotRegular
            | JournalRefusal::ForeignLock { .. } => None,
        }
    }
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiscoveryError::CellsSymlink { path } => f.write_str(&symlink_text(path)),
            DiscoveryError::CellsNotADirectory { path } => {
                write!(f, "{} is not a directory", path.display())
            }
            DiscoveryError::Open { path, source } => {
                write!(f, "cannot open {}: {source}", path.display())
            }
            DiscoveryError::Listing { path, source } => {
                write!(f, "cannot list {}: {source}", path.display())
            }
            DiscoveryError::Classify { path, source } => {
                write!(f, "cannot inspect {}: {source}", path.display())
            }
            DiscoveryError::OutOfDescriptors { path, source } => {
                write!(f, "out of file descriptors at {}: {source}", path.display())
            }
            DiscoveryError::ForeignLock { path } => f.write_str(&foreign_lock_text(path)),
        }
    }
}

impl std::error::Error for DiscoveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DiscoveryError::Open { source, .. }
            | DiscoveryError::Listing { source, .. }
            | DiscoveryError::Classify { source, .. }
            | DiscoveryError::OutOfDescriptors { source, .. } => Some(source),
            DiscoveryError::CellsSymlink { .. }
            | DiscoveryError::CellsNotADirectory { .. }
            | DiscoveryError::ForeignLock { .. } => None,
        }
    }
}

impl fmt::Display for CellDirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CellDirError::InvalidCell(error) => write!(f, "cannot use this cell ID: {error}"),
            CellDirError::Missing { path } => write!(f, "{} does not exist", path.display()),
            CellDirError::Alias { path } => write!(
                f,
                "{} names a cell whose directory is spelled differently on disk",
                path.display()
            ),
            CellDirError::Symlink { path } => f.write_str(&symlink_text(path)),
            CellDirError::NotADirectory { path } => {
                write!(f, "{} is not a directory", path.display())
            }
            CellDirError::AlreadyExists { path } => {
                write!(f, "{} already exists and was not adopted", path.display())
            }
            CellDirError::Io { path, source } => {
                write!(f, "I/O failed at {}: {source}", path.display())
            }
            CellDirError::ForeignLock { path } => f.write_str(&foreign_lock_text(path)),
            CellDirError::NotARegularFile { path } => {
                write!(f, "{} is not a regular file", path.display())
            }
            CellDirError::MarkerUnreadable { path, fault } => {
                write!(f, "{} cannot be read: {fault}", path.display())
            }
            CellDirError::LockNotNamed { path } => write!(
                f,
                "{} names a file other than the one this call locked, so no lock was returned",
                path.display()
            ),
        }
    }
}

impl std::error::Error for CellDirError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CellDirError::InvalidCell(error) => Some(error),
            CellDirError::Io { source, .. } => Some(source),
            CellDirError::MarkerUnreadable { fault, .. } => Some(fault),
            CellDirError::Missing { .. }
            | CellDirError::Alias { .. }
            | CellDirError::Symlink { .. }
            | CellDirError::NotADirectory { .. }
            | CellDirError::AlreadyExists { .. }
            | CellDirError::ForeignLock { .. }
            | CellDirError::NotARegularFile { .. }
            | CellDirError::LockNotNamed { .. } => None,
        }
    }
}

impl fmt::Display for MarkerFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MarkerFault::Symlink => f.write_str("the marker is a symlink and was not followed"),
            MarkerFault::NotRegular => f.write_str("the marker is not a regular file"),
            MarkerFault::TooLarge => {
                write!(f, "the marker is larger than {MAX_MARKER_BYTES} bytes")
            }
            MarkerFault::Io(source) => write!(f, "I/O failed on the marker: {source}"),
            MarkerFault::Malformed(reason) => write!(f, "the marker is malformed: {reason}"),
            MarkerFault::WrongCell { found } => write!(f, "the marker names cell {found}"),
        }
    }
}

impl std::error::Error for MarkerFault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MarkerFault::Io(source) => Some(source),
            MarkerFault::Symlink
            | MarkerFault::NotRegular
            | MarkerFault::TooLarge
            | MarkerFault::Malformed(_)
            | MarkerFault::WrongCell { .. } => None,
        }
    }
}

impl fmt::Display for RetireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RetireError::ForeignLock { path } => f.write_str(&foreign_lock_text(path)),
            RetireError::RetiredNotADirectory { path } => write!(
                f,
                "{} is a symlink or not a directory, so nothing was retired",
                path.display()
            ),
            RetireError::AlreadyRetired { path } => {
                write!(f, "{} already exists and was not replaced", path.display())
            }
            RetireError::Moved { path } => write!(
                f,
                "{} no longer names the cell directory that was opened",
                path.display()
            ),
            RetireError::Io { path, source } => {
                write!(f, "retirement failed at {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for RetireError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RetireError::Io { source, .. } => Some(source),
            RetireError::ForeignLock { .. }
            | RetireError::RetiredNotADirectory { .. }
            | RetireError::AlreadyRetired { .. }
            | RetireError::Moved { .. } => None,
        }
    }
}

impl fmt::Display for AllocateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AllocateError::ForeignLock { path } => f.write_str(&foreign_lock_text(path)),
            AllocateError::Symlink { path } => {
                write!(f, "{}, so no cell was allocated", symlink_text(path))
            }
            AllocateError::NotADirectory { path } => write!(
                f,
                "{} is not a directory, so no cell was allocated",
                path.display()
            ),
            AllocateError::Listing { path, source } => {
                write!(f, "cannot list {}: {source}", path.display())
            }
            AllocateError::Exhausted { path } => write!(
                f,
                "no cell ID is left: cell-{MAX_CELL_NUMBER} is already taken in {}",
                path.display()
            ),
            AllocateError::Contended { path } => write!(
                f,
                "no cell was allocated: each of {ALLOCATION_ATTEMPTS} attempts found the name it \
                 chose already taken in {}",
                path.display()
            ),
            AllocateError::Create { cell, source } => {
                write!(f, "creating {} failed: {source}", cell.as_str())
            }
        }
    }
}

impl std::error::Error for AllocateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AllocateError::Listing { source, .. } => Some(source),
            AllocateError::Create { source, .. } => Some(source),
            AllocateError::ForeignLock { .. }
            | AllocateError::Symlink { .. }
            | AllocateError::NotADirectory { .. }
            | AllocateError::Exhausted { .. }
            | AllocateError::Contended { .. } => None,
        }
    }
}

impl fmt::Display for InstanceRootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstanceRootError::NotAbsolute { path } => {
                write!(f, "{} is not an absolute path", path.display())
            }
            InstanceRootError::Untrusted { root, refusal } => write!(
                f,
                "the instance root {} is refused: {refusal}",
                root.display()
            ),
            InstanceRootError::Open { path, source } => write!(
                f,
                "cannot open {}, the instance root or a directory above it: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for InstanceRootError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            InstanceRootError::Untrusted { refusal, .. } => Some(refusal),
            InstanceRootError::Open { source, .. } => Some(source),
            InstanceRootError::NotAbsolute { .. } => None,
        }
    }
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Busy { path } => write!(f, "{} is held by another writer", path.display()),
            LockError::Symlink { path } => f.write_str(&symlink_text(path)),
            LockError::NotRegular { path } => {
                write!(f, "{} is not a regular file", path.display())
            }
            LockError::Io { path, source } => {
                write!(f, "the writer lock failed at {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for LockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LockError::Io { source, .. } => Some(source),
            LockError::Busy { .. } | LockError::Symlink { .. } | LockError::NotRegular { .. } => {
                None
            }
        }
    }
}

fn c_name(name: &[u8]) -> io::Result<CString> {
    CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

fn name_of(name: &CStr) -> &OsStr {
    OsStr::from_bytes(name.to_bytes())
}

fn cell_dir_io(path: &Path) -> impl FnOnce(io::Error) -> CellDirError + '_ {
    move |source| CellDirError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn retire_io(path: &Path) -> impl FnOnce(io::Error) -> RetireError + '_ {
    move |source| RetireError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[allow(unsafe_code)]
fn open_at(
    dir: BorrowedFd<'_>,
    name: &CStr,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> io::Result<OwnedFd> {
    // SAFETY: `name` is a CStr that outlives the call, and the BorrowedFd keeps `dir` open for it.
    // openat keeps no pointer to the name.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, mode as libc::c_uint) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a non-negative descriptor just above, which nothing else owns, so
    // this OwnedFd becomes its only owner.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn listed_cell_number(name: &[u8]) -> Option<u64> {
    let mut lowered = name.to_vec();
    lowered
        .get_mut(..CELL_ID_PREFIX.len())?
        .make_ascii_lowercase();
    std::str::from_utf8(&lowered).ok().and_then(parse_cell_id)
}

fn parse_cell_name(raw: &[u8]) -> Option<CellId> {
    let cell = CellId::from(std::str::from_utf8(raw).ok()?);
    validate_cell_id(&cell).ok()?;
    Some(cell)
}

fn collect_names(names: impl Iterator<Item = io::Result<Vec<u8>>>) -> io::Result<Vec<Vec<u8>>> {
    names.collect()
}

fn lists_exactly(dir: BorrowedFd<'_>, name: &[u8]) -> io::Result<bool> {
    for listed in DirectoryStream::open(dir)? {
        if listed? == name {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Debug, PartialEq, Eq)]
enum Naming {
    Exact,
    Alias,
    Gone,
}

fn naming(cells: BorrowedFd<'_>, name: &CStr, opened: BorrowedFd<'_>) -> io::Result<Naming> {
    let listed = lists_exactly(cells, name.to_bytes())?;
    let entry = match stat_at(cells, name) {
        Err(error) if error.raw_os_error() == Some(libc::ENOENT) => return Ok(Naming::Gone),
        entry => entry?,
    };
    let opened = stat_of(opened)?;
    if (entry.st_dev, entry.st_ino) != (opened.st_dev, opened.st_ino) {
        Ok(Naming::Gone)
    } else if listed {
        Ok(Naming::Exact)
    } else {
        Ok(Naming::Alias)
    }
}

fn out_of_descriptors(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::EMFILE | libc::ENFILE))
}

fn root_refusal(root: &Path, refusal: WalkRefusal) -> InstanceRootError {
    match refusal {
        WalkRefusal::Missing { path } => InstanceRootError::Open {
            path,
            source: io::Error::from_raw_os_error(libc::ENOENT),
        },
        WalkRefusal::Io { path, errno, .. } => InstanceRootError::Open {
            path,
            source: io::Error::from_raw_os_error(errno),
        },
        refusal => InstanceRootError::Untrusted {
            root: root.to_path_buf(),
            refusal,
        },
    }
}

enum DirectoryRefusal {
    Missing,
    Symlink,
    NotADirectory,
    Io(io::Error),
}

impl DirectoryRefusal {
    fn into_cell_dir_error(self, path: PathBuf) -> CellDirError {
        match self {
            DirectoryRefusal::Symlink => CellDirError::Symlink { path },
            DirectoryRefusal::NotADirectory => CellDirError::NotADirectory { path },
            DirectoryRefusal::Missing => CellDirError::Missing { path },
            DirectoryRefusal::Io(source) => CellDirError::Io { path, source },
        }
    }
}

fn directory_refusal(parent: BorrowedFd<'_>, name: &CStr, error: io::Error) -> DirectoryRefusal {
    match error.raw_os_error() {
        Some(libc::ENOENT) => DirectoryRefusal::Missing,
        Some(libc::ELOOP) => DirectoryRefusal::Symlink,
        Some(libc::ENOTDIR) => match kind_at(parent, name) {
            Ok(Kind::Symlink) => DirectoryRefusal::Symlink,
            _ => DirectoryRefusal::NotADirectory,
        },
        _ => DirectoryRefusal::Io(error),
    }
}

fn open_private_directory(
    parent: BorrowedFd<'_>,
    name: &CStr,
) -> Result<OwnedFd, DirectoryRefusal> {
    let opened = match open_at(parent, name, DIRECTORY_FLAGS, 0) {
        Err(error)
            if error.raw_os_error() == Some(libc::EACCES)
                && kind_at(parent, name).ok() == Some(Kind::Directory) =>
        {
            change_mode_at(parent, name, PRIVATE_DIRECTORY)
                .and_then(|()| open_at(parent, name, DIRECTORY_FLAGS, 0))
        }
        opened => opened,
    };
    let dir = opened.map_err(|error| directory_refusal(parent, name, error))?;
    change_mode(dir.as_fd(), PRIVATE_DIRECTORY).map_err(DirectoryRefusal::Io)?;
    Ok(dir)
}

fn open_journal_at(
    cell: BorrowedFd<'_>,
    access: libc::c_int,
    mode: libc::mode_t,
) -> Result<File, JournalRefusal> {
    let name = c_name(CELL_JOURNAL_FILE.as_bytes()).map_err(JournalRefusal::Io)?;
    let fd =
        open_at(cell, &name, access | FILE_FLAGS, mode).map_err(|error| {
            match file_refusal(cell, &name, error) {
                FileRefusal::Symlink => JournalRefusal::Symlink,
                FileRefusal::NotRegular => JournalRefusal::NotRegular,
                FileRefusal::Io(error) => JournalRefusal::Io(error),
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
    Regular,
    Other,
}

enum FileRefusal {
    Symlink,
    NotRegular,
    Io(io::Error),
}

impl FileRefusal {
    fn into_cell_dir_error(self, path: PathBuf) -> CellDirError {
        match self {
            FileRefusal::Symlink => CellDirError::Symlink { path },
            FileRefusal::NotRegular => CellDirError::NotARegularFile { path },
            FileRefusal::Io(source) => CellDirError::Io { path, source },
        }
    }
}

fn open_regular_at(
    dir: BorrowedFd<'_>,
    name: &CStr,
    access: libc::c_int,
    mode: libc::mode_t,
) -> Result<File, FileRefusal> {
    let fd = open_at(dir, name, access | FILE_FLAGS, mode)
        .map_err(|error| file_refusal(dir, name, error))?;
    let file = File::from(fd);
    if !file.metadata().map_err(FileRefusal::Io)?.is_file() {
        return Err(FileRefusal::NotRegular);
    }
    Ok(file)
}

fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let metadata = file.metadata()?;
    Ok(FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}

fn file_refusal(parent: BorrowedFd<'_>, name: &CStr, error: io::Error) -> FileRefusal {
    if error.raw_os_error() == Some(libc::ELOOP) {
        return FileRefusal::Symlink;
    }
    match kind_at(parent, name) {
        Ok(Kind::Symlink) => FileRefusal::Symlink,
        Ok(Kind::Directory | Kind::Other) => FileRefusal::NotRegular,
        Ok(Kind::Regular) | Err(_) => FileRefusal::Io(error),
    }
}

#[allow(unsafe_code)]
fn stat_at(dir: BorrowedFd<'_>, name: &CStr) -> io::Result<libc::stat> {
    let mut status = MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `name` is a CStr that outlives the call, the BorrowedFd keeps `dir` open for it, and
    // `status` points to writable space for one libc::stat.
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
    // SAFETY: fstatat returned 0 above, so it filled every field of `status`.
    Ok(unsafe { status.assume_init() })
}

#[allow(unsafe_code)]
fn stat_of(fd: BorrowedFd<'_>) -> io::Result<libc::stat> {
    let mut status = MaybeUninit::<libc::stat>::uninit();
    // SAFETY: The BorrowedFd keeps `fd` open for the call, and `status` points to writable space
    // for one libc::stat.
    if unsafe { libc::fstat(fd.as_raw_fd(), status.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fstat returned 0 above, so it filled every field of `status`.
    Ok(unsafe { status.assume_init() })
}

fn kind_at(dir: BorrowedFd<'_>, name: &CStr) -> io::Result<Kind> {
    let format = stat_at(dir, name)?.st_mode & libc::S_IFMT;
    Ok(match format {
        libc::S_IFDIR => Kind::Directory,
        libc::S_IFLNK => Kind::Symlink,
        libc::S_IFREG => Kind::Regular,
        _ => Kind::Other,
    })
}

#[allow(unsafe_code)]
fn clear_nonblocking(file: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: fcntl with F_GETFL takes no pointer, and the BorrowedFd keeps `file` open for the
    // call.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl with F_SETFL takes an integer flag set, not a pointer, and the BorrowedFd keeps
    // `file` open for the call.
    let set = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) };
    if set < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) struct DirectoryStream(NonNull<libc::DIR>);

impl DirectoryStream {
    /// Yields every name in `dir` from the start, without `.` and `..`, however much of `dir`
    /// an earlier stream read.
    ///
    /// The stream reads a duplicate of `dir` and rewinds it when it opens. The duplicate shares
    /// the descriptor's offset, so two streams on one descriptor must not be alive at the same
    /// time.
    #[allow(unsafe_code)]
    pub(crate) fn open(dir: BorrowedFd<'_>) -> io::Result<DirectoryStream> {
        let owned = dir.try_clone_to_owned()?;
        // SAFETY: `owned` is a descriptor this function owns. On success fdopendir takes it over,
        // and into_raw_fd below gives up the OwnedFd, so only closedir closes it. On failure the
        // OwnedFd still owns it and closes it.
        let stream = unsafe { libc::fdopendir(owned.as_raw_fd()) };
        let stream = NonNull::new(stream).ok_or_else(io::Error::last_os_error)?;
        let _owned_by_stream = owned.into_raw_fd();
        // SAFETY: `stream` is the non-null stream fdopendir just returned. Only closedir in Drop
        // closes it, so it is still open here.
        unsafe { libc::rewinddir(stream.as_ptr()) };
        Ok(DirectoryStream(stream))
    }
}

impl Iterator for DirectoryStream {
    type Item = io::Result<Vec<u8>>;

    #[allow(unsafe_code)]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            clear_errno();
            // SAFETY: `self.0` is the non-null stream fdopendir returned, which only closedir in
            // Drop releases.
            let entry = unsafe { libc::readdir(self.0.as_ptr()) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                return (error.raw_os_error() != Some(0)).then_some(Err(error));
            }
            // SAFETY: readdir returned a non-null entry, which stays valid until the next readdir
            // or closedir on this stream. Its d_name is NUL-terminated, and to_vec copies the bytes
            // before either can run.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name != b"." && name != b".." {
                return Some(Ok(name.to_vec()));
            }
        }
    }
}

impl Drop for DirectoryStream {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: `self.0` came from fdopendir, and Drop runs once, so this closes the stream
        // exactly once.
        unsafe { libc::closedir(self.0.as_ptr()) };
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn clear_errno() {
    // SAFETY: __error returns a valid pointer to this thread's errno, which the thread may write.
    unsafe { *libc::__error() = 0 };
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn clear_errno() {
    // SAFETY: __errno_location returns a valid pointer to this thread's errno, which the thread may
    // write.
    unsafe { *libc::__errno_location() = 0 };
}

#[allow(unsafe_code)]
fn make_directory_at(parent: BorrowedFd<'_>, name: &CStr) -> io::Result<()> {
    // SAFETY: `name` is a CStr that outlives the call, and the BorrowedFd keeps `parent` open for
    // it. mkdirat keeps no pointer to the name.
    let made = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), PRIVATE_DIRECTORY) };
    if made != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
fn remove_at(dir: BorrowedFd<'_>, name: &CStr) -> io::Result<()> {
    // SAFETY: `name` is a CStr that outlives the call, and the BorrowedFd keeps `dir` open for
    // it. unlinkat keeps no pointer to the name.
    let removed = unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
    if removed != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
fn rename_at(dir: BorrowedFd<'_>, from: &CStr, to: &CStr) -> io::Result<()> {
    // SAFETY: both names are CStrs that outlive the call, and the BorrowedFd keeps `dir` open
    // for it. renameat keeps no pointer to either name.
    let renamed =
        unsafe { libc::renameat(dir.as_raw_fd(), from.as_ptr(), dir.as_raw_fd(), to.as_ptr()) };
    if renamed != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub(crate) fn rename_no_replace(
    from: BorrowedFd<'_>,
    from_name: &CStr,
    to: BorrowedFd<'_>,
    to_name: &CStr,
) -> io::Result<()> {
    // SAFETY: both names are CStrs that outlive the call, and the two BorrowedFds keep both
    // directories open for it. renameatx_np keeps no pointer to either name, and RENAME_EXCL
    // makes it fail with EEXIST rather than replace an existing entry.
    let renamed = unsafe {
        libc::renameatx_np(
            from.as_raw_fd(),
            from_name.as_ptr(),
            to.as_raw_fd(),
            to_name.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if renamed != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub(crate) fn rename_no_replace(
    from: BorrowedFd<'_>,
    from_name: &CStr,
    to: BorrowedFd<'_>,
    to_name: &CStr,
) -> io::Result<()> {
    // SAFETY: both names are CStrs that outlive the call, and the two BorrowedFds keep both
    // directories open for it. renameat2 keeps no pointer to either name, and RENAME_NOREPLACE
    // makes it fail with EEXIST rather than replace an existing entry.
    let renamed = unsafe {
        libc::renameat2(
            from.as_raw_fd(),
            from_name.as_ptr(),
            to.as_raw_fd(),
            to_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if renamed != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
fn change_mode(fd: BorrowedFd<'_>, mode: libc::mode_t) -> io::Result<()> {
    // SAFETY: fchmod takes no pointer, and the BorrowedFd keeps `fd` open for the call.
    let changed = unsafe { libc::fchmod(fd.as_raw_fd(), mode) };
    if changed != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
fn change_mode_at(dir: BorrowedFd<'_>, name: &CStr, mode: libc::mode_t) -> io::Result<()> {
    // SAFETY: `name` is a CStr that outlives the call, and the BorrowedFd keeps `dir` open for
    // it. fchmodat keeps no pointer to the name, and AT_SYMLINK_NOFOLLOW makes it act on a
    // symlink itself, or refuse it, never on the symlink's target.
    let changed = unsafe {
        libc::fchmodat(
            dir.as_raw_fd(),
            name.as_ptr(),
            mode,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if changed != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
fn lock_exclusive_nonblocking(file: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: flock takes no pointer, and the BorrowedFd keeps `file` open for the call.
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
    use std::cell::{Cell, RefCell};
    use std::error::Error;
    use std::ffi::CString;
    use std::fs;
    use std::io::{self, Read, Write};
    use std::os::fd::{AsRawFd, BorrowedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime};

    use tempfile::TempDir;

    use super::*;
    #[cfg(target_os = "macos")]
    use crate::nofollow::fixtures::add_acl;
    use crate::nofollow::fixtures::{TestRoot, make_dir};
    use crate::private_socket::{PrivateDir, PrivateSocketError};

    macro_rules! expect_match {
        ($value:expr, $pattern:pat => $then:expr) => {
            match $value {
                $pattern => $then,
                other => panic!("expected {}, got {other:?}", stringify!($pattern)),
            }
        };
    }

    const DEADLINE: Duration = Duration::from_secs(5);

    fn temp_root() -> (TempDir, PathBuf) {
        crate::nofollow::fixtures::walkable_temp_dir("in")
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

    #[allow(unsafe_code)]
    fn a_separate_open_can_lock(path: &Path) -> bool {
        let file = fs::File::open(path).expect("the file opens for a lock attempt");
        // SAFETY: flock takes no pointer, and `file` stays open until this function returns.
        let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        taken == 0
    }

    #[allow(unsafe_code)]
    fn a_separate_open_can_share(path: &Path) -> bool {
        let file = fs::File::open(path).expect("the file opens for a lock attempt");
        // SAFETY: flock takes no pointer, and `file` stays open until this function returns.
        let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) };
        taken == 0
    }

    #[allow(unsafe_code)]
    fn make_fifo(path: &Path) {
        let name = CString::new(path.as_os_str().as_bytes()).expect("a path without NUL");
        // SAFETY: `name` is a CString that lives until this function returns, and mkfifo keeps no
        // pointer to it.
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

    struct RestoreMode(PathBuf);

    impl Drop for RestoreMode {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }

    fn restrict(path: &Path, mode: u32) -> RestoreMode {
        set_mode(path, mode);
        RestoreMode(path.to_path_buf())
    }

    #[allow(unsafe_code)]
    fn assert_not_root() {
        assert_ne!(
            // SAFETY: geteuid takes no arguments and cannot fail.
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
            JournalOpen::Refused(JournalRefusal::ForeignLock { .. }) => "refused: foreign lock",
        }
    }

    fn locked(root: &Path) -> (InstanceRoot, WriterLock) {
        let instance = open_root(root);
        let lock = instance.lock().expect("the writer lock is taken");
        (instance, lock)
    }

    fn try_discover(root: &Path) -> Result<Discovery, DiscoveryError> {
        let (instance, lock) = locked(root);
        instance.discover(&lock)
    }

    fn discover(root: &Path) -> Discovery {
        try_discover(root).expect("discovery succeeds")
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
        expect_match!(
            InstanceRoot::open(Path::new("relative/root")),
            Err(InstanceRootError::NotAbsolute { path }) => {
                assert_eq!(path, PathBuf::from("relative/root"))
            }
        );
    }

    #[test]
    fn a_missing_root_is_an_open_error_naming_it() {
        let (_dir, root) = temp_root();
        let missing = root.join("missing");
        expect_match!(InstanceRoot::open(&missing), Err(InstanceRootError::Open { path, source }) => {
            assert_eq!(path, missing);
            assert_eq!(source.kind(), io::ErrorKind::NotFound);
        });
    }

    struct Refused {
        open: PathBuf,
        real: Option<PathBuf>,
        refusal: WalkRefusal,
        socket_check: bool,
    }

    fn refused_at(open: PathBuf, real: Option<PathBuf>, refusal: WalkRefusal) -> Refused {
        Refused {
            open,
            real,
            refusal,
            socket_check: true,
        }
    }

    fn entries_of(path: &Path) -> Vec<PathBuf> {
        let mut entries: Vec<PathBuf> = fs::read_dir(path)
            .expect("the directory lists")
            .map(|entry| entry.expect("an entry").path())
            .collect();
        entries.sort();
        entries
    }

    fn make_cell_dir_by_hand(root: &Path) -> PathBuf {
        make_dir(&root.join("cells"), 0o700);
        make_dir(&root.join("cells/cell-1"), 0o700);
        root.join("cells/cell-1")
    }

    fn assert_refused_where_its_socket_would_be(row: &str, refused: Refused) {
        let Refused {
            open,
            real,
            refusal,
            socket_check,
        } = refused;
        match InstanceRoot::open(&open) {
            Err(InstanceRootError::Untrusted {
                root,
                refusal: found,
            }) => {
                assert_eq!(root, open, "{row}");
                assert_eq!(found, refusal, "{row}");
            }
            other => panic!("{row}: expected Untrusted, got {other:?}"),
        }
        if let Some(real) = &real {
            assert_eq!(entries_of(real), Vec::<PathBuf>::new(), "{row}");
            make_cell_dir_by_hand(real);
        }
        if socket_check {
            assert_eq!(
                PrivateDir::open(&open.join("cells/cell-1")).map(|dir| dir.path().to_path_buf()),
                Err(PrivateSocketError::from(refusal)),
                "{row}: membraned's socket check refuses the same component"
            );
        }
    }

    fn assert_opens_where_its_socket_would(row: &str, root: &Path) {
        let opened = InstanceRoot::open(root).unwrap_or_else(|error| panic!("{row}: {error}"));
        assert_eq!(opened.path(), root, "{row}");
        let cell = make_cell_dir_by_hand(root);
        PrivateDir::open(&cell).unwrap_or_else(|error| panic!("{row}: {error}"));
    }

    fn under_parent(base: &Path, mode: u32) -> Refused {
        let parent = base.join("p");
        make_dir(&parent, 0o700);
        make_dir(&parent.join("inst"), 0o700);
        set_mode(&parent, mode);
        let root = parent.join("inst");
        refused_at(
            root.clone(),
            Some(root),
            WalkRefusal::Replaceable { path: parent, mode },
        )
    }

    fn root_at(base: &Path, mode: u32) -> Refused {
        let root = base.join("inst");
        make_dir(&root, mode);
        refused_at(
            root.clone(),
            Some(root.clone()),
            WalkRefusal::Replaceable { path: root, mode },
        )
    }

    fn through_a_symlink(base: &Path, link: &str, target: &str, open: &str) -> Refused {
        make_dir(&base.join("real"), 0o700);
        make_dir(&base.join("real/inst"), 0o700);
        symlink(base.join(target), base.join(link)).expect("the link is made");
        refused_at(
            base.join(open),
            Some(base.join("real/inst")),
            WalkRefusal::Symlink {
                path: base.join(link),
            },
        )
    }

    fn with_a_trailing_slash(base: &Path) -> Refused {
        let root = base.join("inst");
        make_dir(&root, 0o700);
        let mut spelled = root.clone().into_os_string();
        spelled.push("/");
        let spelled = PathBuf::from(spelled);
        Refused {
            open: spelled.clone(),
            real: Some(root),
            refusal: WalkRefusal::NotNormal { path: spelled },
            socket_check: false,
        }
    }

    fn a_file(base: &Path) -> Refused {
        let root = base.join("inst");
        fs::write(&root, b"").expect("the file is written");
        refused_at(root.clone(), None, WalkRefusal::NotDirectory { path: root })
    }

    #[cfg(target_os = "macos")]
    fn under_parent_with_acl(base: &Path, rule: &str) -> Refused {
        let parent = base.join("p");
        make_dir(&parent, 0o755);
        make_dir(&parent.join("inst"), 0o700);
        add_acl(&parent, rule);
        let root = parent.join("inst");
        refused_at(
            root.clone(),
            Some(root),
            WalkRefusal::ReplaceableByAcl { path: parent },
        )
    }

    #[cfg(target_os = "macos")]
    fn root_with_acl(base: &Path) -> Refused {
        let root = base.join("inst");
        make_dir(&root, 0o700);
        add_acl(&root, "everyone allow add_subdirectory");
        refused_at(
            root.clone(),
            Some(root.clone()),
            WalkRefusal::ReplaceableByAcl { path: root },
        )
    }

    #[cfg(target_os = "macos")]
    fn under_a_foreign_owner(_: &Path) -> Refused {
        let foreign = Path::new("/private/var/at");
        let metadata = fs::symlink_metadata(foreign).expect("/private/var/at exists");
        let euid = crate::nofollow::effective_uid();
        assert!(
            metadata.is_dir() && metadata.uid() != 0 && metadata.uid() != euid,
            "{} must be a directory owned by neither root nor uid {euid} for this row",
            foreign.display()
        );
        refused_at(
            foreign.join("inst"),
            None,
            WalkRefusal::ForeignOwner {
                path: foreign.to_path_buf(),
                uid: metadata.uid(),
            },
        )
    }

    type Row = (&'static str, fn(&Path) -> Refused);

    #[test]
    fn a_root_is_refused_exactly_where_its_cell_socket_would_be() {
        let mut rows: Vec<Row> = vec![
            ("parent 0777", |base| under_parent(base, 0o777)),
            ("parent 0770", |base| under_parent(base, 0o770)),
            ("parent 0707", |base| under_parent(base, 0o707)),
            ("parent sticky 1777", |base| under_parent(base, 0o1777)),
            ("root 0770", |base| root_at(base, 0o770)),
            ("symlink above", |base| {
                through_a_symlink(base, "link", "real", "link/inst")
            }),
            ("symlink last", |base| {
                through_a_symlink(base, "last", "real/inst", "last")
            }),
            ("trailing slash", with_a_trailing_slash),
            ("a file", a_file),
        ];
        #[cfg(target_os = "macos")]
        rows.extend::<[Row; 6]>([
            ("add_subdirectory on the parent", |base| {
                under_parent_with_acl(base, "everyone allow add_subdirectory")
            }),
            ("delete_child on the parent", |base| {
                under_parent_with_acl(base, "everyone allow delete_child")
            }),
            ("add_file for staff on the parent", |base| {
                under_parent_with_acl(base, "staff allow add_file")
            }),
            ("inherit-only add_subdirectory on the parent", |base| {
                under_parent_with_acl(
                    base,
                    "everyone allow add_subdirectory,directory_inherit,only_inherit",
                )
            }),
            ("add_subdirectory on the root", root_with_acl),
            ("a foreign owner above", under_a_foreign_owner),
        ]);
        for (row, setup) in rows {
            let base = TestRoot::walkable_from_root();
            assert_refused_where_its_socket_would_be(row, setup(base.path()));
        }
        let base = TestRoot::walkable_from_root();
        make_dir(&base.join("inst"), 0o700);
        assert_opens_where_its_socket_would("clean", &base.join("inst"));
    }

    #[test]
    fn a_root_in_the_system_temp_directory_is_refused() {
        #[cfg(target_os = "macos")]
        let sticky = Path::new("/private/tmp");
        #[cfg(target_os = "linux")]
        let sticky = Path::new("/tmp");
        let held = tempfile::Builder::new()
            .prefix("ir")
            .tempdir_in(sticky)
            .expect("a directory in the sticky temp directory");
        set_mode(held.path(), 0o700);
        let root = held.path().join("inst");
        make_dir(&root, 0o700);
        assert_refused_where_its_socket_would_be(
            "the sticky temp directory",
            refused_at(
                root.clone(),
                Some(root),
                WalkRefusal::Replaceable {
                    path: sticky.to_path_buf(),
                    mode: 0o1777,
                },
            ),
        );
        #[cfg(target_os = "macos")]
        {
            let root = held.path().join("through-tmp");
            make_dir(&root, 0o700);
            let name = held.path().file_name().expect("the directory has a name");
            assert_refused_where_its_socket_would_be(
                "spelled through /tmp",
                refused_at(
                    Path::new("/tmp").join(name).join("through-tmp"),
                    Some(root),
                    WalkRefusal::Symlink {
                        path: PathBuf::from("/tmp"),
                    },
                ),
            );
        }
    }

    #[test]
    fn a_root_under_a_parent_that_replaces_nothing_opens() {
        let base = TestRoot::walkable_from_root();
        let parent = base.join("p");
        make_dir(&parent, 0o711);
        make_dir(&parent.join("inst"), 0o700);
        assert_opens_where_its_socket_would("search-only parent", &parent.join("inst"));
        #[cfg(target_os = "macos")]
        {
            let base = TestRoot::walkable_from_root();
            let parent = base.join("p");
            make_dir(&parent, 0o755);
            make_dir(&parent.join("inst"), 0o700);
            add_acl(&parent, "everyone deny delete");
            add_acl(&parent, "everyone allow list,search");
            assert_opens_where_its_socket_would(
                "a parent that denies delete and allows list and search",
                &parent.join("inst"),
            );
        }
    }

    #[test]
    fn a_missing_or_unsearchable_ancestor_is_an_open_error_naming_it() {
        let base = TestRoot::walkable_from_root();
        let absent = base.join("absent");
        expect_match!(InstanceRoot::open(&absent.join("inst")), Err(InstanceRootError::Open { path, source }) => {
            assert_eq!(path, absent);
            assert_eq!(source.raw_os_error(), Some(libc::ENOENT));
        });
        if crate::nofollow::effective_uid() == 0 {
            println!("skipped the unsearchable case: root searches every directory");
            return;
        }
        let locked = base.join("locked");
        make_dir(&locked, 0o700);
        make_dir(&locked.join("inst"), 0o700);
        set_mode(&locked, 0o000);
        let outcome = InstanceRoot::open(&locked.join("inst"));
        set_mode(&locked, 0o700);
        #[cfg(target_os = "macos")]
        let unopened = locked.clone();
        #[cfg(target_os = "linux")]
        let unopened = locked.join("inst");
        expect_match!(outcome, Err(InstanceRootError::Open { path, source }) => {
            assert_eq!(path, unopened);
            assert_eq!(source.raw_os_error(), Some(libc::EACCES));
        });
        assert_eq!(entries_of(&locked.join("inst")), Vec::<PathBuf>::new());
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
    #[allow(unsafe_code)]
    fn the_root_and_lock_descriptors_are_close_on_exec() {
        let (_dir, root) = temp_root();
        let instance = open_root(&root);
        let lock = instance.lock().expect("the first lock is taken");
        for (what, fd) in [
            ("root", instance.dir.as_raw_fd()),
            ("lock", lock.file.as_raw_fd()),
        ] {
            // SAFETY: fcntl with F_GETFD takes no pointer, and `instance` and `lock` keep both
            // descriptors open through the loop.
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
        expect_match!(within("a contended lock", move || second.lock()), Err(LockError::Busy { path }) => assert_eq!(path, root.join("controller.lock")));
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
        expect_match!(open_root(&root).lock(), Err(LockError::Symlink { path }) => assert_eq!(path, root.join("controller.lock")));
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
        expect_match!(open_root(&root).lock(), Err(LockError::Symlink { path }) => assert_eq!(path, root.join("controller.lock")));
        assert!(
            fs::symlink_metadata(&target).is_err(),
            "the link target was created"
        );
    }

    #[test]
    fn a_directory_at_the_lock_path_is_refused() {
        let (_dir, root) = temp_root();
        fs::create_dir(root.join("controller.lock")).expect("the directory is made");
        expect_match!(open_root(&root).lock(), Err(LockError::NotRegular { path }) => assert_eq!(path, root.join("controller.lock")));
    }

    #[test]
    fn a_fifo_at_the_lock_path_is_refused_as_not_regular() {
        let (_dir, root) = temp_root();
        make_fifo(&root.join("controller.lock"));
        expect_match!(open_root(&root).lock(), Err(LockError::NotRegular { path }) => assert_eq!(path, root.join("controller.lock")));
    }

    #[test]
    fn a_socket_at_the_lock_path_is_refused_as_not_regular() {
        let (_dir, root) = temp_root();
        let _socket = UnixListener::bind(root.join("controller.lock")).expect("the socket binds");
        expect_match!(open_root(&root).lock(), Err(LockError::NotRegular { path }) => assert_eq!(path, root.join("controller.lock")));
    }

    #[test]
    fn a_lock_file_that_cannot_be_created_is_an_io_error() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let restore = restrict(&root, 0o500);
        let result = open_root(&root).lock();
        drop(restore);
        expect_match!(result, Err(LockError::Io { path, source }) => {
            assert_eq!(path, root.join("controller.lock"));
            assert_eq!(source.raw_os_error(), Some(libc::EACCES));
        });
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
        expect_match!(try_discover(&root), Err(DiscoveryError::CellsSymlink { path }) => assert_eq!(path, root.join("cells")));
    }

    #[test]
    fn a_regular_file_at_cells_aborts_discovery() {
        let (_dir, root) = temp_root();
        fs::write(root.join("cells"), b"").expect("the file is made");
        expect_match!(try_discover(&root), Err(DiscoveryError::CellsNotADirectory { path }) => {
            assert_eq!(path, root.join("cells"))
        });
    }

    #[test]
    fn an_unreadable_cells_directory_aborts_discovery() {
        assert_not_root();
        let (_dir, root) = temp_root();
        make_cell(&root, "cell-1", None);
        let restore = restrict(&root.join("cells"), 0o000);
        let result = try_discover(&root);
        drop(restore);
        expect_match!(result, Err(DiscoveryError::Open { path, source }) => {
            assert_eq!(path, root.join("cells"));
            assert_eq!(source.raw_os_error(), Some(libc::EACCES));
        });
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

    fn names_in(dir: BorrowedFd<'_>) -> Vec<Vec<u8>> {
        let mut names = DirectoryStream::open(dir)
            .and_then(collect_names)
            .expect("the directory lists");
        names.sort();
        names
    }

    #[test]
    fn a_second_stream_on_one_descriptor_lists_every_name_again() {
        let (_dir, root) = temp_root();
        for name in ["a", "b", "c"] {
            fs::write(root.join(name), b"").expect("write");
        }
        let held = File::open(&root).expect("open");
        let every = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        assert_eq!(names_in(held.as_fd()), every, "the first stream");
        assert_eq!(names_in(held.as_fd()), every, "the second stream");
    }

    #[test]
    fn a_stream_dropped_after_one_name_leaves_the_next_stream_every_name() {
        let (_dir, root) = temp_root();
        let long = "n".repeat(200);
        let mut every: Vec<Vec<u8>> = (0..400)
            .map(|index| format!("{index:03}{long}").into_bytes())
            .collect();
        for name in &every {
            fs::write(root.join(OsStr::from_bytes(name)), b"").expect("write");
        }
        every.sort();
        let held = File::open(&root).expect("open");
        let mut first = DirectoryStream::open(held.as_fd()).expect("the first stream opens");
        assert!(
            matches!(first.next(), Some(Ok(_))),
            "the first stream yields a name"
        );
        drop(first);
        let listed = names_in(held.as_fd());
        assert!(
            listed == every,
            "the next stream listed {} of {} names",
            listed.len(),
            every.len()
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
        let _restore = restrict(&locked, 0o000);
        symlink(&locked, root.join("cells/c-link")).expect("the link is made");
        fs::write(root.join("cells/d-file"), b"not a cell").expect("the file is made");
        fs::create_dir(root.join("cells/e\\bad")).expect("the bad name is made");

        let mut discovery = discover(&root);

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
    fn a_validly_named_symlink_to_a_cell_is_not_a_cell() {
        let (_dir, root) = temp_root();
        make_cell(&root.join("elsewhere"), "cell-1", Some(b"{}\n"));
        fs::create_dir(root.join("cells")).expect("cells is made");
        symlink(
            root.join("elsewhere/cells/cell-1"),
            root.join("cells/cell-1"),
        )
        .expect("the link is made");
        assert_eq!(described(&discover(&root)), ["not a cell: Symlink"]);
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
    fn a_journal_linked_to_a_readable_file_is_refused_as_a_symlink() {
        let (_dir, root) = temp_root();
        let cell = make_cell(&root, "cell-1", None);
        let target = root.join("elsewhere");
        fs::write(&target, b"{}\n").expect("the target is made");
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
        let (instance, lock) = locked(&root);
        let discovery = within("discovering a FIFO journal", move || {
            instance.discover(&lock)
        })
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
    fn an_unreadable_cell_directory_refuses_only_that_cell() {
        assert_not_root();
        let (_dir, root) = temp_root();
        make_cell(&root, "cell-a", Some(b"{}\n"));
        let locked = make_cell(&root, "cell-b", None);
        let restore = restrict(&locked, 0o000);
        let discovery = discover(&root);
        drop(restore);
        assert_eq!(
            described(&discovery),
            [
                "cell cell-a: regular",
                "cell cell-b: refused: cell directory"
            ]
        );
    }

    #[test]
    #[allow(unsafe_code)]
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
        // SAFETY: fcntl with F_GETFL takes no pointer, and `discovery` keeps `file` open for the
        // call.
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

    type BeforeSync = Box<dyn FnOnce()>;

    thread_local! {
        static SYNCED: RefCell<Vec<(u64, u64)>> = const { RefCell::new(Vec::new()) };
        static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };
        static FAIL_NEXT_SYNC_OF: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
        static FAIL_NEXT_FILE_SYNC: Cell<bool> = const { Cell::new(false) };
        static BEFORE_NEXT_FILE_SYNC: RefCell<Option<BeforeSync>> = const { RefCell::new(None) };
    }

    const WATCHED: [&CStr; 10] = [
        c"launched",
        c"launched.tmp",
        c"created",
        c"runtime",
        c"cell-1",
        c"retired",
        c"cells",
        c"controller.lock",
        c"supervisor.lock",
        c"supervisor.lock.tmp",
    ];

    #[derive(Debug, Clone)]
    struct Seen {
        synced: (u64, u64),
        bytes: Option<u64>,
        names: Vec<(&'static CStr, (u64, u64))>,
    }

    impl Seen {
        fn named(&self, name: &CStr) -> Option<(u64, u64)> {
            self.names
                .iter()
                .find(|(watched, _)| *watched == name)
                .map(|&(_, found)| found)
        }
    }

    fn identity_in(dir: BorrowedFd<'_>, name: &CStr) -> Option<(u64, u64)> {
        let entry = open_at(dir, name, libc::O_RDONLY | FILE_FLAGS, 0).ok()?;
        fs::File::from(entry)
            .metadata()
            .ok()
            .map(|metadata| identity(&metadata))
    }

    fn seen_by(fd: BorrowedFd<'_>, metadata: &fs::Metadata) -> Seen {
        let names = if metadata.is_dir() {
            WATCHED
                .iter()
                .filter_map(|&name| identity_in(fd, name).map(|found| (name, found)))
                .collect()
        } else {
            Vec::new()
        };
        Seen {
            synced: identity(metadata),
            bytes: metadata.is_file().then_some(metadata.len()),
            names,
        }
    }

    fn recording_sync(fd: BorrowedFd<'_>) -> io::Result<()> {
        let metadata = fs::File::from(fd.try_clone_to_owned()?).metadata()?;
        let synced = identity(&metadata);
        SYNCED.with(|all| all.borrow_mut().push(synced));
        SEEN.with(|all| all.borrow_mut().push(seen_by(fd, &metadata)));
        if FAIL_NEXT_SYNC_OF.with(|fail| fail.get()) == Some(synced) {
            FAIL_NEXT_SYNC_OF.with(|fail| fail.set(None));
            return Err(io::Error::from_raw_os_error(libc::EIO));
        }
        if metadata.is_file() {
            if FAIL_NEXT_FILE_SYNC.with(|fail| fail.replace(false)) {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            if let Some(work) = BEFORE_NEXT_FILE_SYNC.with(|slot| slot.borrow_mut().take()) {
                work();
            }
        }
        sync_directory(fd)
    }

    fn reset_syncs() {
        SYNCED.with(|all| all.borrow_mut().clear());
        SEEN.with(|all| all.borrow_mut().clear());
        FAIL_NEXT_SYNC_OF.with(|fail| fail.set(None));
        FAIL_NEXT_FILE_SYNC.with(|fail| fail.set(false));
        BEFORE_NEXT_FILE_SYNC.with(|slot| slot.borrow_mut().take());
    }

    fn last_seen(path: &Path) -> Seen {
        let synced = identity_at(path);
        SEEN.with(|all| {
            all.borrow()
                .iter()
                .rev()
                .find(|seen| seen.synced == synced)
                .cloned()
        })
        .unwrap_or_else(|| panic!("{} was never synced", path.display()))
    }

    fn fail_next_file_sync() {
        FAIL_NEXT_FILE_SYNC.with(|fail| fail.set(true));
    }

    fn before_next_file_sync(work: impl FnOnce() + 'static) {
        BEFORE_NEXT_FILE_SYNC.with(|slot| *slot.borrow_mut() = Some(Box::new(work)));
    }

    fn recording_root(root: &Path) -> InstanceRoot {
        reset_syncs();
        let mut instance = open_root(root);
        instance.sync = recording_sync;
        instance
    }

    fn recording_locked(root: &Path) -> (InstanceRoot, WriterLock) {
        let instance = recording_root(root);
        let lock = instance.lock().expect("the writer lock is taken");
        reset_syncs();
        (instance, lock)
    }

    fn syncs_of(path: &Path) -> usize {
        let directory = identity(&fs::metadata(path).expect("the synced directory exists"));
        SYNCED.with(|all| {
            all.borrow()
                .iter()
                .filter(|&&synced| synced == directory)
                .count()
        })
    }

    fn fail_next_sync_of(path: &Path) {
        let directory = identity(&fs::metadata(path).expect("the directory exists"));
        FAIL_NEXT_SYNC_OF.with(|fail| fail.set(Some(directory)));
    }

    fn assert_injected(source: &io::Error) {
        assert_eq!(source.raw_os_error(), Some(libc::EIO), "{source}");
    }

    fn appended(dir: &CellDir, lock: &WriterLock) -> JournalAppend {
        dir.open_journal_for_append(lock)
            .expect("the journal opens for appending")
    }

    #[test]
    fn create_cell_dir_creates_private_directories() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let created = instance
            .create_cell_dir(&lock, &cell("cell-1"))
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
        let (instance, lock) = locked(&root);
        let existing = make_cell(&root, "cell-1", None);
        set_mode(&existing, 0o755);
        expect_match!(instance.create_cell_dir(&lock, &cell("cell-1")), Err(CellDirError::AlreadyExists { path }) => assert_eq!(path, existing));
        assert_eq!(mode_of(&existing), 0o755, "an existing cell is not adopted");
    }

    #[test]
    fn create_cell_dir_refuses_a_symlinked_cells_directory() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let elsewhere = root.join("elsewhere");
        fs::create_dir(&elsewhere).expect("the link target is made");
        symlink(&elsewhere, root.join("cells")).expect("the link is made");
        expect_match!(instance.create_cell_dir(&lock, &cell("cell-1")), Err(CellDirError::Symlink { path }) => assert_eq!(path, root.join("cells")));
        assert_eq!(
            fs::read_dir(&elsewhere).expect("the target lists").count(),
            0,
            "nothing was created through the link"
        );
    }

    #[test]
    fn create_cell_dir_refuses_a_regular_file_at_cells() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        fs::write(root.join("cells"), b"").expect("the file is made");
        expect_match!(instance.create_cell_dir(&lock, &cell("cell-1")), Err(CellDirError::NotADirectory { path }) => assert_eq!(path, root.join("cells")));
    }

    #[test]
    fn create_cell_dir_reports_a_parent_it_cannot_write() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let restore = restrict(&root, 0o500);
        let without_cells = instance.create_cell_dir(&lock, &cell("cell-1"));
        drop(restore);
        expect_match!(without_cells, Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, root.join("cells"));
            assert_eq!(source.raw_os_error(), Some(libc::EACCES));
        });
    }

    #[test]
    fn create_cell_dir_names_the_cell_when_its_directory_cannot_be_made() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let long = "c".repeat(256);
        expect_match!(instance.create_cell_dir(&lock, &cell(&long)), Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, root.join("cells").join(&long));
            assert_eq!(source.raw_os_error(), Some(libc::ENAMETOOLONG));
        });
    }

    #[test]
    fn create_cell_dir_refuses_invalid_ids_before_creating_anything() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        expect_match!(instance.create_cell_dir(&lock, &cell("..")), Err(CellDirError::InvalidCell(CellPathError::NotACellName(text))) => {
            assert_eq!(text, "..")
        });
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
        expect_match!(open_root(&root).cell_dir(&cell("..")), Err(CellDirError::InvalidCell(CellPathError::NotACellName(text))) => {
            assert_eq!(text, "..")
        });
    }

    #[test]
    fn cell_dir_refuses_symlinks_and_non_directories() {
        let (_dir, root) = temp_root();
        make_cell(&root, "real", None);
        symlink(root.join("cells/real"), root.join("cells/linked")).expect("the link is made");
        fs::write(root.join("cells/file"), b"").expect("the file is made");
        let instance = open_root(&root);
        expect_match!(instance.cell_dir(&cell("linked")), Err(CellDirError::Symlink { path }) => assert_eq!(path, root.join("cells/linked")));
        expect_match!(instance.cell_dir(&cell("file")), Err(CellDirError::NotADirectory { path }) => {
            assert_eq!(path, root.join("cells/file"))
        });
        expect_match!(instance.cell_dir(&cell("missing")), Err(CellDirError::Missing { path }) => {
            assert_eq!(path, root.join("cells/missing"))
        });
    }

    #[test]
    fn cell_dir_reports_a_missing_cells_directory_as_missing() {
        let (_dir, root) = temp_root();
        expect_match!(open_root(&root).cell_dir(&cell("cell-1")), Err(CellDirError::Missing { path }) => {
            assert_eq!(path, root.join("cells"))
        });
    }

    #[test]
    fn cell_dir_reports_an_unreadable_cells_directory_as_io() {
        assert_not_root();
        let (_dir, root) = temp_root();
        make_cell(&root, "cell-1", None);
        let restore = restrict(&root.join("cells"), 0o000);
        let result = open_root(&root).cell_dir(&cell("cell-1"));
        drop(restore);
        expect_match!(result, Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, root.join("cells"));
            assert_eq!(source.raw_os_error(), Some(libc::EACCES));
        });
    }

    #[test]
    fn cell_dir_refuses_a_case_or_normalization_alias() {
        let (_dir, root) = temp_root();
        let composed: String = ['c', 'a', 'f', char::from_u32(0xe9).expect("U+00E9")]
            .into_iter()
            .collect();
        let decomposed: String = ['c', 'a', 'f', 'e', char::from_u32(0x301).expect("U+0301")]
            .into_iter()
            .collect();
        assert_ne!(composed.as_bytes(), decomposed.as_bytes());
        make_cell(&root, "cell-1", Some(b"{}\n"));
        make_cell(&root, &composed, Some(b"{}\n"));
        let instance = open_root(&root);
        for (created, alias) in [
            ("cell-1", "CELL-1"),
            (composed.as_str(), decomposed.as_str()),
        ] {
            instance
                .cell_dir(&cell(created))
                .unwrap_or_else(|error| panic!("{created:?} opens: {error:?}"));
            let path = root.join("cells").join(alias);
            let folded = fs::symlink_metadata(&path).is_ok();
            match (folded, instance.cell_dir(&cell(alias))) {
                (true, Err(CellDirError::Alias { path: refused })) => assert_eq!(refused, path),
                (false, Err(CellDirError::Missing { path: refused })) => assert_eq!(refused, path),
                (_, other) => panic!("{alias:?}, folded to {created:?}: {folded}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_longer_name_beginning_with_the_id_is_not_its_spelling() {
        let (_dir, root) = temp_root();
        make_cell(&root, "cell-1", None);
        make_cell(&root, "CELL-10", None);
        let folded = fs::symlink_metadata(root.join("cells/CELL-1")).is_ok();
        match (folded, open_root(&root).cell_dir(&cell("CELL-1"))) {
            (true, Err(CellDirError::Alias { .. }))
            | (false, Err(CellDirError::Missing { .. })) => {}
            (_, other) => panic!("folded {folded}: {other:?}"),
        }
    }

    fn cells_of(root: &Path) -> OwnedFd {
        open_at(open_root(root).dir.as_fd(), c"cells", DIRECTORY_FLAGS, 0).expect("cells opens")
    }

    fn opened_cell(root: &Path) -> (OwnedFd, OwnedFd) {
        make_cell(root, "cell-1", None);
        let cells = cells_of(root);
        let opened = open_at(cells.as_fd(), c"cell-1", DIRECTORY_FLAGS, 0).expect("the cell opens");
        (cells, opened)
    }

    #[test]
    fn an_opened_cell_listed_by_its_own_name_is_named_exactly() {
        let (_dir, root) = temp_root();
        let (cells, opened) = opened_cell(&root);
        let named = naming(cells.as_fd(), c"cell-1", opened.as_fd()).expect("the name is read");
        assert_eq!(named, Naming::Exact);
    }

    #[test]
    fn a_cell_retired_after_its_open_no_longer_names_the_opened_directory() {
        let (_dir, root) = temp_root();
        let (cells, opened) = opened_cell(&root);
        fs::rename(root.join("cells/cell-1"), root.join("retired")).expect("the cell is retired");
        let named = naming(cells.as_fd(), c"cell-1", opened.as_fd()).expect("the name is read");
        assert_eq!(named, Naming::Gone);
    }

    #[test]
    fn a_cell_replaced_after_its_open_no_longer_names_the_opened_directory() {
        let (_dir, root) = temp_root();
        let (cells, opened) = opened_cell(&root);
        fs::rename(root.join("cells/cell-1"), root.join("retired")).expect("the cell is retired");
        fs::create_dir(root.join("cells/cell-1")).expect("another directory takes its name");
        let named = naming(cells.as_fd(), c"cell-1", opened.as_fd()).expect("the name is read");
        assert_eq!(named, Naming::Gone);
    }

    #[test]
    fn a_cell_replaced_by_a_link_to_its_directory_is_gone() {
        let (_dir, root) = temp_root();
        let (cells, opened) = opened_cell(&root);
        fs::rename(root.join("cells/cell-1"), root.join("retired")).expect("the cell is retired");
        symlink(root.join("retired"), root.join("cells/cell-1")).expect("the link is made");
        let named = naming(cells.as_fd(), c"cell-1", opened.as_fd()).expect("the name is read");
        assert_eq!(named, Naming::Gone);
    }

    #[test]
    fn another_spelling_of_the_opened_cell_is_an_alias_where_the_filesystem_folds_it() {
        let (_dir, root) = temp_root();
        let (cells, opened) = opened_cell(&root);
        let folded = fs::symlink_metadata(root.join("cells/CELL-1")).is_ok();
        let named = naming(cells.as_fd(), c"CELL-1", opened.as_fd()).expect("the name is read");
        assert_eq!(named, if folded { Naming::Alias } else { Naming::Gone });
    }

    #[test]
    fn cell_dir_refuses_a_symlinked_cells_directory() {
        let (_dir, root) = temp_root();
        make_cell(&root.join("elsewhere"), "cell-1", None);
        symlink(root.join("elsewhere/cells"), root.join("cells")).expect("the link is made");
        expect_match!(open_root(&root).cell_dir(&cell("cell-1")), Err(CellDirError::Symlink { path }) => assert_eq!(path, root.join("cells")));
    }

    #[test]
    fn open_journal_for_append_creates_once_and_never_truncates() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let created = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell directory is created");
        let journal = root.join("cells/cell-1/ledger.ndjson");

        let mut first = appended(&created, &lock);
        assert!(first.created);
        assert_eq!(mode_of(&journal), 0o600);
        first.file.write_all(b"one\n").expect("the first record");
        let before = identity(&fs::symlink_metadata(&journal).expect("the journal"));

        let mut second = appended(
            &instance.cell_dir(&cell("cell-1")).expect("the cell"),
            &lock,
        );
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
        let (instance, lock) = locked(&root);
        make_cell(&root, "cell-1", Some(b"one\n"));
        let opened = instance.cell_dir(&cell("cell-1")).expect("the cell");
        let mut handle = appended(&opened, &lock);
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
    fn a_created_journal_handle_appends_after_other_writers() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let created = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell directory is created");
        let mut handle = appended(&created, &lock);
        assert!(handle.created);
        let journal = root.join("cells/cell-1/ledger.ndjson");
        fs::OpenOptions::new()
            .append(true)
            .open(&journal)
            .and_then(|mut other| other.write_all(b"one\n"))
            .expect("another writer extends the journal");
        handle
            .file
            .write_all(b"two\n")
            .expect("the record is written");
        assert_eq!(fs::read(&journal).expect("the journal"), b"one\ntwo\n");
    }

    #[test]
    fn the_append_handle_and_the_constructor_name_the_same_file() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let created = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell directory is created");
        let handle = appended(&created, &lock);
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
        let (instance, lock) = locked(&root);
        let cell_path = make_cell(&root, "cell-1", None);
        let target = root.join("elsewhere");
        fs::write(&target, b"keep").expect("the target is made");
        symlink(&target, cell_path.join("ledger.ndjson")).expect("the link is made");
        let opened = instance.cell_dir(&cell("cell-1")).expect("the cell");
        assert!(
            matches!(
                opened.open_journal_for_append(&lock),
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
        let (instance, lock) = locked(&root);
        for name in ["dir", "fifo", "socket"] {
            let opened = instance.cell_dir(&cell(name)).expect("the cell");
            let result = opened.open_journal_for_append(&lock);
            assert!(
                matches!(result, Err(JournalRefusal::NotRegular)),
                "{name}: {result:?}"
            );
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn the_cell_and_journal_descriptors_are_close_on_exec() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let created = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell directory is created");
        let handle = appended(&created, &lock);
        for (what, fd) in [
            ("cell", created.dir.as_raw_fd()),
            ("journal", handle.file.as_raw_fd()),
        ] {
            // SAFETY: fcntl with F_GETFD takes no pointer, and `created` and `handle` keep both
            // descriptors open through the loop.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            assert!(
                flags >= 0 && flags & libc::FD_CLOEXEC != 0,
                "the {what} descriptor would leak into a spawned process: flags {flags}"
            );
        }
    }

    #[test]
    fn lock_syncs_the_root_on_every_call() {
        let (_dir, root) = temp_root();
        let instance = recording_root(&root);
        drop(instance.lock().expect("the first lock is taken"));
        drop(instance.lock().expect("the second lock is taken"));
        assert_eq!(syncs_of(&root), 2);
    }

    #[test]
    fn lock_syncs_the_root_after_it_names_the_lock_file() {
        let (_dir, root) = temp_root();
        let instance = recording_root(&root);
        let _lock = instance.lock().expect("the lock is taken");
        assert_eq!(
            last_seen(&root).named(c"controller.lock"),
            Some(identity_at(&root.join("controller.lock"))),
            "the root was synced before it named the lock file"
        );
    }

    #[test]
    fn a_lock_retried_after_a_failed_root_sync_syncs_the_root() {
        let (_dir, root) = temp_root();
        let instance = recording_root(&root);
        fail_next_sync_of(&root);
        expect_match!(instance.lock(), Err(LockError::Io { path, source }) => {
            assert_eq!(path, root);
            assert_injected(&source);
        });
        assert!(
            a_separate_open_can_lock(&root.join("controller.lock")),
            "a failed lock() holds nothing"
        );
        let _lock = instance.lock().expect("the retry takes the lock");
        assert_eq!(syncs_of(&root), 2);
        assert_eq!(mode_of(&root.join("controller.lock")), 0o600);
    }

    #[test]
    fn create_cell_dir_syncs_the_root_and_cells_on_every_call() {
        let (_dir, root) = temp_root();
        let (instance, lock) = recording_locked(&root);
        instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the first cell is created");
        assert_eq!((syncs_of(&root), syncs_of(&root.join("cells"))), (1, 1));
        instance
            .create_cell_dir(&lock, &cell("cell-2"))
            .expect("the second cell is created");
        assert_eq!((syncs_of(&root), syncs_of(&root.join("cells"))), (2, 2));
    }

    #[test]
    fn create_cell_dir_syncs_each_parent_after_it_names_the_new_directory() {
        let (_dir, root) = temp_root();
        let (instance, lock) = recording_locked(&root);
        instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell is created");
        let cells = root.join("cells");
        assert_eq!(
            last_seen(&root).named(c"cells"),
            Some(identity_at(&cells)),
            "the root was synced before it named cells"
        );
        assert_eq!(
            last_seen(&cells).named(c"cell-1"),
            Some(identity_at(&cells.join("cell-1"))),
            "cells was synced before it named the cell"
        );
    }

    #[test]
    fn a_cell_creation_retried_after_a_failed_root_sync_syncs_the_root() {
        let (_dir, root) = temp_root();
        let (instance, lock) = recording_locked(&root);
        fail_next_sync_of(&root);
        expect_match!(instance.create_cell_dir(&lock, &cell("cell-1")), Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, root);
            assert_injected(&source);
        });
        instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the retry creates the cell");
        assert_eq!(syncs_of(&root), 2);
        assert_eq!(mode_of(&root.join("cells")), 0o700);
    }

    #[test]
    fn a_cells_directory_left_without_its_mode_is_made_private() {
        assert_not_root();
        for left in [0o500, 0o755, 0o300, 0o000] {
            let (_dir, root) = temp_root();
            let (instance, lock) = locked(&root);
            fs::create_dir(root.join("cells")).expect("cells is made");
            let _restore = restrict(&root.join("cells"), left);
            instance
                .create_cell_dir(&lock, &cell("cell-1"))
                .unwrap_or_else(|error| panic!("cells left at {left:o}: {error:?}"));
            assert_eq!(mode_of(&root.join("cells")), 0o700, "left at {left:o}");
        }
    }

    #[test]
    fn a_mode_set_by_name_is_private_and_never_follows_a_link() {
        let (_dir, root) = temp_root();
        let target = root.join("target");
        fs::create_dir(&target).expect("the target is made");
        set_mode(&target, 0o500);
        symlink(&target, root.join("link")).expect("the link is made");
        fs::create_dir(root.join("plain")).expect("the directory is made");
        set_mode(&root.join("plain"), 0o300);
        let parent = fs::File::open(&root).expect("the root opens");
        let on_link = change_mode_at(parent.as_fd(), c"link", PRIVATE_DIRECTORY);
        assert_eq!(
            mode_of(&target),
            0o500,
            "the chmod by name followed the link"
        );
        if cfg!(target_os = "macos") {
            on_link.expect("macOS sets the link's own mode");
            assert_eq!(mode_of(&root.join("link")), 0o700);
        } else {
            expect_match!(on_link, Err(error) => {
                assert_eq!(error.raw_os_error(), Some(libc::EOPNOTSUPP), "{error}")
            });
        }
        change_mode_at(parent.as_fd(), c"plain", PRIVATE_DIRECTORY).expect("the mode is set");
        assert_eq!(mode_of(&root.join("plain")), 0o700);
    }

    #[test]
    fn a_cell_whose_cells_sync_failed_is_not_adopted_and_its_retry_syncs_cells() {
        let (_dir, root) = temp_root();
        fs::create_dir(root.join("cells")).expect("cells is made");
        let (instance, lock) = recording_locked(&root);
        fail_next_sync_of(&root.join("cells"));
        expect_match!(instance.create_cell_dir(&lock, &cell("cell-1")), Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, root.join("cells"));
            assert_injected(&source);
        });
        expect_match!(
            instance.create_cell_dir(&lock, &cell("cell-1")),
            Err(CellDirError::AlreadyExists { path }) => assert_eq!(path, root.join("cells/cell-1"))
        );
        assert_eq!(syncs_of(&root.join("cells")), 2);
    }

    #[test]
    fn an_existing_journal_is_made_private_when_opened_for_append() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let cell_path = make_cell(&root, "cell-1", Some(b"one\n"));
        set_mode(&cell_path.join("ledger.ndjson"), 0o644);
        let opened = instance.cell_dir(&cell("cell-1")).expect("the cell");
        assert!(!appended(&opened, &lock).created);
        assert_eq!(mode_of(&cell_path.join("ledger.ndjson")), 0o600);
    }

    #[test]
    fn a_cell_directory_syncs_through_the_roots_function() {
        let (_dir, root) = temp_root();
        let (instance, lock) = recording_locked(&root);
        let created = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell is created");
        created.sync().expect("the cell directory syncs");
        assert_eq!(syncs_of(&root.join("cells/cell-1")), 1);
    }

    #[test]
    fn a_lock_on_another_root_is_refused_as_foreign() {
        let (_first_dir, first) = temp_root();
        let (_second_dir, second) = temp_root();
        let (_first, foreign) = locked(&first);
        let (instance, lock) = locked(&second);
        let created = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell is created");
        let lock_path = first.join("controller.lock");
        expect_match!(instance.discover(&foreign), Err(DiscoveryError::ForeignLock { path }) => {
            assert_eq!(path, lock_path)
        });
        expect_match!(instance.create_cell_dir(&foreign, &cell("cell-2")), Err(CellDirError::ForeignLock { path }) => {
            assert_eq!(path, lock_path)
        });
        expect_match!(created.open_journal_for_append(&foreign), Err(JournalRefusal::ForeignLock { path }) => {
            assert_eq!(path, lock_path)
        });
        for path in ["cells/cell-2", "cells/cell-1/ledger.ndjson"] {
            assert!(
                fs::symlink_metadata(second.join(path)).is_err(),
                "{path} was created under a foreign lock"
            );
        }
    }

    #[test]
    fn a_lock_taken_through_another_open_of_the_same_root_is_accepted() {
        let (_dir, root) = temp_root();
        let (_locking, lock) = locked(&root);
        let instance = open_root(&root);
        let created = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the same root's lock is accepted");
        assert!(appended(&created, &lock).created);
        assert_eq!(
            described(&instance.discover(&lock).expect("discovery succeeds")),
            ["cell cell-1: regular"]
        );
    }

    fn eio() -> io::Error {
        io::Error::from_raw_os_error(libc::EIO)
    }

    macro_rules! refused_at {
        ($($variant:ident)::+) => {
            Box::new($($variant)::+ {
                path: PathBuf::from("/inst/x"),
            }) as Box<dyn Error>
        };
    }

    macro_rules! failed_at {
        ($($variant:ident)::+) => {
            Box::new($($variant)::+ {
                path: PathBuf::from("/inst/x"),
                source: eio(),
            }) as Box<dyn Error>
        };
    }

    #[test]
    fn every_error_names_what_it_refused_and_keeps_its_cause() {
        let cause = Some(eio().to_string());
        let invalid = || CellPathError::NotACellName("..".to_string());
        let replaceable = || WalkRefusal::Replaceable {
            path: PathBuf::from("/inst"),
            mode: 0o777,
        };
        let refused = [
            refused_at!(LockError::Busy),
            refused_at!(LockError::Symlink),
            refused_at!(LockError::NotRegular),
            refused_at!(InstanceRootError::NotAbsolute),
            refused_at!(DiscoveryError::CellsSymlink),
            refused_at!(DiscoveryError::CellsNotADirectory),
            refused_at!(DiscoveryError::ForeignLock),
            refused_at!(CellDirError::Missing),
            refused_at!(CellDirError::Alias),
            refused_at!(CellDirError::Symlink),
            refused_at!(CellDirError::NotADirectory),
            refused_at!(CellDirError::AlreadyExists),
            refused_at!(CellDirError::ForeignLock),
            refused_at!(CellDirError::LockNotNamed),
            refused_at!(JournalRefusal::ForeignLock),
        ];
        let failed = [
            failed_at!(LockError::Io),
            failed_at!(InstanceRootError::Open),
            failed_at!(DiscoveryError::Open),
            failed_at!(DiscoveryError::Listing),
            failed_at!(DiscoveryError::Classify),
            failed_at!(DiscoveryError::OutOfDescriptors),
            failed_at!(CellDirError::Io),
        ];
        let mut rows: Vec<(Box<dyn Error>, &str, Option<String>)> = vec![
            (
                Box::new(CellDirError::InvalidCell(invalid())),
                "..",
                Some(invalid().to_string()),
            ),
            (
                Box::new(JournalRefusal::CellDirectory(eio())),
                "cell directory",
                cause.clone(),
            ),
            (
                Box::new(JournalRefusal::Io(eio())),
                "journal",
                cause.clone(),
            ),
            (Box::new(JournalRefusal::Symlink), "journal", None),
            (Box::new(JournalRefusal::NotRegular), "journal", None),
            (
                Box::new(InstanceRootError::Untrusted {
                    root: PathBuf::from("/inst/x"),
                    refusal: replaceable(),
                }),
                "/inst/x",
                Some(replaceable().to_string()),
            ),
        ];
        rows.extend(refused.into_iter().map(|error| (error, "/inst/x", None)));
        rows.extend(
            failed
                .into_iter()
                .map(|error| (error, "/inst/x", cause.clone())),
        );
        for (error, names, source) in rows {
            let text = error.to_string();
            assert!(text.contains(names), "{text:?} does not name {names:?}");
            assert_eq!(error.source().map(ToString::to_string), source, "{text}");
            if let Some(source) = source {
                assert!(text.contains(&source), "{text:?} omits its cause");
            }
        }
    }

    #[test]
    fn an_existing_lock_file_is_made_private_when_locked() {
        let (_dir, root) = temp_root();
        let path = root.join("controller.lock");
        fs::write(&path, b"").expect("the lock file is made");
        set_mode(&path, 0o644);
        let _lock = open_root(&root).lock().expect("the lock is taken");
        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn a_lock_is_foreign_to_a_new_root_made_where_its_own_root_was() {
        let (_dir, base) = temp_root();
        let at = base.join("inst");
        fs::create_dir(&at).expect("the root is made");
        let (_old, lock) = locked(&at);
        let moved = base.join("moved");
        fs::rename(&at, &moved).expect("the root is moved aside");
        fs::create_dir(&at).expect("a new root is made at the old path");
        make_cell(&at, "cell-1", None);
        let new = open_root(&at);
        let opened = new
            .cell_dir(&cell("cell-1"))
            .expect("the new root's cell opens");
        let lock_path = at.join("controller.lock");
        expect_match!(new.discover(&lock), Err(DiscoveryError::ForeignLock { path }) => {
            assert_eq!(path, lock_path)
        });
        expect_match!(new.create_cell_dir(&lock, &cell("cell-2")), Err(CellDirError::ForeignLock { path }) => {
            assert_eq!(path, lock_path)
        });
        expect_match!(opened.open_journal_for_append(&lock), Err(JournalRefusal::ForeignLock { path }) => {
            assert_eq!(path, lock_path)
        });
        for created in ["cells/cell-2", "cells/cell-1/ledger.ndjson"] {
            assert!(
                fs::symlink_metadata(at.join(created)).is_err(),
                "{created} was created under a foreign lock"
            );
        }
        open_root(&moved)
            .discover(&lock)
            .expect("the lock's own root is accepted at another path");
    }

    #[test]
    fn a_foreign_lock_creates_no_cells_directory() {
        let (_first_dir, first) = temp_root();
        let (_second_dir, second) = temp_root();
        let (_first, foreign) = locked(&first);
        let instance = open_root(&second);
        expect_match!(instance.create_cell_dir(&foreign, &cell("cell-1")), Err(CellDirError::ForeignLock { .. }) => ());
        assert!(
            fs::symlink_metadata(second.join("cells")).is_err(),
            "cells was created under a foreign lock"
        );
    }

    #[test]
    fn running_out_of_the_system_file_table_is_a_process_fault() {
        assert!(out_of_descriptors(&io::Error::from_raw_os_error(
            libc::ENFILE
        )));
        assert!(out_of_descriptors(&io::Error::from_raw_os_error(
            libc::EMFILE
        )));
        assert!(!out_of_descriptors(&io::Error::from_raw_os_error(
            libc::EACCES
        )));
    }

    #[test]
    fn a_refused_file_open_is_labelled_by_what_is_there_whatever_the_error() {
        let (_dir, root) = temp_root();
        symlink(root.join("missing"), root.join("link")).expect("the link is made");
        fs::create_dir(root.join("dir")).expect("the directory is made");
        fs::write(root.join("file"), b"").expect("the file is made");
        let instance = open_root(&root);
        let refusal = |name: &str| {
            let name = CString::new(name).expect("a name without NUL");
            file_refusal(
                instance.dir.as_fd(),
                &name,
                io::Error::from_raw_os_error(libc::EEXIST),
            )
        };
        assert!(matches!(refusal("link"), FileRefusal::Symlink));
        assert!(matches!(refusal("dir"), FileRefusal::NotRegular));
        assert!(
            matches!(refusal("file"), FileRefusal::Io(error) if error.raw_os_error() == Some(libc::EEXIST))
        );
    }

    const BOOT_TEXT: &str = "5d1e7c3a-0b2f-4e6d-9a8b-1c2d3e4f5a6b";

    fn boot() -> BootSession {
        BootSession::parse(BOOT_TEXT).expect("a boot session")
    }

    fn marker(name: &str, stopped: bool) -> LaunchedMarker {
        LaunchedMarker {
            cell: cell(name),
            boot_session: boot(),
            stopped,
        }
    }

    fn opened(root: &Path, name: &str) -> CellDir {
        make_cell(root, name, None);
        open_root(root)
            .cell_dir(&cell(name))
            .expect("the cell opens")
    }

    fn recording_cell(root: &Path, name: &str) -> (InstanceRoot, WriterLock, CellDir) {
        let (instance, lock) = recording_locked(root);
        let dir = instance
            .create_cell_dir(&lock, &cell(name))
            .expect("the cell is created");
        reset_syncs();
        (instance, lock, dir)
    }

    fn synced() -> Vec<(u64, u64)> {
        SYNCED.with(|all| all.borrow().clone())
    }

    fn identity_at(path: &Path) -> (u64, u64) {
        identity(&fs::symlink_metadata(path).expect("the entry exists"))
    }

    fn exists(path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok()
    }

    fn no_sync(_: BorrowedFd<'_>) -> io::Result<()> {
        Ok(())
    }

    #[test]
    fn a_launched_marker_is_one_line_with_or_without_a_clean_stop() {
        let running = marker("cell-1", false);
        let stopped = marker("cell-1", true);
        assert_eq!(
            running.encode(),
            format!("{{\"cell\":\"cell-1\",\"boot_session\":\"{BOOT_TEXT}\"}}\n").into_bytes()
        );
        assert_eq!(
            stopped.encode(),
            format!("{{\"cell\":\"cell-1\",\"boot_session\":\"{BOOT_TEXT}\",\"stopped\":true}}\n")
                .into_bytes()
        );
        for each in [running, stopped] {
            assert_eq!(
                LaunchedMarker::decode(&each.encode()).expect("its own record decodes"),
                each
            );
        }
        let spaced = format!("{{\"cell\": \"cell-9\", \"boot_session\": \"{BOOT_TEXT}\"}}\n");
        assert_eq!(
            LaunchedMarker::decode(spaced.as_bytes()).expect("spec 023's example form decodes"),
            marker("cell-9", false)
        );
        let upper = LaunchedMarker {
            boot_session: BootSession::parse(&BOOT_TEXT.to_uppercase()).expect("a boot session"),
            ..marker("cell-1", false)
        };
        assert_eq!(upper.encode(), running_bytes());
    }

    fn running_bytes() -> Vec<u8> {
        marker("cell-1", false).encode()
    }

    #[test]
    fn a_launched_marker_decodes_only_one_strict_record() {
        let good = format!("{{\"cell\":\"cell-1\",\"boot_session\":\"{BOOT_TEXT}\"}}");
        let with = |extra: &str| {
            format!("{{\"cell\":\"cell-1\",\"boot_session\":\"{BOOT_TEXT}\"{extra}}}\n")
                .into_bytes()
        };
        let session = |text: &str| {
            format!("{{\"cell\":\"cell-1\",\"boot_session\":\"{text}\"}}\n").into_bytes()
        };
        let cell_is = |text: &str| {
            format!("{{\"cell\":{text},\"boot_session\":\"{BOOT_TEXT}\"}}\n").into_bytes()
        };
        let refused: Vec<(&str, Vec<u8>)> = vec![
            ("no newline", good.clone().into_bytes()),
            ("a second line", format!("{good}\n{good}\n").into_bytes()),
            ("text after the newline", format!("{good}\n ").into_bytes()),
            ("a CRLF ending", format!("{good}\r\n").into_bytes()),
            ("an empty record", Vec::new()),
            ("only a newline", b"\n".to_vec()),
            ("a torn record", format!("{}\n", &good[..20]).into_bytes()),
            ("a repeated key", with(",\"cell\":\"cell-1\"")),
            ("an unknown key", with(",\"pid\":1")),
            ("stopped false", with(",\"stopped\":false")),
            ("stopped as a string", with(",\"stopped\":\"true\"")),
            ("stopped null", with(",\"stopped\":null")),
            ("stopped as a number", with(",\"stopped\":1")),
            ("an uppercase session", session(&BOOT_TEXT.to_uppercase())),
            (
                "a session without hyphens",
                session(&BOOT_TEXT.replace('-', "")),
            ),
            ("a braced session", session(&format!("{{{BOOT_TEXT}}}"))),
            ("a 35-character session", session(&BOOT_TEXT[..35])),
            ("a 37-character session", session(&format!("{BOOT_TEXT}0"))),
            (
                "a session that is not hex",
                session(&BOOT_TEXT.replace('a', "g")),
            ),
            (
                "a session that is a number",
                b"{\"cell\":\"cell-1\",\"boot_session\":5}\n".to_vec(),
            ),
            ("cell-0", cell_is("\"cell-0\"")),
            ("a cell outside spec 023's IDs", cell_is("\"a-journal\"")),
            ("a cell that is a number", cell_is("1")),
            (
                "no cell",
                format!("{{\"boot_session\":\"{BOOT_TEXT}\"}}\n").into_bytes(),
            ),
            ("no boot session", b"{\"cell\":\"cell-1\"}\n".to_vec()),
            ("an array", format!("[{good}]\n").into_bytes()),
            ("bytes that are not UTF-8", b"\xff\n".to_vec()),
        ];
        for (what, bytes) in refused {
            match LaunchedMarker::decode(&bytes) {
                Err(MarkerFault::Malformed(reason)) => assert!(!reason.is_empty(), "{what}"),
                other => panic!("{what} is malformed, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_launched_marker_reads_any_spelling_of_one_record() {
        let backslash = char::from(0x5c);
        let escaped_hyphen = format!("cell{backslash}u002d1");
        for (what, text) in [
            (
                "keys in reverse order",
                format!(
                    "{{\"stopped\":true,\"boot_session\":\"{BOOT_TEXT}\",\"cell\":\"cell-1\"}}\n"
                ),
            ),
            (
                "tabs and spaces around the tokens",
                format!(
                    " {{\t\"cell\" :\t\"cell-1\" , \"boot_session\":\"{BOOT_TEXT}\",\"stopped\": true }}\t\n"
                ),
            ),
            (
                "an escaped hyphen",
                format!(
                    "{{\"cell\":\"{escaped_hyphen}\",\"boot_session\":\"{BOOT_TEXT}\",\"stopped\":true}}\n"
                ),
            ),
        ] {
            assert_eq!(
                LaunchedMarker::decode(text.as_bytes())
                    .unwrap_or_else(|fault| panic!("{what}: {fault}")),
                marker("cell-1", true),
                "{what}"
            );
        }
    }

    #[test]
    fn a_launched_marker_holds_at_most_4096_bytes() {
        let padded = |total: usize| {
            let record = running_bytes();
            let mut bytes = b"{".to_vec();
            bytes.extend(std::iter::repeat_n(b' ', total - record.len()));
            bytes.extend(&record[1..]);
            assert_eq!(bytes.len(), total);
            bytes
        };
        assert_eq!(MAX_MARKER_BYTES, 4096);
        assert_eq!(
            LaunchedMarker::decode(&padded(4096)).expect("4,096 bytes decode"),
            marker("cell-1", false)
        );
        expect_match!(LaunchedMarker::decode(&padded(4097)), Err(MarkerFault::TooLarge) => ());
    }

    #[test]
    fn read_launched_is_absent_only_when_no_entry_is_there() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        expect_match!(dir.read_launched(), MarkerRead::Absent => ());
        fs::write(
            root.join("cells/cell-1/launched"),
            marker("cell-1", true).encode(),
        )
        .expect("the marker is written");
        expect_match!(dir.read_launched(), MarkerRead::Present(found) => {
            assert_eq!(found, marker("cell-1", true))
        });
    }

    #[test]
    fn a_symlink_named_launched_is_unreadable_and_not_followed() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = root.join("cells/cell-1/launched");
        let target = root.join("elsewhere");
        fs::write(&target, running_bytes()).expect("the target is written");
        symlink(&target, &path).expect("the link is made");
        expect_match!(dir.read_launched(), MarkerRead::Unreadable(MarkerFault::Symlink) => ());
        fs::remove_file(&path).expect("the link is removed");
        symlink(root.join("missing"), &path).expect("the dangling link is made");
        expect_match!(dir.read_launched(), MarkerRead::Unreadable(MarkerFault::Symlink) => ());
    }

    #[test]
    fn a_directory_or_fifo_named_launched_is_unreadable_without_blocking() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = root.join("cells/cell-1/launched");
        fs::create_dir(&path).expect("the directory is made");
        expect_match!(dir.read_launched(), MarkerRead::Unreadable(MarkerFault::NotRegular) => ());
        fs::remove_dir(&path).expect("the directory is removed");
        make_fifo(&path);
        expect_match!(
            within("reading a FIFO marker", move || dir.read_launched()),
            MarkerRead::Unreadable(MarkerFault::NotRegular) => ()
        );
    }

    #[test]
    fn a_marker_naming_another_cell_is_unreadable() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        fs::write(
            root.join("cells/cell-1/launched"),
            marker("cell-2", false).encode(),
        )
        .expect("the marker is written");
        expect_match!(dir.read_launched(), MarkerRead::Unreadable(MarkerFault::WrongCell { found }) => {
            assert_eq!(found, cell("cell-2"))
        });
    }

    #[test]
    fn a_torn_second_line_or_oversized_marker_is_unreadable() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = root.join("cells/cell-1/launched");
        let whole = running_bytes();
        for (what, bytes) in [
            ("an empty marker", Vec::new()),
            (
                "a marker without its newline",
                whole[..whole.len() - 1].to_vec(),
            ),
            ("half a marker", whole[..20].to_vec()),
            ("two records", [whole.clone(), whole.clone()].concat()),
            (
                "a record and a torn second line",
                [whole.clone(), whole[..9].to_vec()].concat(),
            ),
        ] {
            fs::write(&path, &bytes).expect("the marker is written");
            match dir.read_launched() {
                MarkerRead::Unreadable(MarkerFault::Malformed(_)) => {}
                other => panic!("{what} is malformed, got {other:?}"),
            }
        }
        fs::write(&path, vec![b' '; 5000]).expect("the marker is written");
        expect_match!(dir.read_launched(), MarkerRead::Unreadable(MarkerFault::TooLarge) => ());
    }

    #[test]
    fn a_marker_that_cannot_be_opened_is_an_io_fault_not_absence() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = root.join("cells/cell-1/launched");
        fs::write(&path, running_bytes()).expect("the marker is written");
        let restore = restrict(&path, 0o000);
        expect_match!(dir.read_launched(), MarkerRead::Unreadable(MarkerFault::Io(source)) => {
            assert_eq!(source.raw_os_error(), Some(libc::EACCES))
        });
        drop(restore);
        let restore = restrict(&root.join("cells/cell-1"), 0o000);
        let result = dir.read_launched();
        drop(restore);
        expect_match!(result, MarkerRead::Unreadable(MarkerFault::Io(source)) => {
            assert_eq!(source.raw_os_error(), Some(libc::EACCES))
        });
    }

    #[test]
    fn create_launched_writes_a_private_marker_then_syncs_it_and_the_directory() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        dir.create_launched(boot()).expect("the marker is created");
        let path = root.join("cells/cell-1/launched");
        assert_eq!(fs::read(&path).expect("the marker reads"), running_bytes());
        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(
            synced(),
            [identity_at(&path), identity_at(&root.join("cells/cell-1"))]
        );
        expect_match!(dir.read_launched(), MarkerRead::Present(found) => {
            assert_eq!(found, marker("cell-1", false))
        });
    }

    #[test]
    fn create_launched_never_replaces_an_entry() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = root.join("cells/cell-1/launched");
        dir.create_launched(boot())
            .expect("the first marker is created");
        let later = BootSession::parse("00000000-0000-4000-8000-000000000001").expect("a session");
        expect_match!(dir.create_launched(later), Err(CellDirError::AlreadyExists { path: refused }) => {
            assert_eq!(refused, path)
        });
        assert_eq!(fs::read(&path).expect("the marker reads"), running_bytes());
        fs::remove_file(&path).expect("the marker is removed");
        let target = root.join("never-created");
        symlink(&target, &path).expect("the dangling link is made");
        expect_match!(dir.create_launched(boot()), Err(CellDirError::AlreadyExists { .. }) => ());
        assert!(!exists(&target), "the marker was created through the link");
        assert!(
            !exists(&root.join("cells/cell-1/launched.tmp")),
            "a refused create left its temporary"
        );
    }

    #[test]
    fn create_launched_replaces_a_temporary_left_by_a_crash() {
        for stale in ["a torn file", "a symlink"] {
            let (_dir, root) = temp_root();
            let dir = opened(&root, "cell-1");
            let temporary = root.join("cells/cell-1/launched.tmp");
            let target = root.join("elsewhere");
            if stale == "a symlink" {
                fs::write(&target, b"keep").expect("the target is written");
                symlink(&target, &temporary).expect("the link is made");
            } else {
                fs::write(&temporary, b"{\"cell\":\"cel").expect("the temporary is written");
            }
            dir.create_launched(boot())
                .unwrap_or_else(|error| panic!("{stale}: {error:?}"));
            expect_match!(dir.read_launched(), MarkerRead::Present(found) => {
                assert_eq!(found, marker("cell-1", false), "{stale}")
            });
            assert!(!exists(&temporary), "{stale} was left");
            if stale == "a symlink" {
                assert_eq!(fs::read(&target).expect("the target reads"), b"keep");
            }
        }
    }

    #[test]
    fn a_directory_named_launched_tmp_is_an_io_error_and_no_marker_is_written() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = root.join("cells/cell-1/launched");
        let temporary = root.join("cells/cell-1/launched.tmp");
        fs::create_dir(&temporary).expect("the directory is made");
        expect_match!(dir.create_launched(boot()), Err(CellDirError::Io { path: failed, .. }) => {
            assert_eq!(failed, temporary)
        });
        expect_match!(dir.read_launched(), MarkerRead::Absent => ());
        fs::write(&path, running_bytes()).expect("the marker is written");
        expect_match!(dir.mark_launched_stopped(), Err(CellDirError::Io { path: failed, .. }) => {
            assert_eq!(failed, temporary)
        });
        assert_eq!(fs::read(&path).expect("the marker reads"), running_bytes());
        assert!(
            temporary.is_dir(),
            "the directory named launched.tmp was removed"
        );
    }

    #[test]
    fn create_launched_syncs_the_whole_record_and_then_the_directory_that_names_it() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        dir.create_launched(boot()).expect("the marker is created");
        let path = root.join("cells/cell-1/launched");
        assert_eq!(
            last_seen(&path).bytes,
            Some(running_bytes().len() as u64),
            "the file was synced before its bytes were written"
        );
        let directory = last_seen(&root.join("cells/cell-1"));
        assert_eq!(
            directory.named(c"launched"),
            Some(identity_at(&path)),
            "the directory was synced before it named the marker"
        );
        assert_eq!(directory.named(c"launched.tmp"), None);
    }

    #[test]
    fn a_failed_file_sync_leaves_no_launched_and_a_retry_creates_it() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        let cell_path = root.join("cells/cell-1");
        fail_next_file_sync();
        expect_match!(dir.create_launched(boot()), Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, cell_path.join("launched.tmp"));
            assert_injected(&source);
        });
        expect_match!(dir.read_launched(), MarkerRead::Absent => ());
        assert_eq!(
            syncs_of(&cell_path),
            0,
            "the directory was synced after a failed file sync"
        );
        dir.create_launched(boot())
            .expect("a retry creates the marker");
        expect_match!(dir.read_launched(), MarkerRead::Present(found) => {
            assert_eq!(found, marker("cell-1", false))
        });
        assert!(!exists(&cell_path.join("launched.tmp")));
    }

    #[test]
    fn a_failed_directory_sync_after_create_launched_leaves_the_whole_marker() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        let cell_path = root.join("cells/cell-1");
        fail_next_sync_of(&cell_path);
        expect_match!(dir.create_launched(boot()), Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, cell_path);
            assert_injected(&source);
        });
        expect_match!(dir.read_launched(), MarkerRead::Present(found) => {
            assert_eq!(found, marker("cell-1", false))
        });
        expect_match!(dir.create_launched(boot()), Err(CellDirError::AlreadyExists { .. }) => ());
        assert!(!exists(&cell_path.join("launched.tmp")));
        dir.sync().expect("a later sync of the directory completes");
    }

    #[test]
    fn create_launched_reports_a_marker_it_cannot_put_in_place() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        let taken = root.join("cells/cell-1/launched.tmp");
        before_next_file_sync(move || {
            let _ = fs::remove_file(&taken);
        });
        expect_match!(dir.create_launched(boot()), Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, root.join("cells/cell-1/launched"));
            assert_eq!(source.raw_os_error(), Some(libc::ENOENT), "{source}");
        });
        expect_match!(dir.read_launched(), MarkerRead::Absent => ());
    }

    #[test]
    fn create_launched_refuses_a_cell_outside_spec_023s_ids() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "a-journal");
        expect_match!(
            dir.create_launched(boot()),
            Err(CellDirError::InvalidCell(CellPathError::NotACellName(text))) => assert_eq!(text, "a-journal")
        );
        assert!(!exists(&root.join("cells/a-journal/launched")));
    }

    #[cfg(target_os = "macos")]
    fn host_boot_session_text() -> String {
        let output = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "kern.bootsessionuuid"])
            .output()
            .expect("sysctl runs");
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout)
            .expect("sysctl prints text")
            .trim_end_matches('\n')
            .to_lowercase()
    }

    #[cfg(target_os = "linux")]
    fn host_boot_session_text() -> String {
        fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .expect("the boot ID reads")
            .trim_end_matches('\n')
            .to_lowercase()
    }

    #[test]
    fn the_marker_records_the_hosts_boot_session_in_lowercase() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        dir.create_launched(crate::clock::boot_session().expect("the host boot session"))
            .expect("the marker is created");
        let record: serde_json::Value = serde_json::from_slice(
            &fs::read(root.join("cells/cell-1/launched")).expect("the marker reads"),
        )
        .expect("the marker is JSON");
        assert_eq!(record["boot_session"], host_boot_session_text());
    }

    #[test]
    fn mark_launched_stopped_replaces_the_marker_by_a_synced_rename() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        dir.create_launched(boot()).expect("the marker is created");
        let path = root.join("cells/cell-1/launched");
        let before = identity_at(&path);
        let mut old = fs::File::open(&path).expect("the old marker opens");
        reset_syncs();
        dir.mark_launched_stopped()
            .expect("the clean stop is recorded");
        assert_eq!(
            fs::read(&path).expect("the marker reads"),
            marker("cell-1", true).encode()
        );
        let after = identity_at(&path);
        assert_ne!(after, before, "the marker was rewritten in place");
        let mut kept = Vec::new();
        old.read_to_end(&mut kept).expect("the old marker reads");
        assert_eq!(
            kept,
            running_bytes(),
            "the old record changed under its reader"
        );
        assert_eq!(mode_of(&path), 0o600);
        assert!(!exists(&root.join("cells/cell-1/launched.tmp")));
        assert_eq!(synced(), [after, identity_at(&root.join("cells/cell-1"))]);
    }

    #[test]
    fn mark_launched_stopped_replaces_a_temporary_left_by_a_crash() {
        for stale in ["a torn file", "a symlink"] {
            let (_dir, root) = temp_root();
            let dir = opened(&root, "cell-1");
            dir.create_launched(boot()).expect("the marker is created");
            let temporary = root.join("cells/cell-1/launched.tmp");
            let target = root.join("elsewhere");
            if stale == "a symlink" {
                fs::write(&target, b"keep").expect("the target is written");
                symlink(&target, &temporary).expect("the link is made");
            } else {
                fs::write(&temporary, b"{\"cell\":\"cel").expect("the temporary is written");
            }
            dir.mark_launched_stopped()
                .unwrap_or_else(|error| panic!("{stale}: {error:?}"));
            expect_match!(dir.read_launched(), MarkerRead::Present(found) => {
                assert_eq!(found, marker("cell-1", true), "{stale}")
            });
            assert!(!exists(&temporary), "{stale} was left");
            if stale == "a symlink" {
                assert_eq!(fs::read(&target).expect("the target reads"), b"keep");
            }
        }
    }

    #[test]
    fn mark_launched_stopped_needs_this_cells_marker_and_writes_nothing_without_it() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = root.join("cells/cell-1/launched");
        let temporary = root.join("cells/cell-1/launched.tmp");
        expect_match!(dir.mark_launched_stopped(), Err(CellDirError::Missing { path: refused }) => {
            assert_eq!(refused, path)
        });
        assert!(!exists(&path) && !exists(&temporary));
        let foreign = marker("cell-2", false).encode();
        fs::write(&path, &foreign).expect("the marker is written");
        expect_match!(
            dir.mark_launched_stopped(),
            Err(CellDirError::MarkerUnreadable { path: refused, fault: MarkerFault::WrongCell { .. } }) => {
                assert_eq!(refused, path)
            }
        );
        assert_eq!(fs::read(&path).expect("the marker reads"), foreign);
        assert!(!exists(&temporary));
    }

    #[test]
    fn mark_launched_stopped_on_a_stopped_marker_keeps_it_stopped() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        dir.create_launched(boot()).expect("the marker is created");
        dir.mark_launched_stopped().expect("the first stop");
        dir.mark_launched_stopped().expect("the second stop");
        assert_eq!(
            fs::read(root.join("cells/cell-1/launched")).expect("the marker reads"),
            marker("cell-1", true).encode()
        );
    }

    #[test]
    fn mark_launched_stopped_syncs_the_whole_record_and_then_the_directory_after_the_rename() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        dir.create_launched(boot()).expect("the marker is created");
        reset_syncs();
        dir.mark_launched_stopped()
            .expect("the clean stop is recorded");
        let path = root.join("cells/cell-1/launched");
        assert_eq!(
            last_seen(&path).bytes,
            Some(marker("cell-1", true).encode().len() as u64),
            "the file was synced before its bytes were written"
        );
        let directory = last_seen(&root.join("cells/cell-1"));
        assert_eq!(
            directory.named(c"launched"),
            Some(identity_at(&path)),
            "the directory was synced before the rename"
        );
        assert_eq!(directory.named(c"launched.tmp"), None);
    }

    #[test]
    fn a_failed_sync_in_mark_launched_stopped_leaves_one_whole_record() {
        for failing in ["the file", "the directory"] {
            let (_dir, root) = temp_root();
            let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
            dir.create_launched(boot()).expect("the marker is created");
            reset_syncs();
            let cell_path = root.join("cells/cell-1");
            let (failed_at, kept) = if failing == "the file" {
                fail_next_file_sync();
                (cell_path.join("launched.tmp"), marker("cell-1", false))
            } else {
                fail_next_sync_of(&cell_path);
                (cell_path.clone(), marker("cell-1", true))
            };
            expect_match!(dir.mark_launched_stopped(), Err(CellDirError::Io { path, source }) => {
                assert_eq!(path, failed_at, "{failing}");
                assert_injected(&source);
            });
            expect_match!(dir.read_launched(), MarkerRead::Present(found) => {
                assert_eq!(found, kept, "{failing}")
            });
            dir.mark_launched_stopped()
                .unwrap_or_else(|error| panic!("{failing}: a retry failed: {error:?}"));
            expect_match!(dir.read_launched(), MarkerRead::Present(found) => {
                assert_eq!(found, marker("cell-1", true), "{failing}")
            });
            assert!(!exists(&cell_path.join("launched.tmp")), "{failing}");
        }
    }

    #[test]
    fn mark_launched_stopped_reports_a_rename_it_cannot_make_and_keeps_the_temporary() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        dir.create_launched(boot()).expect("the marker is created");
        let path = root.join("cells/cell-1/launched");
        let obstacle = path.clone();
        before_next_file_sync(move || {
            fs::remove_file(&obstacle).expect("the marker is removed");
            fs::create_dir(&obstacle).expect("a directory takes its name");
            fs::write(obstacle.join("keep"), b"x").expect("the directory is not empty");
        });
        expect_match!(dir.mark_launched_stopped(), Err(CellDirError::Io { path: failed, source }) => {
            assert_eq!(failed, path);
            assert_eq!(source.raw_os_error(), Some(libc::EISDIR), "{source}");
        });
        assert_eq!(
            fs::read(path.join("keep")).expect("the directory is kept"),
            b"x"
        );
        assert_eq!(
            fs::read(root.join("cells/cell-1/launched.tmp")).expect("the temporary is kept"),
            marker("cell-1", true).encode()
        );
    }

    fn read_while(reader: CellDir, accepted: fn(&MarkerRead) -> bool, work: impl FnOnce()) {
        let started = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let reading = {
            let started = Arc::clone(&started);
            let done = Arc::clone(&done);
            thread::spawn(move || {
                loop {
                    let read = reader.read_launched();
                    started.store(true, Ordering::SeqCst);
                    if !accepted(&read) {
                        return Err(format!("{read:?}"));
                    }
                    if done.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                }
            })
        };
        while !started.load(Ordering::SeqCst) && !reading.is_finished() {
            thread::yield_now();
        }
        work();
        done.store(true, Ordering::SeqCst);
        reading
            .join()
            .expect("the reader did not panic")
            .unwrap_or_else(|seen| panic!("a reader saw {seen}"));
    }

    fn writer_and_reader(root: &Path) -> (CellDir, CellDir) {
        make_cell(root, "cell-1", None);
        let mut instance = open_root(root);
        instance.sync = no_sync;
        let writer = instance.cell_dir(&cell("cell-1")).expect("the cell opens");
        let reader = instance.cell_dir(&cell("cell-1")).expect("the cell opens");
        (writer, reader)
    }

    #[test]
    fn a_reader_never_sees_a_partial_marker_while_it_is_rewritten() {
        let (_dir, root) = temp_root();
        let (writer, reader) = writer_and_reader(&root);
        writer
            .create_launched(boot())
            .expect("the marker is created");
        let present = |read: &MarkerRead| matches!(read, MarkerRead::Present(found) if found.cell == cell("cell-1"));
        read_while(reader, present, || {
            for _ in 0..1_000 {
                writer
                    .mark_launched_stopped()
                    .expect("the clean stop is recorded");
            }
        });
    }

    #[test]
    fn a_reader_never_sees_a_partial_marker_while_it_is_created() {
        let (_dir, root) = temp_root();
        let (writer, reader) = writer_and_reader(&root);
        let absent_or_whole = |read: &MarkerRead| match read {
            MarkerRead::Absent => true,
            MarkerRead::Present(found) => *found == marker("cell-1", false),
            MarkerRead::Unreadable(_) => false,
        };
        read_while(reader, absent_or_whole, || {
            for _ in 0..500 {
                writer
                    .create_launched(boot())
                    .expect("the marker is created");
                writer.remove_launched().expect("the marker is removed");
            }
        });
    }

    #[test]
    fn remove_launched_removes_the_marker_syncs_and_tolerates_absence() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        dir.create_launched(boot()).expect("the marker is created");
        reset_syncs();
        dir.remove_launched().expect("the marker is removed");
        expect_match!(dir.read_launched(), MarkerRead::Absent => ());
        dir.remove_launched()
            .expect("a missing marker is not an error");
        assert_eq!(synced(), [identity_at(&root.join("cells/cell-1")); 2]);
        let path = root.join("cells/cell-1/launched");
        fs::create_dir(&path).expect("the directory is made");
        expect_match!(dir.remove_launched(), Err(CellDirError::Io { path: refused, .. }) => {
            assert_eq!(refused, path)
        });
        assert!(exists(&path), "a directory named launched was removed");
    }

    #[test]
    fn remove_launched_syncs_the_directory_after_the_unlink() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        dir.create_launched(boot()).expect("the marker is created");
        dir.remove_launched().expect("the marker is removed");
        assert_eq!(
            last_seen(&root.join("cells/cell-1")).named(c"launched"),
            None,
            "the directory was synced before the unlink"
        );
    }

    #[test]
    fn a_failed_sync_after_remove_launched_leaves_no_marker_and_a_retry_syncs() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        let cell_path = root.join("cells/cell-1");
        dir.create_launched(boot()).expect("the marker is created");
        reset_syncs();
        fail_next_sync_of(&cell_path);
        expect_match!(dir.remove_launched(), Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, cell_path);
            assert_injected(&source);
        });
        expect_match!(dir.read_launched(), MarkerRead::Absent => ());
        dir.remove_launched().expect("a retry syncs the directory");
        assert_eq!(syncs_of(&cell_path), 2);
    }

    #[test]
    fn create_runtime_dir_syncs_the_directory_after_it_names_runtime() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        drop(dir.create_runtime_dir().expect("runtime is created"));
        let cell_path = root.join("cells/cell-1");
        assert_eq!(
            last_seen(&cell_path).named(c"runtime"),
            Some(identity_at(&cell_path.join("runtime"))),
            "the directory was synced before it named runtime"
        );
    }

    #[test]
    fn a_failed_sync_after_create_runtime_dir_leaves_runtime_in_place() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        let cell_path = root.join("cells/cell-1");
        let runtime = cell_path.join("runtime");
        fail_next_sync_of(&cell_path);
        expect_match!(dir.create_runtime_dir(), Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, cell_path);
            assert_injected(&source);
        });
        assert_eq!(mode_of(&runtime), 0o700);
        expect_match!(dir.create_runtime_dir(), Err(CellDirError::AlreadyExists { path }) => {
            assert_eq!(path, runtime)
        });
        dir.sync().expect("a later sync of the directory completes");
    }

    #[test]
    fn create_runtime_dir_is_private_exclusive_and_synced() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        let runtime = dir.create_runtime_dir().expect("runtime is created");
        let path = root.join("cells/cell-1/runtime");
        assert!(
            fs::symlink_metadata(&path)
                .expect("runtime exists")
                .is_dir()
        );
        assert_eq!(mode_of(&path), 0o700);
        assert_eq!(
            identity(
                &fs::File::from(runtime)
                    .metadata()
                    .expect("the descriptor's fstat")
            ),
            identity_at(&path)
        );
        assert_eq!(synced(), [identity_at(&root.join("cells/cell-1"))]);
        expect_match!(dir.create_runtime_dir(), Err(CellDirError::AlreadyExists { path: refused }) => {
            assert_eq!(refused, path)
        });
    }

    #[test]
    fn create_runtime_dir_never_follows_or_reuses_an_entry() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let target = root.join("elsewhere");
        symlink(&target, root.join("cells/cell-1/runtime")).expect("the dangling link is made");
        expect_match!(dir.create_runtime_dir(), Err(CellDirError::AlreadyExists { .. }) => ());
        assert!(!exists(&target), "runtime was created through the link");
    }

    #[test]
    fn create_created_is_exclusive_private_and_synced() {
        let (_dir, root) = temp_root();
        let (_instance, lock, dir) = recording_cell(&root, "cell-1");
        assert!(!dir.has_created().expect("created is checked"));
        dir.create_created(&lock).expect("created is written");
        let path = root.join("cells/cell-1/created");
        assert_eq!(fs::read(&path).expect("created reads"), b"");
        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(
            synced(),
            [identity_at(&path), identity_at(&root.join("cells/cell-1"))]
        );
        assert!(dir.has_created().expect("created is checked"));
        expect_match!(dir.create_created(&lock), Err(CellDirError::AlreadyExists { path: refused }) => {
            assert_eq!(refused, path)
        });
    }

    #[test]
    fn a_failed_sync_after_create_created_leaves_created_in_place() {
        for failing in ["the file", "the directory"] {
            let (_dir, root) = temp_root();
            let (_instance, lock, dir) = recording_cell(&root, "cell-1");
            let cell_path = root.join("cells/cell-1");
            let failed_at = if failing == "the file" {
                fail_next_file_sync();
                cell_path.join("created")
            } else {
                fail_next_sync_of(&cell_path);
                cell_path.clone()
            };
            expect_match!(dir.create_created(&lock), Err(CellDirError::Io { path, source }) => {
                assert_eq!(path, failed_at, "{failing}");
                assert_injected(&source);
            });
            assert!(dir.has_created().expect("created is checked"), "{failing}");
            expect_match!(dir.create_created(&lock), Err(CellDirError::AlreadyExists { .. }) => ());
        }
    }

    #[test]
    fn create_created_refuses_a_foreign_lock() {
        let (_first_dir, first) = temp_root();
        let (_second_dir, second) = temp_root();
        let (_first, foreign) = locked(&first);
        let (instance, lock) = locked(&second);
        let dir = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell is created");
        expect_match!(dir.create_created(&foreign), Err(CellDirError::ForeignLock { path }) => {
            assert_eq!(path, first.join("controller.lock"))
        });
        assert!(!exists(&second.join("cells/cell-1/created")));
    }

    #[test]
    fn a_created_entry_that_is_not_a_regular_file_is_refused_and_not_followed() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let dir = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell is created");
        let path = root.join("cells/cell-1/created");
        let target = root.join("elsewhere");
        fs::write(&target, b"").expect("the target is written");
        symlink(&target, &path).expect("the link is made");
        expect_match!(dir.has_created(), Err(CellDirError::NotARegularFile { path: refused }) => {
            assert_eq!(refused, path)
        });
        expect_match!(dir.create_created(&lock), Err(CellDirError::AlreadyExists { .. }) => ());
        fs::remove_file(&path).expect("the link is removed");
        fs::create_dir(&path).expect("the directory is made");
        expect_match!(dir.has_created(), Err(CellDirError::NotARegularFile { .. }) => ());
    }

    #[test]
    fn retire_moves_the_whole_cell_directory_and_unlinks_nothing() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let dir = instance
            .create_cell_dir(&lock, &cell("cell-3"))
            .expect("the cell is created");
        let cell_path = root.join("cells/cell-3");
        appended(&dir, &lock)
            .file
            .write_all(b"{}\n")
            .expect("the journal is written");
        dir.create_launched(boot()).expect("the marker is created");
        fs::write(cell_path.join("membrane.uds"), b"stale").expect("the stale socket is made");
        drop(dir.create_runtime_dir().expect("runtime is created"));
        fs::write(cell_path.join("runtime/root.img"), b"disk").expect("the disk is written");
        let before = identity_at(&cell_path);
        instance.retire(&lock, dir).expect("the cell retires");
        assert!(!exists(&cell_path));
        let retired = root.join("retired/cell-3");
        assert_eq!(identity_at(&retired), before);
        let marker = marker("cell-3", false).encode();
        for (name, bytes) in [
            ("ledger.ndjson", &b"{}\n"[..]),
            ("launched", &marker[..]),
            ("membrane.uds", b"stale"),
            ("runtime/root.img", b"disk"),
        ] {
            assert_eq!(
                fs::read(retired.join(name)).expect("the entry moved"),
                bytes,
                "{name}"
            );
        }
        assert!(described(&instance.discover(&lock).expect("discovery succeeds")).is_empty());
    }

    #[test]
    fn retire_makes_retired_private_and_syncs_the_root_then_cells_then_retired() {
        for existing in [None, Some(0o755)] {
            let (_dir, root) = temp_root();
            if let Some(mode) = existing {
                fs::create_dir(root.join("retired")).expect("retired is made");
                set_mode(&root.join("retired"), mode);
            }
            let (instance, lock, dir) = recording_cell(&root, "cell-1");
            instance.retire(&lock, dir).expect("the cell retires");
            assert_eq!(mode_of(&root.join("retired")), 0o700, "{existing:?}");
            assert_eq!(
                synced(),
                [
                    identity_at(&root),
                    identity_at(&root.join("cells")),
                    identity_at(&root.join("retired")),
                ],
                "{existing:?}"
            );
        }
    }

    #[test]
    fn retire_makes_a_retired_it_cannot_read_private_and_retires_into_it() {
        assert_not_root();
        for left in [0o300, 0o000] {
            let (_dir, root) = temp_root();
            let retired = root.join("retired");
            fs::create_dir(&retired).expect("retired is made");
            let _restore = restrict(&retired, left);
            let (instance, lock, dir) = recording_cell(&root, "cell-1");
            instance
                .retire(&lock, dir)
                .unwrap_or_else(|error| panic!("retired left at {left:o}: {error:?}"));
            assert_eq!(mode_of(&retired), 0o700, "left at {left:o}");
            assert!(exists(&retired.join("cell-1")), "left at {left:o}");
        }
    }

    #[test]
    fn retire_syncs_cells_and_retired_after_the_rename() {
        let (_dir, root) = temp_root();
        let (instance, lock, dir) = recording_cell(&root, "cell-1");
        instance.retire(&lock, dir).expect("the cell retires");
        let retired = root.join("retired");
        assert_eq!(
            last_seen(&root).named(c"retired"),
            Some(identity_at(&retired)),
            "the root was synced before it named retired"
        );
        assert_eq!(
            last_seen(&root.join("cells")).named(c"cell-1"),
            None,
            "cells was synced before the rename"
        );
        assert_eq!(
            last_seen(&retired).named(c"cell-1"),
            Some(identity_at(&retired.join("cell-1"))),
            "retired was synced before the rename"
        );
    }

    #[test]
    fn retire_returns_a_failed_sync_naming_the_directory() {
        for (failing, renamed) in [("the root", false), ("cells", true), ("retired", true)] {
            let (_dir, root) = temp_root();
            fs::create_dir(root.join("retired")).expect("retired is made");
            let (instance, lock, dir) = recording_cell(&root, "cell-1");
            let failed_at = if failing == "the root" {
                root.clone()
            } else {
                root.join(failing)
            };
            fail_next_sync_of(&failed_at);
            expect_match!(instance.retire(&lock, dir), Err(RetireError::Io { path, source }) => {
                assert_eq!(path, failed_at, "{failing}");
                assert_injected(&source);
            });
            assert_eq!(exists(&root.join("retired/cell-1")), renamed, "{failing}");
            assert_eq!(exists(&root.join("cells/cell-1")), !renamed, "{failing}");
        }
    }

    #[test]
    fn retire_refuses_a_retired_that_is_a_symlink_or_not_a_directory() {
        for planted in ["a symlink", "a regular file"] {
            let (_dir, root) = temp_root();
            let (instance, lock) = locked(&root);
            let dir = instance
                .create_cell_dir(&lock, &cell("cell-1"))
                .expect("the cell is created");
            let elsewhere = root.join("elsewhere");
            fs::create_dir(&elsewhere).expect("the link target is made");
            if planted == "a symlink" {
                symlink(&elsewhere, root.join("retired")).expect("the link is made");
            } else {
                fs::write(root.join("retired"), b"").expect("the file is made");
            }
            expect_match!(instance.retire(&lock, dir), Err(RetireError::RetiredNotADirectory { path }) => {
                assert_eq!(path, root.join("retired"), "{planted}")
            });
            assert!(
                exists(&root.join("cells/cell-1")),
                "{planted}: the cell moved"
            );
            assert_eq!(
                fs::read_dir(&elsewhere).expect("the target lists").count(),
                0,
                "{planted}: the cell moved through the link"
            );
        }
    }

    #[test]
    fn retire_never_replaces_an_earlier_retired_directory() {
        for earlier_holds in [None, Some("root.img")] {
            let (_dir, root) = temp_root();
            let (instance, lock) = locked(&root);
            let dir = instance
                .create_cell_dir(&lock, &cell("cell-3"))
                .expect("the cell is created");
            let earlier = root.join("retired/cell-3");
            fs::create_dir_all(&earlier).expect("the earlier retired cell is made");
            if let Some(name) = earlier_holds {
                fs::write(earlier.join(name), b"kept disk").expect("the kept disk is written");
            }
            let live = identity_at(&root.join("cells/cell-3"));
            let kept = identity_at(&earlier);
            expect_match!(instance.retire(&lock, dir), Err(RetireError::AlreadyRetired { path }) => {
                assert_eq!(path, earlier, "{earlier_holds:?}")
            });
            assert_eq!(identity_at(&root.join("cells/cell-3")), live);
            assert_eq!(identity_at(&earlier), kept);
            if let Some(name) = earlier_holds {
                assert_eq!(
                    fs::read(earlier.join(name)).expect("the kept disk reads"),
                    b"kept disk"
                );
            }
        }
    }

    #[test]
    fn retire_refuses_a_directory_swapped_in_under_the_name() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let dir = instance
            .create_cell_dir(&lock, &cell("cell-2"))
            .expect("the cell is created");
        fs::rename(root.join("cells/cell-2"), root.join("aside")).expect("the cell is moved aside");
        fs::create_dir(root.join("cells/cell-2")).expect("another directory takes its name");
        let swapped = identity_at(&root.join("cells/cell-2"));
        expect_match!(instance.retire(&lock, dir), Err(RetireError::Moved { path }) => {
            assert_eq!(path, root.join("cells/cell-2"))
        });
        assert_eq!(identity_at(&root.join("cells/cell-2")), swapped);
        assert!(!exists(&root.join("retired/cell-2")));
    }

    #[test]
    fn retire_refuses_a_cell_whose_name_is_gone_or_a_link() {
        for replaced_by in [None, Some("a link to it")] {
            let (_dir, root) = temp_root();
            let (instance, lock) = locked(&root);
            let dir = instance
                .create_cell_dir(&lock, &cell("cell-2"))
                .expect("the cell is created");
            fs::rename(root.join("cells/cell-2"), root.join("aside"))
                .expect("the cell is moved aside");
            if replaced_by.is_some() {
                symlink(root.join("aside"), root.join("cells/cell-2")).expect("the link is made");
            }
            expect_match!(instance.retire(&lock, dir), Err(RetireError::Moved { path }) => {
                assert_eq!(path, root.join("cells/cell-2"), "{replaced_by:?}")
            });
            assert!(exists(&root.join("aside")));
            assert!(!exists(&root.join("retired/cell-2")), "{replaced_by:?}");
        }
    }

    #[test]
    fn retire_refuses_a_foreign_lock_and_moves_nothing() {
        let (_first_dir, first) = temp_root();
        let (_second_dir, second) = temp_root();
        let (_first, foreign) = locked(&first);
        let (instance, lock) = locked(&second);
        let dir = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell is created");
        expect_match!(instance.retire(&foreign, dir), Err(RetireError::ForeignLock { path }) => {
            assert_eq!(path, first.join("controller.lock"))
        });
        assert!(exists(&second.join("cells/cell-1")));
        assert!(!exists(&second.join("retired")));
    }

    #[test]
    fn a_socket_named_launched_is_unreadable() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let _socket =
            UnixListener::bind(root.join("cells/cell-1/launched")).expect("the socket binds");
        expect_match!(dir.read_launched(), MarkerRead::Unreadable(MarkerFault::NotRegular) => ());
    }

    #[test]
    fn has_created_reports_a_directory_it_cannot_search_as_io() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let restore = restrict(&root.join("cells/cell-1"), 0o000);
        let result = dir.has_created();
        drop(restore);
        expect_match!(result, Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, root.join("cells/cell-1/created"));
            assert_eq!(source.raw_os_error(), Some(libc::EACCES));
        });
    }

    #[test]
    fn retire_of_a_cell_whose_cells_directory_is_gone_is_moved() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let dir = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell is created");
        fs::rename(root.join("cells"), root.join("aside")).expect("cells is moved aside");
        expect_match!(instance.retire(&lock, dir), Err(RetireError::Moved { path }) => {
            assert_eq!(path, root.join("cells/cell-1"))
        });
        assert!(exists(&root.join("aside/cell-1")));
    }

    #[test]
    fn retire_reports_io_failures_and_moves_nothing() {
        assert_not_root();
        for (blocked, mode, failed_at) in
            [("cells", 0o000, "cells"), ("cells", 0o500, "cells/cell-1")]
        {
            let (_dir, root) = temp_root();
            let (instance, lock) = locked(&root);
            let dir = instance
                .create_cell_dir(&lock, &cell("cell-1"))
                .expect("the cell is created");
            fs::create_dir_all(root.join("retired")).expect("retired is made");
            let restore = restrict(&root.join(blocked), mode);
            let result = instance.retire(&lock, dir);
            drop(restore);
            expect_match!(result, Err(RetireError::Io { path, source }) => {
                assert_eq!(path, root.join(failed_at), "{blocked} at {mode:o}");
                assert_eq!(source.raw_os_error(), Some(libc::EACCES), "{blocked} at {mode:o}");
            });
            assert!(exists(&root.join("cells/cell-1")), "{blocked} at {mode:o}");
            assert!(
                !exists(&root.join("retired/cell-1")),
                "{blocked} at {mode:o}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn set_file_flags(path: &Path, flags: libc::c_uint) -> io::Result<()> {
        let name = CString::new(path.as_os_str().as_bytes()).expect("a path without NUL");
        // SAFETY: `name` is a CString that lives until this function returns, and chflags keeps no
        // pointer to it.
        if unsafe { libc::chflags(name.as_ptr(), flags) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    struct ClearFlags(PathBuf);

    #[cfg(target_os = "macos")]
    impl Drop for ClearFlags {
        fn drop(&mut self) {
            let _ = set_file_flags(&self.0, 0);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn retire_reports_a_retired_it_can_neither_read_nor_make_private_and_moves_nothing() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let dir = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell is created");
        let retired = root.join("retired");
        fs::create_dir(&retired).expect("retired is made");
        let _restore = restrict(&retired, 0o000);
        set_file_flags(&retired, libc::UF_IMMUTABLE).expect("the owner sets the immutable flag");
        let clear = ClearFlags(retired.clone());
        let result = instance.retire(&lock, dir);
        drop(clear);
        expect_match!(result, Err(RetireError::Io { path, source }) => {
            assert_eq!(path, retired);
            assert_eq!(source.raw_os_error(), Some(libc::EPERM), "{source}");
        });
        assert_eq!(mode_of(&retired), 0o000);
        assert!(exists(&root.join("cells/cell-1")));
        assert!(!exists(&retired.join("cell-1")));
    }

    #[test]
    fn every_marker_and_retirement_error_names_what_it_refused_and_keeps_its_cause() {
        let cause = Some(eio().to_string());
        let fault = MarkerFault::Symlink.to_string();
        let rows: Vec<(Box<dyn Error>, &str, Option<String>)> = vec![
            (refused_at!(CellDirError::NotARegularFile), "/inst/x", None),
            (
                Box::new(CellDirError::MarkerUnreadable {
                    path: PathBuf::from("/inst/x"),
                    fault: MarkerFault::Symlink,
                }),
                "/inst/x",
                Some(fault),
            ),
            (refused_at!(RetireError::ForeignLock), "/inst/x", None),
            (
                refused_at!(RetireError::RetiredNotADirectory),
                "/inst/x",
                None,
            ),
            (refused_at!(RetireError::AlreadyRetired), "/inst/x", None),
            (refused_at!(RetireError::Moved), "/inst/x", None),
            (failed_at!(RetireError::Io), "/inst/x", cause.clone()),
            (Box::new(MarkerFault::Symlink), "symlink", None),
            (Box::new(MarkerFault::NotRegular), "regular file", None),
            (Box::new(MarkerFault::TooLarge), "4096", None),
            (Box::new(MarkerFault::Io(eio())), "marker", cause.clone()),
            (
                Box::new(MarkerFault::Malformed("an unknown key \"pid\"".into())),
                "an unknown key \"pid\"",
                None,
            ),
            (
                Box::new(MarkerFault::WrongCell {
                    found: cell("cell-2"),
                }),
                "cell-2",
                None,
            ),
        ];
        for (error, names, source) in rows {
            let text = error.to_string();
            assert!(text.contains(names), "{text:?} does not name {names:?}");
            assert_eq!(error.source().map(ToString::to_string), source, "{text}");
            if let Some(source) = source {
                assert!(text.contains(&source), "{text:?} omits its cause");
            }
        }
    }

    type ScanHook = Box<dyn FnMut()>;

    type SyncHook = Box<dyn FnMut(BorrowedFd<'_>)>;

    thread_local! {
        static BETWEEN_LISTINGS: RefCell<Option<ScanHook>> = const { RefCell::new(None) };
        static BEFORE_NAMING_THE_LOCK: RefCell<Option<ScanHook>> = const { RefCell::new(None) };
        static AFTER_NAMING_THE_LOCK: RefCell<Option<ScanHook>> = const { RefCell::new(None) };
        static ON_SYNC: RefCell<Option<SyncHook>> = const { RefCell::new(None) };
        static FAIL_NEXT_PROBE_LOCK: Cell<Option<i32>> = const { Cell::new(None) };
    }

    pub(super) fn probe_lock_result(locked: io::Result<()>) -> io::Result<()> {
        match FAIL_NEXT_PROBE_LOCK.with(Cell::take) {
            Some(errno) => Err(io::Error::from_raw_os_error(errno)),
            None => locked,
        }
    }

    fn fail_next_probe_lock(errno: i32) {
        FAIL_NEXT_PROBE_LOCK.with(|fail| fail.set(Some(errno)));
    }

    pub(super) fn before_naming_the_lock() {
        BEFORE_NAMING_THE_LOCK.with(|slot| {
            if let Some(work) = slot.borrow_mut().as_mut() {
                work();
            }
        });
    }

    fn on_naming_the_lock(work: impl FnMut() + 'static) {
        BEFORE_NAMING_THE_LOCK.with(|slot| *slot.borrow_mut() = Some(Box::new(work)));
    }

    pub(super) fn after_naming_the_lock() {
        AFTER_NAMING_THE_LOCK.with(|slot| {
            if let Some(work) = slot.borrow_mut().as_mut() {
                work();
            }
        });
    }

    fn once_the_lock_is_named(work: impl FnMut() + 'static) {
        AFTER_NAMING_THE_LOCK.with(|slot| *slot.borrow_mut() = Some(Box::new(work)));
    }

    pub(super) fn between_listings() {
        BETWEEN_LISTINGS.with(|slot| {
            if let Some(work) = slot.borrow_mut().as_mut() {
                work();
            }
        });
    }

    fn on_every_scan(work: impl FnMut() + 'static) {
        BETWEEN_LISTINGS.with(|slot| *slot.borrow_mut() = Some(Box::new(work)));
    }

    fn hooked_sync(fd: BorrowedFd<'_>) -> io::Result<()> {
        ON_SYNC.with(|slot| {
            if let Some(work) = slot.borrow_mut().as_mut() {
                work(fd);
            }
        });
        Ok(())
    }

    fn on_every_sync(work: impl FnMut(BorrowedFd<'_>) + 'static) {
        ON_SYNC.with(|slot| *slot.borrow_mut() = Some(Box::new(work)));
    }

    fn on_first_scan(work: impl FnOnce() + 'static) {
        let mut work = Some(work);
        on_every_scan(move || {
            if let Some(work) = work.take() {
                work();
            }
        });
    }

    fn allocated(instance: &InstanceRoot, lock: &WriterLock) -> CellId {
        instance
            .allocate_cell(lock)
            .expect("a cell is allocated")
            .cell()
            .clone()
    }

    fn entries(root: &Path, place: &str, names: &[&str]) {
        for name in names {
            fs::create_dir_all(root.join(place).join(name)).expect("the entry is made");
        }
    }

    fn names_under(path: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(path)
            .expect("the directory lists")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_first_cell_is_cell_1_whatever_is_missing() {
        for made in [
            &[][..],
            &["cells"][..],
            &["retired"][..],
            &["cells", "retired"][..],
        ] {
            let (_dir, root) = temp_root();
            for name in made {
                fs::create_dir(root.join(name)).expect("the directory is made");
            }
            let (instance, lock) = locked(&root);
            let dir = instance.allocate_cell(&lock).expect("a cell is allocated");
            assert_eq!(dir.cell(), &cell("cell-1"), "{made:?}");
            assert_eq!(
                dir.journal_path(),
                root.join("cells/cell-1/ledger.ndjson"),
                "{made:?}"
            );
            assert_eq!(mode_of(&root.join("cells/cell-1")), 0o700, "{made:?}");
            assert_eq!(names_under(&root.join("cells")), ["cell-1"], "{made:?}");
        }
    }

    #[test]
    fn the_next_cell_is_one_above_the_largest_number_in_cells_or_retired() {
        for (cells, retired, next) in [
            (&["cell-2"][..], &["cell-7"][..], "cell-8"),
            (&["cell-12", "cell-3"][..], &["cell-9"][..], "cell-13"),
            (&["cell-1"][..], &[][..], "cell-2"),
        ] {
            let (_dir, root) = temp_root();
            entries(&root, "cells", cells);
            entries(&root, "retired", retired);
            let (instance, lock) = locked(&root);
            assert_eq!(
                allocated(&instance, &lock),
                cell(next),
                "{cells:?} {retired:?}"
            );
        }
    }

    #[test]
    fn names_that_are_not_cell_ids_do_not_count() {
        let (_dir, root) = temp_root();
        entries(
            &root,
            "cells",
            &["cell-2", "cell-07", "cell-x", "cell-0", "CELL-08", "cell-"],
        );
        entries(
            &root,
            "retired",
            &["cell-10000000000", "cell-99999999999", "notes", "cell-5 "],
        );
        fs::write(root.join("cells/cell-30.tmp"), b"").expect("the file is made");
        let (instance, lock) = locked(&root);
        assert_eq!(allocated(&instance, &lock), cell("cell-3"));
    }

    #[test]
    fn a_name_that_differs_from_a_cell_id_only_in_the_case_of_its_prefix_counts_as_that_id() {
        let rows = [
            ("cells", &["cell-2", "CELL-3"][..], "cell-4"),
            ("retired", &["Cell-3"][..], "cell-4"),
            ("cells", &["cElL-12", "cell-3"][..], "cell-13"),
        ];
        let found: Vec<_> = rows
            .iter()
            .map(|&(place, names, _)| {
                let (_dir, root) = temp_root();
                entries(&root, place, names);
                let (mut instance, lock) = locked(&root);
                instance.sync = no_sync;
                let outcome = instance
                    .allocate_cell(&lock)
                    .map(|dir| dir.cell().as_str().to_owned())
                    .map_err(|error| error.to_string());
                (place, names, outcome)
            })
            .collect();
        let wanted: Vec<_> = rows
            .iter()
            .map(|&(place, names, next)| (place, names, Ok(next.to_owned())))
            .collect();
        assert_eq!(found, wanted);
    }

    #[test]
    fn a_taken_name_the_listing_cannot_see_makes_the_allocation_try_the_next_number() {
        let (_dir, root) = temp_root();
        entries(&root, "cells", &["cell-2"]);
        let (mut instance, lock) = locked(&root);
        instance.sync = hooked_sync;
        let scans = hide_after_each_listing(&root, "cell-3");
        assert_eq!(allocated(&instance, &lock), cell("cell-4"));
        assert_eq!(scans.get(), 2);
        assert_eq!(names_under(&root.join("cells")), ["cell-2", "cell-4"]);
    }

    #[test]
    fn a_hidden_cell_9999999999_leaves_no_id_and_names_cells() {
        for place in ["cells", "retired"] {
            let (_dir, root) = temp_root();
            fs::create_dir(root.join("cells")).expect("cells is made");
            entries(&root, place, &["cell-9999999998"]);
            let (mut instance, lock) = locked(&root);
            instance.sync = hooked_sync;
            let scans = hide_after_each_listing(&root, "cell-9999999999");
            expect_match!(instance.allocate_cell(&lock), Err(AllocateError::Exhausted { path }) => {
                assert_eq!(path, root.join("cells"), "{place}")
            });
            assert_eq!(scans.get(), 2, "{place}");
            assert!(
                names_under(&root.join("cells")).contains(&"cell-9999999999".to_owned()),
                "{place}"
            );
        }
    }

    fn hide_after_each_listing(root: &Path, name: &str) -> Rc<Cell<usize>> {
        let cells = root.join("cells");
        let cells_identity = identity_at(&cells);
        let hidden = cells.join(name);
        let scans = Rc::new(Cell::new(0));
        let (counted, planted) = (Rc::clone(&scans), hidden.clone());
        on_every_scan(move || {
            counted.set(counted.get() + 1);
            fs::create_dir(&planted).expect("the hidden name is made after cells/ is listed");
        });
        on_every_sync(move |fd| {
            let synced = fs::File::from(fd.try_clone_to_owned().expect("the descriptor is copied"))
                .metadata()
                .expect("the synced directory has metadata");
            if identity(&synced) == cells_identity {
                fs::remove_dir(&hidden)
                    .expect("the hidden name is removed before the next listing");
            }
        });
        scans
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_name_that_is_not_utf8_does_not_count() {
        let (_dir, root) = temp_root();
        entries(&root, "retired", &["cell-2"]);
        fs::create_dir(root.join("retired").join(OsStr::from_bytes(b"cell-9\xff")))
            .expect("the entry is made");
        let (instance, lock) = locked(&root);
        assert_eq!(allocated(&instance, &lock), cell("cell-3"));
    }

    #[test]
    fn a_retired_id_is_never_allocated_again() {
        let (_dir, root) = temp_root();
        entries(&root, "retired", &["cell-3"]);
        let (instance, lock) = locked(&root);
        let dir = instance.allocate_cell(&lock).expect("a cell is allocated");
        assert_eq!(dir.cell(), &cell("cell-4"));
        instance.retire(&lock, dir).expect("the cell retires");
        assert!(names_under(&root.join("cells")).is_empty());
        assert_eq!(allocated(&instance, &lock), cell("cell-5"));
    }

    #[test]
    fn every_entry_counts_by_its_name_and_none_is_followed() {
        let (_dir, root) = temp_root();
        let elsewhere = root.join("elsewhere");
        fs::create_dir(&elsewhere).expect("the link target is made");
        for place in ["cells", "retired"] {
            fs::create_dir(root.join(place)).expect("the directory is made");
        }
        fs::write(root.join("retired/cell-5"), b"").expect("the file is made");
        symlink(&elsewhere, root.join("cells/cell-6")).expect("the link is made");
        symlink(root.join("nowhere"), root.join("retired/cell-4")).expect("the link is made");
        make_fifo(&root.join("cells/cell-2"));
        let (instance, lock) = locked(&root);
        assert_eq!(allocated(&instance, &lock), cell("cell-7"));
        assert!(names_under(&elsewhere).is_empty());
        assert!(!exists(&root.join("nowhere")));
    }

    #[test]
    fn a_name_taken_after_the_scan_makes_the_allocation_scan_again() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let scans = Rc::new(Cell::new(0));
        let (counted, cells) = (Rc::clone(&scans), root.join("cells"));
        on_every_scan(move || {
            counted.set(counted.get() + 1);
            if counted.get() == 1 {
                for name in ["cell-1", "cell-4"] {
                    fs::create_dir_all(cells.join(name)).expect("the racing cell is made");
                }
            }
        });
        assert_eq!(allocated(&instance, &lock), cell("cell-5"));
        assert_eq!(scans.get(), 2);
        assert_eq!(
            names_under(&root.join("cells")),
            ["cell-1", "cell-4", "cell-5"]
        );
    }

    #[test]
    fn sixteen_allocations_at_once_get_distinct_contiguous_ids() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let mut numbers: Vec<u64> = thread::scope(|scope| {
            let workers: Vec<_> = (0..16)
                .map(|_| scope.spawn(|| allocated(&instance, &lock)))
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().expect("an allocation thread"))
                .map(|id| parse_cell_id(id.as_str()).expect("a cell ID"))
                .collect()
        });
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=16).collect::<Vec<_>>());
        assert_eq!(names_under(&root.join("cells")).len(), 16);
    }

    #[test]
    fn an_allocation_waits_for_the_one_in_progress_and_never_takes_its_id() {
        const WAIT: Duration = Duration::from_millis(300);
        for through_another_open in [false, true] {
            let (_dir, root) = temp_root();
            let mut opened_first = open_root(&root);
            opened_first.sync = hooked_sync;
            let instance = Arc::new(opened_first);
            let lock = Arc::new(instance.lock().expect("the writer lock is taken"));
            let second = if through_another_open {
                Arc::new(open_root(&root))
            } else {
                Arc::clone(&instance)
            };
            let (listed, listing) = mpsc::channel();
            let (finished, second_cell) = mpsc::channel();
            let listing = Rc::new(listing);
            let early = Rc::new(RefCell::new(Vec::new()));
            let (watching, recording) = (Rc::clone(&listing), Rc::clone(&early));
            let second_lock = Arc::clone(&lock);
            let second_scans = Arc::new(AtomicUsize::new(0));
            let counting = Arc::clone(&second_scans);
            on_first_scan(move || {
                thread::spawn(move || {
                    on_every_scan(move || {
                        counting.fetch_add(1, Ordering::SeqCst);
                        let _ = listed.send(());
                    });
                    let _ = finished.send(allocated(&second, &second_lock));
                });
                if watching.recv_timeout(WAIT).is_ok() {
                    recording.borrow_mut().push("while the first listed");
                }
            });
            let (watching, recording) = (Rc::clone(&listing), Rc::clone(&early));
            let mut paused = false;
            on_every_sync(move |_| {
                if !std::mem::replace(&mut paused, true) && watching.recv_timeout(WAIT).is_ok() {
                    recording
                        .borrow_mut()
                        .push("after the first listed and before its mkdir");
                }
            });
            let first = allocated(&instance, &lock);
            let taken = second_cell
                .recv_timeout(DEADLINE)
                .expect("the waiting allocation finishes");
            assert!(
                early.borrow().is_empty(),
                "the second allocation listed cells/ {:?}; through another open: \
                 {through_another_open}",
                early.borrow()
            );
            assert_eq!(
                second_scans.load(Ordering::SeqCst),
                1,
                "the second allocation listed again, so its first listing of cells/ ran before the \
                 first cell existed; through another open: {through_another_open}"
            );
            assert_eq!(
                (first, taken),
                (cell("cell-1"), cell("cell-2")),
                "through another open: {through_another_open}"
            );
        }
    }

    #[test]
    fn a_cell_retired_between_the_two_listings_is_never_allocated_again() {
        let (_dir, root) = temp_root();
        entries(&root, "cells", &["cell-1", "cell-2"]);
        let (instance, lock) = locked(&root);
        let (instance, lock) = (Rc::new(instance), Rc::new(lock));
        let (retiring, retiring_lock) = (Rc::clone(&instance), Rc::clone(&lock));
        on_first_scan(move || {
            let newest = retiring
                .cell_dir(&cell("cell-2"))
                .expect("the newest cell opens");
            retiring
                .retire(&retiring_lock, newest)
                .expect("the newest cell retires");
        });
        assert_eq!(allocated(&instance, &lock), cell("cell-3"));
        assert_eq!(names_under(&root.join("retired")), ["cell-2"]);
        assert_eq!(names_under(&root.join("cells")), ["cell-1", "cell-3"]);
    }

    #[test]
    fn a_panic_during_an_allocation_does_not_stop_the_next() {
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        on_first_scan(|| panic!("an allocation panics after its scan"));
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            instance.allocate_cell(&lock)
        }));
        assert!(panicked.is_err(), "the injected panic was not raised");
        assert_eq!(allocated(&instance, &lock), cell("cell-1"));
    }

    #[test]
    fn the_largest_cell_id_is_allocated_once_and_then_none_is_left() {
        for place in ["cells", "retired"] {
            let (_dir, root) = temp_root();
            entries(&root, place, &["cell-9999999998"]);
            let (instance, lock) = locked(&root);
            assert_eq!(
                allocated(&instance, &lock),
                cell("cell-9999999999"),
                "{place}"
            );
            let before = names_under(&root.join("cells"));
            expect_match!(instance.allocate_cell(&lock), Err(AllocateError::Exhausted { path }) => {
                assert_eq!(path, root.join("cells"), "{place}")
            });
            assert_eq!(names_under(&root.join("cells")), before, "{place}");
        }
        for place in ["cells", "retired"] {
            let (_dir, root) = temp_root();
            entries(&root, place, &["cell-9999999999"]);
            let (instance, lock) = locked(&root);
            expect_match!(instance.allocate_cell(&lock), Err(AllocateError::Exhausted { path }) => {
                assert_eq!(path, root.join(place), "{place}")
            });
        }
    }

    #[test]
    fn a_sync_that_fails_after_the_mkdir_names_the_cell_it_created() {
        let (_dir, root) = temp_root();
        fs::create_dir(root.join("cells")).expect("cells is made");
        let (instance, lock) = recording_locked(&root);
        fail_next_sync_of(&root.join("cells"));
        let result = instance.allocate_cell(&lock);
        let text = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        expect_match!(result, Err(AllocateError::Create { cell: created, source: CellDirError::Io { path, source } }) => {
            assert_eq!(created, cell("cell-1"));
            assert_eq!(path, root.join("cells"));
            assert_injected(&source);
        });
        assert!(text.contains("cell-1"), "{text:?} does not name the cell");
        assert!(root.join("cells/cell-1").is_dir());
        assert_eq!(allocated(&instance, &lock), cell("cell-2"));
    }

    #[test]
    fn an_allocation_that_cannot_make_cells_names_its_cell_but_creates_none() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let (instance, lock) = locked(&root);
        let restore = restrict(&root, 0o500);
        let result = instance.allocate_cell(&lock);
        drop(restore);
        expect_match!(result, Err(AllocateError::Create { cell: chosen, source: CellDirError::Io { path, source } }) => {
            assert_eq!(chosen, cell("cell-1"));
            assert_eq!(path, root.join("cells"));
            assert_eq!(source.raw_os_error(), Some(libc::EACCES));
        });
        assert!(!exists(&root.join("cells")), "cells was made");
        assert!(!exists(&root.join("cells/cell-1")), "cell-1 was made");
    }

    #[test]
    fn a_symlinked_or_non_directory_cells_or_retired_refuses_and_follows_nothing() {
        for place in ["cells", "retired"] {
            for planted in ["a symlink", "a regular file"] {
                let (_dir, root) = temp_root();
                let elsewhere = root.join("elsewhere");
                entries(&root, "elsewhere", &["cell-3"]);
                set_mode(&elsewhere, 0o755);
                if planted == "a symlink" {
                    symlink(&elsewhere, root.join(place)).expect("the link is made");
                } else {
                    fs::write(root.join(place), b"").expect("the file is made");
                }
                let (instance, lock) = locked(&root);
                let refused = instance.allocate_cell(&lock);
                if planted == "a symlink" {
                    expect_match!(refused, Err(AllocateError::Symlink { path }) => {
                        assert_eq!(path, root.join(place), "{place}: {planted}")
                    });
                } else {
                    expect_match!(refused, Err(AllocateError::NotADirectory { path }) => {
                        assert_eq!(path, root.join(place), "{place}: {planted}")
                    });
                }
                assert_eq!(names_under(&elsewhere), ["cell-3"], "{place}: {planted}");
                assert_eq!(mode_of(&elsewhere), 0o755, "{place}: {planted}");
                let other = if place == "cells" { "retired" } else { "cells" };
                assert!(!exists(&root.join(other)), "{place}: {planted}");
            }
        }
    }

    #[test]
    fn cells_swapped_for_a_symlink_after_the_scan_is_refused_and_not_followed() {
        let (_dir, root) = temp_root();
        entries(&root, "cells", &["cell-1"]);
        let (instance, lock) = locked(&root);
        let hook_root = root.clone();
        on_first_scan(move || {
            fs::rename(hook_root.join("cells"), hook_root.join("aside"))
                .expect("cells is moved aside");
            symlink(hook_root.join("aside"), hook_root.join("cells")).expect("the link is made");
        });
        expect_match!(instance.allocate_cell(&lock), Err(AllocateError::Create { cell: chosen, source: CellDirError::Symlink { path } }) => {
            assert_eq!(chosen, cell("cell-2"));
            assert_eq!(path, root.join("cells"));
        });
        assert_eq!(names_under(&root.join("aside")), ["cell-1"]);
    }

    #[test]
    fn a_cells_or_retired_the_umask_left_unreadable_is_made_private_and_counted() {
        assert_not_root();
        for place in ["cells", "retired"] {
            let (_dir, root) = temp_root();
            entries(&root, place, &["cell-5"]);
            let _restore = restrict(&root.join(place), 0o300);
            let (instance, lock) = locked(&root);
            assert_eq!(allocated(&instance, &lock), cell("cell-6"), "{place}");
            assert_eq!(mode_of(&root.join(place)), 0o700, "{place}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_retired_it_can_neither_read_nor_make_private_is_a_listing_error() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let retired = root.join("retired");
        fs::create_dir(&retired).expect("retired is made");
        let _restore = restrict(&retired, 0o000);
        set_file_flags(&retired, libc::UF_IMMUTABLE).expect("the owner sets the immutable flag");
        let clear = ClearFlags(retired.clone());
        let (instance, lock) = locked(&root);
        let result = instance.allocate_cell(&lock);
        drop(clear);
        expect_match!(result, Err(AllocateError::Listing { path, source }) => {
            assert_eq!(path, retired);
            assert_eq!(source.raw_os_error(), Some(libc::EPERM), "{source}");
        });
        assert!(!exists(&root.join("cells")));
    }

    #[test]
    fn a_foreign_lock_allocates_nothing() {
        let (_first_dir, first) = temp_root();
        let (_second_dir, second) = temp_root();
        let (_first, foreign) = locked(&first);
        let instance = open_root(&second);
        expect_match!(instance.allocate_cell(&foreign), Err(AllocateError::ForeignLock { path }) => {
            assert_eq!(path, first.join("controller.lock"))
        });
        assert!(!exists(&second.join("cells")));
    }

    #[test]
    fn an_allocation_that_loses_every_race_gives_up_after_64_scans() {
        let (_dir, root) = temp_root();
        let (mut instance, lock) = locked(&root);
        instance.sync = no_sync;
        let scans = Rc::new(Cell::new(0));
        let (counted, cells) = (Rc::clone(&scans), root.join("cells"));
        on_every_scan(move || {
            counted.set(counted.get() + 1);
            assert!(counted.get() <= 100, "the allocation never gives up");
            fs::create_dir_all(cells.join(format!("cell-{}", counted.get())))
                .expect("the racing cell is made");
        });
        expect_match!(instance.allocate_cell(&lock), Err(AllocateError::Contended { path }) => {
            assert_eq!(path, root.join("cells"))
        });
        assert_eq!(scans.get(), 64);
        assert_eq!(names_under(&root.join("cells")).len(), 64);
    }

    fn lock_path(root: &Path) -> PathBuf {
        root.join("cells/cell-1/supervisor.lock")
    }

    fn log_path(root: &Path) -> PathBuf {
        root.join("cells/cell-1/membraned.log")
    }

    fn lock_temporary_path(root: &Path) -> PathBuf {
        root.join("cells/cell-1/supervisor.lock.tmp")
    }

    /// Probes until a lock whose descriptors this test has closed reads `Free`. A process another
    /// test thread is spawning holds a copy of every descriptor until its exec closes it, and the
    /// lock stays held until then.
    fn free_once_released(dir: &CellDir) -> SupervisorLock {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match dir.probe_supervisor_lock() {
                Ok(LockProbe::Free(lock)) => return lock,
                Ok(LockProbe::Held) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1))
                }
                other => panic!("expected the released lock to probe Free, got {other:?}"),
            }
        }
    }

    fn free_test_once_released(dir: &CellDir) -> LockTest {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match dir.test_supervisor_lock() {
                LockTest::Held if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1))
                }
                free @ LockTest::Free(_) => return free,
                other => panic!("expected the released lock to test Free, got {other:?}"),
            }
        }
    }

    #[allow(unsafe_code)]
    fn descriptor_flags(fd: BorrowedFd<'_>, command: libc::c_int) -> libc::c_int {
        // SAFETY: fcntl with F_GETFD or F_GETFL takes no pointer, and the BorrowedFd keeps `fd`
        // open for the call.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), command) };
        assert!(flags >= 0, "fcntl: {}", io::Error::last_os_error());
        flags
    }

    fn assert_close_on_exec(fd: BorrowedFd<'_>, what: &str) {
        assert_ne!(
            descriptor_flags(fd, libc::F_GETFD) & libc::FD_CLOEXEC,
            0,
            "the {what} descriptor would leak into a spawned process"
        );
    }

    #[test]
    fn create_supervisor_lock_makes_a_private_close_on_exec_file_and_holds_it() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        let supervisor = dir
            .create_supervisor_lock()
            .expect("the supervisor lock is created");
        let metadata = fs::symlink_metadata(&path).expect("the lock file exists");
        assert!(metadata.is_file(), "{metadata:?}");
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_close_on_exec(supervisor.as_fd(), "supervisor lock");
        expect_match!(dir.probe_supervisor_lock(), Ok(LockProbe::Held) => ());
        assert!(
            !a_separate_open_can_lock(&path),
            "the lock is held while the SupervisorLock lives"
        );
        assert!(
            !a_separate_open_can_share(&path),
            "the lock is exclusive, so a shared lock is refused too"
        );
        let held = identity_at(&path);
        expect_match!(dir.create_supervisor_lock(), Err(CellDirError::AlreadyExists { path: refused }) => {
            assert_eq!(refused, path)
        });
        assert_eq!(
            identity_at(&path),
            held,
            "the refused create replaced the lock file"
        );
        assert!(!exists(&lock_temporary_path(&root)));
        assert!(
            format!("{supervisor:?}").contains("supervisor.lock"),
            "{supervisor:?}"
        );
    }

    #[test]
    fn create_supervisor_lock_syncs_the_cell_directory_after_it_names_the_lock() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        let _supervisor = dir
            .create_supervisor_lock()
            .expect("the supervisor lock is created");
        let cell_path = root.join("cells/cell-1");
        assert_eq!(syncs_of(&cell_path), 1);
        assert_eq!(
            last_seen(&cell_path).named(c"supervisor.lock"),
            Some(identity_at(&lock_path(&root))),
            "the cell directory was synced before it named supervisor.lock"
        );
        assert_eq!(
            last_seen(&cell_path).named(c"supervisor.lock.tmp"),
            None,
            "the cell directory was synced while it still named the temporary"
        );
    }

    #[test]
    fn a_failed_sync_after_create_supervisor_lock_leaves_a_free_lock_file() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        let cell_path = root.join("cells/cell-1");
        fail_next_sync_of(&cell_path);
        expect_match!(dir.create_supervisor_lock(), Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, cell_path);
            assert_injected(&source);
        });
        assert!(
            fs::symlink_metadata(lock_path(&root))
                .expect("the lock file is in place")
                .is_file()
        );
        drop(free_once_released(&dir));
        expect_match!(dir.create_supervisor_lock(), Err(CellDirError::AlreadyExists { .. }) => ());
    }

    #[test]
    fn a_dropped_lock_probes_free_and_the_probe_keeps_it() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        drop(
            dir.create_supervisor_lock()
                .expect("the supervisor lock is created"),
        );
        let kept = free_once_released(&dir);
        expect_match!(dir.probe_supervisor_lock(), Ok(LockProbe::Held) => ());
        assert!(
            !a_separate_open_can_lock(&path),
            "a Free probe keeps the lock"
        );
        assert_close_on_exec(kept.as_fd(), "probed lock");
        drop(kept);
        drop(free_once_released(&dir));
    }

    #[test]
    fn a_probe_racing_the_creation_never_takes_the_new_lock_first() {
        const CREATES: u32 = 500;
        const PROBERS: usize = 4;
        let (_dir, root) = temp_root();
        let (mut instance, lock) = locked(&root);
        instance.sync = no_sync;
        let dir = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell is created");
        let path = lock_path(&root);
        let creating = AtomicBool::new(true);
        let (lost, probes) = thread::scope(|scope| {
            let probers: Vec<_> = (0..PROBERS)
                .map(|_| {
                    scope.spawn(|| {
                        let mut probes = 0_u32;
                        while creating.load(Ordering::Relaxed) {
                            drop(dir.probe_supervisor_lock());
                            probes += 1;
                        }
                        probes
                    })
                })
                .collect();
            let mut lost = 0_u32;
            for _ in 0..CREATES {
                match dir.create_supervisor_lock() {
                    Ok(held) => drop(held),
                    Err(CellDirError::Io { source, .. })
                        if source.raw_os_error() == Some(libc::EWOULDBLOCK) =>
                    {
                        lost += 1;
                    }
                    Err(other) => panic!("the supervisor lock was not created: {other}"),
                }
                fs::remove_file(&path).expect("the lock file is removed");
            }
            creating.store(false, Ordering::Relaxed);
            let probes: u32 = probers
                .into_iter()
                .map(|prober| prober.join().expect("a prober finishes"))
                .sum();
            (lost, probes)
        });
        assert_eq!(
            lost, 0,
            "{lost} of {CREATES} creations lost their new lock to {PROBERS} probers that ran \
             {probes} times"
        );
    }

    #[test]
    fn a_temporary_swapped_between_the_lock_and_the_rename_returns_no_lock() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let swapped = Rc::new(Cell::new(None));
        let (recorded, temporary) = (Rc::clone(&swapped), lock_temporary_path(&root));
        on_naming_the_lock(move || {
            fs::remove_file(&temporary).expect("this call's temporary is removed");
            let other = fs::File::create(&temporary).expect("a second creator's file is made");
            recorded.set(Some(identity(
                &other.metadata().expect("the second file has metadata"),
            )));
        });
        let result = dir.create_supervisor_lock();
        let text = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        expect_match!(result, Err(CellDirError::LockNotNamed { path }) => {
            assert_eq!(path, lock_path(&root));
        });
        assert!(text.contains("cell-1"), "{text:?} does not name the cell");
        assert_eq!(
            Some(identity_at(&lock_path(&root))),
            swapped.get(),
            "supervisor.lock names the second creator's file, which is left as it is"
        );
        assert!(!exists(&lock_temporary_path(&root)));
        drop(free_once_released(&dir));
    }

    #[test]
    fn a_lock_whose_name_is_removed_before_the_check_returns_no_lock() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let named = lock_path(&root);
        once_the_lock_is_named(move || {
            fs::remove_file(&named).expect("the lock's name is removed");
        });
        expect_match!(dir.create_supervisor_lock(), Err(CellDirError::Io { path, source }) => {
            assert_eq!(path, lock_path(&root));
            assert_eq!(source.kind(), io::ErrorKind::NotFound);
        });
        assert!(!exists(&lock_path(&root)));
        assert!(!exists(&lock_temporary_path(&root)));
        expect_match!(dir.probe_supervisor_lock(), Ok(LockProbe::Missing) => ());
    }

    #[test]
    fn two_creators_of_one_lock_never_get_a_lock_its_name_does_not_hold() {
        const ROUNDS: usize = 300;
        let (_dir, root) = temp_root();
        let (mut instance, lock) = locked(&root);
        instance.sync = no_sync;
        let dir = instance
            .create_cell_dir(&lock, &cell("cell-1"))
            .expect("the cell is created");
        let mut wrong = Vec::new();
        for round in 0..ROUNDS {
            let start = Barrier::new(2);
            let created: Vec<_> = thread::scope(|scope| {
                let creators: Vec<_> = (0..2)
                    .map(|_| {
                        scope.spawn(|| {
                            start.wait();
                            dir.create_supervisor_lock()
                        })
                    })
                    .collect();
                creators
                    .into_iter()
                    .map(|creator| creator.join().expect("a creator finishes"))
                    .collect()
            });
            let named = dir.supervisor_lock_identity().ok();
            let held: Vec<_> = created
                .iter()
                .filter_map(|outcome| outcome.as_ref().ok())
                .map(|held| held.identity().expect("the held file has an identity"))
                .collect();
            if held.len() > 1 || held.iter().any(|held| Some(*held) != named) {
                wrong.push(round);
            }
            drop(created);
            for leftover in [lock_path(&root), lock_temporary_path(&root)] {
                match fs::remove_file(&leftover) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    removed => removed.expect("the round's files are removed"),
                }
            }
        }
        assert_eq!(
            wrong,
            Vec::<usize>::new(),
            "in these of {ROUNDS} rounds a creator got a lock supervisor.lock does not name"
        );
    }

    #[test]
    fn a_directory_at_the_lock_temporary_refuses_every_create_and_names_it() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let temporary = lock_temporary_path(&root);
        fs::create_dir(&temporary).expect("the directory is made");
        fs::write(temporary.join("kept"), b"inside").expect("a file is made inside it");
        for attempt in ["first", "second"] {
            expect_match!(dir.create_supervisor_lock(), Err(CellDirError::Io { path, source }) => {
                assert_eq!(path, temporary, "{attempt}");
                assert_eq!(source.raw_os_error(), Some(UNLINK_OF_A_DIRECTORY), "{attempt}");
            });
        }
        expect_match!(dir.probe_supervisor_lock(), Ok(LockProbe::Missing) => ());
        assert!(!exists(&lock_path(&root)));
        assert_eq!(
            fs::read(temporary.join("kept")).ok(),
            Some(b"inside".to_vec())
        );
    }

    #[cfg(target_os = "macos")]
    const UNLINK_OF_A_DIRECTORY: i32 = libc::EPERM;

    #[cfg(target_os = "linux")]
    const UNLINK_OF_A_DIRECTORY: i32 = libc::EISDIR;

    #[test]
    fn a_leftover_temporary_is_removed_without_being_followed() {
        for planted in ["a file", "a symlink", "a dangling symlink"] {
            let (_dir, root) = temp_root();
            let dir = opened(&root, "cell-1");
            let temporary = lock_temporary_path(&root);
            let target = root.join("elsewhere");
            match planted {
                "a file" => {
                    fs::write(&temporary, b"left by a crash").expect("the leftover is made")
                }
                "a symlink" => {
                    fs::write(&target, b"not the lock").expect("the link target is made");
                    symlink(&target, &temporary).expect("the link is made");
                }
                _ => symlink(&target, &temporary).expect("the link is made"),
            }
            let supervisor = dir
                .create_supervisor_lock()
                .unwrap_or_else(|error| panic!("{planted}: {error}"));
            assert!(!exists(&temporary), "{planted}");
            assert!(
                fs::symlink_metadata(lock_path(&root))
                    .expect("the lock file exists")
                    .is_file(),
                "{planted}"
            );
            assert_eq!(
                fs::read(lock_path(&root)).expect("the lock file reads"),
                b"",
                "{planted}"
            );
            expect_match!(dir.probe_supervisor_lock(), Ok(LockProbe::Held) => ());
            if planted == "a symlink" {
                assert_eq!(
                    fs::read(&target).expect("the target reads"),
                    b"not the lock"
                );
            } else {
                assert!(!exists(&target), "{planted}");
            }
            drop(supervisor);
        }
    }

    #[test]
    fn with_no_lock_file_the_probe_is_missing_and_creates_nothing() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        expect_match!(dir.probe_supervisor_lock(), Ok(LockProbe::Missing) => ());
        expect_match!(dir.supervisor_lock_identity(), Err(CellDirError::Missing { path }) => {
            assert_eq!(path, lock_path(&root))
        });
        assert!(!exists(&lock_path(&root)));
    }

    #[test]
    fn the_lock_is_held_while_any_copy_of_its_open_file_lives() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let supervisor = dir
            .create_supervisor_lock()
            .expect("the supervisor lock is created");
        let copy = supervisor
            .as_fd()
            .try_clone_to_owned()
            .expect("the descriptor is copied");
        drop(supervisor);
        expect_match!(dir.probe_supervisor_lock(), Ok(LockProbe::Held) => ());
        drop(copy);
        let handed = free_once_released(&dir).into_owned_fd();
        expect_match!(dir.probe_supervisor_lock(), Ok(LockProbe::Held) => ());
        drop(handed);
        drop(free_once_released(&dir));
    }

    #[test]
    fn a_symlink_named_supervisor_lock_is_refused_and_never_followed() {
        for target_exists in [true, false] {
            let (_dir, root) = temp_root();
            let dir = opened(&root, "cell-1");
            let target = root.join("elsewhere.lock");
            if target_exists {
                fs::write(&target, b"").expect("the link target is made");
            }
            symlink(&target, lock_path(&root)).expect("the link is made");
            expect_match!(dir.create_supervisor_lock(), Err(CellDirError::Symlink { path }) => {
                assert_eq!(path, lock_path(&root), "target exists: {target_exists}")
            });
            assert!(
                !exists(&lock_temporary_path(&root)),
                "target exists: {target_exists}"
            );
            expect_match!(dir.probe_supervisor_lock(), Err(CellDirError::Symlink { path }) => {
                assert_eq!(path, lock_path(&root), "target exists: {target_exists}")
            });
            expect_match!(dir.supervisor_lock_identity(), Err(CellDirError::Symlink { .. }) => ());
            assert_eq!(exists(&target), target_exists);
        }
    }

    #[test]
    fn a_symlink_named_membraned_log_is_refused_and_never_followed() {
        for target_exists in [true, false] {
            let (_dir, root) = temp_root();
            let dir = opened(&root, "cell-1");
            let target = root.join("elsewhere.log");
            if target_exists {
                fs::write(&target, b"kept\n").expect("the link target is made");
                set_mode(&target, 0o644);
            }
            symlink(&target, log_path(&root)).expect("the link is made");
            expect_match!(dir.open_supervisor_log(), Err(CellDirError::Symlink { path }) => {
                assert_eq!(path, log_path(&root), "target exists: {target_exists}")
            });
            if target_exists {
                assert_eq!(fs::read(&target).expect("the target reads"), b"kept\n");
                assert_eq!(mode_of(&target), 0o644);
            } else {
                assert!(!exists(&target), "the log was created through the link");
            }
        }
    }

    #[test]
    fn a_directory_fifo_or_socket_named_supervisor_lock_or_membraned_log_is_refused_without_blocking()
     {
        for planted in ["a directory", "a FIFO", "a socket"] {
            for name in ["supervisor.lock", "membraned.log"] {
                let (_dir, root) = temp_root();
                let dir = opened(&root, "cell-1");
                let path = root.join("cells/cell-1").join(name);
                let _socket = match planted {
                    "a directory" => {
                        fs::create_dir(&path).expect("the directory is made");
                        None
                    }
                    "a FIFO" => {
                        make_fifo(&path);
                        None
                    }
                    _ => Some(UnixListener::bind(&path).expect("the socket binds")),
                };
                let outcomes = within(&format!("opening {planted} as {name}"), move || {
                    if name == "supervisor.lock" {
                        vec![
                            dir.create_supervisor_lock().map(drop),
                            dir.probe_supervisor_lock().map(drop),
                            dir.supervisor_lock_identity().map(drop),
                        ]
                    } else {
                        vec![dir.open_supervisor_log().map(drop)]
                    }
                });
                for outcome in outcomes {
                    expect_match!(outcome, Err(CellDirError::NotARegularFile { path: refused }) => {
                        assert_eq!(refused, path, "{planted} as {name}")
                    });
                }
                assert!(!exists(&lock_temporary_path(&root)), "{planted} as {name}");
            }
        }
    }

    #[test]
    fn supervisor_lock_identity_names_the_file_the_lock_holds() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        let supervisor = dir
            .create_supervisor_lock()
            .expect("the supervisor lock is created");
        let held = supervisor.identity().expect("the held lock's identity");
        let (dev, ino) = identity_at(&path);
        assert_eq!(held, FileIdentity { dev, ino });
        assert_eq!(
            dir.supervisor_lock_identity()
                .expect("the named lock's identity"),
            held
        );
        fs::rename(&path, root.join("cells/cell-1/aside.lock")).expect("the lock is moved aside");
        fs::write(&path, b"").expect("another file takes its name");
        assert_ne!(
            dir.supervisor_lock_identity()
                .expect("the new file's identity"),
            held
        );
        assert_eq!(
            supervisor.identity().expect("the held lock's identity"),
            held
        );
    }

    #[test]
    fn open_supervisor_log_creates_a_private_log_and_appends_to_it() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = log_path(&root);
        dir.open_supervisor_log()
            .expect("the log is created")
            .write_all(b"first\n")
            .expect("the log is written");
        assert!(
            fs::symlink_metadata(&path)
                .expect("the log exists")
                .is_file()
        );
        assert_eq!(mode_of(&path), 0o600);
        set_mode(&path, 0o644);
        let mut log = dir.open_supervisor_log().expect("the log opens again");
        assert_eq!(mode_of(&path), 0o600, "an existing log is made private");
        let flags = descriptor_flags(log.as_fd(), libc::F_GETFL);
        assert_eq!(flags & libc::O_ACCMODE, libc::O_WRONLY);
        assert_ne!(
            flags & libc::O_APPEND,
            0,
            "the log is not opened for append"
        );
        assert_eq!(flags & libc::O_NONBLOCK, 0, "O_NONBLOCK was left set");
        assert_close_on_exec(log.as_fd(), "log");
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("another writer opens the log")
            .write_all(b"other\n")
            .expect("another writer appends");
        log.write_all(b"second\n").expect("the log is written");
        assert_eq!(
            fs::read(&path).expect("the log reads"),
            b"first\nother\nsecond\n"
        );
    }

    #[test]
    fn a_lock_or_log_it_cannot_open_is_an_io_error_naming_it() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        drop(
            dir.create_supervisor_lock()
                .expect("the supervisor lock is created"),
        );
        fs::write(log_path(&root), b"").expect("the log is made");
        set_mode(&lock_path(&root), 0o000);
        set_mode(&log_path(&root), 0o400);
        for (what, refused, path) in [
            (
                "probe",
                dir.probe_supervisor_lock().map(drop),
                lock_path(&root),
            ),
            (
                "identity",
                dir.supervisor_lock_identity().map(drop),
                lock_path(&root),
            ),
            ("log", dir.open_supervisor_log().map(drop), log_path(&root)),
        ] {
            expect_match!(refused, Err(CellDirError::Io { path: named, source }) => {
                assert_eq!(named, path, "{what}");
                assert_eq!(source.raw_os_error(), Some(libc::EACCES), "{what}: {source}");
            });
        }
    }

    #[test]
    fn a_lock_test_reads_a_lock_held_elsewhere_as_held() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let supervisor = dir
            .create_supervisor_lock()
            .expect("the supervisor lock is created");
        let held = identity_at(&lock_path(&root));
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Held, "{test:?}");
        expect_match!(test, LockTest::Held => ());
        assert_eq!(
            identity_at(&lock_path(&root)),
            held,
            "the test replaced the held lock file"
        );
        drop(supervisor);
    }

    #[test]
    fn a_lock_test_takes_an_unlocked_lock_file_as_free_and_keeps_it() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        drop(
            dir.create_supervisor_lock()
                .expect("the supervisor lock is created"),
        );
        let file = identity_at(&path);
        let test = free_test_once_released(&dir);
        assert_eq!(test.fact(), LockFact::Free, "{test:?}");
        expect_match!(dir.probe_supervisor_lock(), Ok(LockProbe::Held) => ());
        assert!(
            !a_separate_open_can_lock(&path),
            "a Free test keeps the lock"
        );
        assert_eq!(identity_at(&path), file, "the test replaced the lock file");
        drop(test);
        drop(free_once_released(&dir));
    }

    #[test]
    fn a_lock_test_creates_a_missing_lock_locked_and_free() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Free, "{test:?}");
        let metadata = fs::symlink_metadata(&path).expect("the lock file exists");
        assert!(metadata.is_file(), "{metadata:?}");
        assert_eq!(mode_of(&path), 0o600);
        expect_match!(dir.probe_supervisor_lock(), Ok(LockProbe::Held) => ());
        assert!(!exists(&lock_temporary_path(&root)));
        let (dev, ino) = identity_at(&path);
        expect_match!(test, LockTest::Free(lock) => {
            assert_eq!(
                lock.identity().expect("the held lock's identity"),
                FileIdentity { dev, ino },
                "the test holds a file supervisor.lock does not name"
            )
        });
    }

    #[test]
    fn a_directory_or_a_symlink_at_the_lock_is_untestable_and_left_alone() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        fs::create_dir(&path).expect("the directory is made");
        fs::write(path.join("kept"), b"inside").expect("a file is made inside it");
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Untestable, "{test:?}");
        expect_match!(&test, LockTest::Untestable(CellDirError::NotARegularFile { path: refused }) => {
            assert_eq!(refused.as_path(), path.as_path())
        });
        assert!(
            fs::symlink_metadata(&path)
                .expect("the directory is still there")
                .is_dir()
        );
        assert_eq!(fs::read(path.join("kept")).ok(), Some(b"inside".to_vec()));
        assert!(!exists(&lock_temporary_path(&root)));
        drop(test);

        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        let target = root.join("elsewhere.lock");
        fs::write(&target, b"not the lock").expect("the link target is made");
        symlink(&target, &path).expect("the link is made");
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Untestable, "{test:?}");
        expect_match!(&test, LockTest::Untestable(CellDirError::Symlink { path: refused }) => {
            assert_eq!(refused.as_path(), path.as_path())
        });
        assert!(
            fs::symlink_metadata(&path)
                .expect("the link is still there")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(&target).expect("the target reads"),
            b"not the lock"
        );
        assert!(
            a_separate_open_can_lock(&target),
            "the test locked the link's target"
        );
        assert!(!exists(&lock_temporary_path(&root)));
        drop(test);
    }

    #[test]
    fn a_lock_file_it_cannot_open_is_untestable() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        drop(
            dir.create_supervisor_lock()
                .expect("the supervisor lock is created"),
        );
        set_mode(&path, 0o000);
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Untestable, "{test:?}");
        expect_match!(test, LockTest::Untestable(CellDirError::Io { path: named, source }) => {
            assert_eq!(named, path);
            assert_eq!(source.raw_os_error(), Some(libc::EACCES), "{source}");
        });
        assert_eq!(mode_of(&path), 0o000);
        assert!(!exists(&lock_temporary_path(&root)));
    }

    #[test]
    fn a_missing_lock_it_cannot_create_in_a_read_only_cell_is_untestable() {
        assert_not_root();
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let _restore = restrict(&root.join("cells/cell-1"), 0o500);
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Untestable, "{test:?}");
        expect_match!(test, LockTest::Untestable(CellDirError::Io { path, source }) => {
            assert_eq!(path, lock_temporary_path(&root));
            assert_eq!(source.raw_os_error(), Some(libc::EACCES), "{source}");
        });
        assert!(!exists(&lock_path(&root)));
        assert!(!exists(&lock_temporary_path(&root)));
    }

    #[test]
    fn a_directory_at_the_lock_temporary_makes_a_missing_lock_untestable() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let temporary = lock_temporary_path(&root);
        fs::create_dir(&temporary).expect("the directory is made");
        fs::write(temporary.join("kept"), b"inside").expect("a file is made inside it");
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Untestable, "{test:?}");
        expect_match!(test, LockTest::Untestable(CellDirError::Io { path, source }) => {
            assert_eq!(path, temporary);
            assert_eq!(source.raw_os_error(), Some(UNLINK_OF_A_DIRECTORY), "{source}");
        });
        assert!(!exists(&lock_path(&root)));
        assert_eq!(
            fs::read(temporary.join("kept")).ok(),
            Some(b"inside".to_vec())
        );
    }

    #[test]
    fn a_lock_whose_cell_sync_fails_after_naming_is_untestable() {
        let (_dir, root) = temp_root();
        let (_instance, _lock, dir) = recording_cell(&root, "cell-1");
        let cell_path = root.join("cells/cell-1");
        fail_next_sync_of(&cell_path);
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Untestable, "{test:?}");
        expect_match!(test, LockTest::Untestable(CellDirError::Io { path, source }) => {
            assert_eq!(path, cell_path);
            assert_injected(&source);
        });
        assert!(
            fs::symlink_metadata(lock_path(&root))
                .expect("the lock file is in place")
                .is_file()
        );
        expect_match!(free_test_once_released(&dir), LockTest::Free(_) => ());
    }

    #[test]
    fn a_lock_whose_flock_fails_for_another_reason_is_untestable() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        drop(
            dir.create_supervisor_lock()
                .expect("the supervisor lock is created"),
        );
        drop(free_once_released(&dir));
        fail_next_probe_lock(libc::ENOLCK);
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Untestable, "{test:?}");
        expect_match!(test, LockTest::Untestable(CellDirError::Io { path: named, source }) => {
            assert_eq!(named, path);
            assert_eq!(source.raw_os_error(), Some(libc::ENOLCK), "{source}");
        });
        expect_match!(free_test_once_released(&dir), LockTest::Free(_) => ());
    }

    #[test]
    fn a_temporary_swapped_before_the_rename_makes_a_missing_lock_untestable() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        let mut swap = Some(lock_temporary_path(&root));
        on_naming_the_lock(move || {
            if let Some(temporary) = swap.take() {
                fs::remove_file(&temporary).expect("this call's temporary is removed");
                fs::File::create(&temporary).expect("a second creator's file is made");
            }
        });
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Untestable, "{test:?}");
        expect_match!(test, LockTest::Untestable(CellDirError::LockNotNamed { path: refused }) => {
            assert_eq!(refused, path)
        });
        assert!(
            a_separate_open_can_lock(&path),
            "the failed test kept a lock on the second creator's file"
        );
        assert!(!exists(&lock_temporary_path(&root)));
    }

    #[test]
    fn a_held_file_planted_before_the_rename_makes_a_missing_lock_untestable() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        let racer = Rc::new(RefCell::new(None));
        let (kept, planted) = (Rc::clone(&racer), path.clone());
        on_naming_the_lock(move || {
            if kept.borrow().is_some() {
                return;
            }
            let file = fs::File::create(&planted).expect("a file is planted at the name");
            lock_exclusive_nonblocking(file.as_fd()).expect("the planter locks its file");
            *kept.borrow_mut() = Some(file);
        });
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Untestable, "{test:?}");
        expect_match!(test, LockTest::Untestable(CellDirError::AlreadyExists { path: refused }) => {
            assert_eq!(refused, path)
        });
        let planter = racer
            .borrow_mut()
            .take()
            .expect("the planted file was made");
        assert_eq!(
            identity_at(&path),
            identity(&planter.metadata().expect("the planted file's metadata")),
            "the failed test replaced the planted file"
        );
        assert!(!exists(&lock_temporary_path(&root)));
        drop(planter);
    }

    #[test]
    fn a_temporary_named_by_a_second_creator_makes_a_missing_lock_untestable() {
        let (_dir, root) = temp_root();
        let dir = opened(&root, "cell-1");
        let path = lock_path(&root);
        let mut rename = Some((lock_temporary_path(&root), path.clone()));
        on_naming_the_lock(move || {
            if let Some((temporary, lock)) = rename.take() {
                fs::rename(&temporary, &lock).expect("a second creator names the temporary");
            }
        });
        let test = dir.test_supervisor_lock();
        assert_eq!(test.fact(), LockFact::Untestable, "{test:?}");
        expect_match!(test, LockTest::Untestable(CellDirError::Io { path: named, source }) => {
            assert_eq!(named, path);
            assert_eq!(source.kind(), io::ErrorKind::NotFound, "{source}");
        });
        assert!(
            fs::symlink_metadata(&path)
                .expect("the second creator's rename left a file at the name")
                .is_file()
        );
        assert!(!exists(&lock_temporary_path(&root)));
    }

    #[test]
    fn every_allocation_error_names_what_it_refused_and_keeps_its_cause() {
        let cause = Some(eio().to_string());
        let created = || CellDirError::Symlink {
            path: PathBuf::from("/inst/cells"),
        };
        let rows: Vec<(Box<dyn Error>, &str, Option<String>)> = vec![
            (refused_at!(AllocateError::ForeignLock), "/inst/x", None),
            (refused_at!(AllocateError::NotADirectory), "/inst/x", None),
            (refused_at!(AllocateError::Symlink), "/inst/x", None),
            (refused_at!(AllocateError::Contended), "/inst/x", None),
            (failed_at!(AllocateError::Listing), "/inst/x", cause),
            (refused_at!(AllocateError::Exhausted), "/inst/x", None),
            (
                Box::new(AllocateError::Create {
                    cell: cell("cell-4"),
                    source: created(),
                }),
                "cell-4",
                Some(created().to_string()),
            ),
        ];
        for (error, names, source) in rows {
            let text = error.to_string();
            assert!(text.contains(names), "{text:?} does not name {names:?}");
            assert_eq!(error.source().map(ToString::to_string), source, "{text}");
            if let Some(source) = source {
                assert!(text.contains(&source), "{text:?} omits its cause");
            }
        }
        let contended = AllocateError::Contended {
            path: PathBuf::from("/inst/cells"),
        };
        assert!(contended.to_string().contains("64"), "{contended}");
        let exhausted = AllocateError::Exhausted {
            path: PathBuf::from("/inst/cells"),
        };
        assert!(
            exhausted.to_string().contains("cell-9999999999"),
            "{exhausted}"
        );
        assert!(
            !contended.to_string().contains("another process"),
            "{contended} blames a process that may not exist"
        );
    }
}
