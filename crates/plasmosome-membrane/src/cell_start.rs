use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;

use plasmosome_backend::CellId;
use plasmosome_core::runtime::recipe::PlatformLimits;
use plasmosome_core::runtime::supervisor_config::{
    ConfigRecordRefusal, MAX_RECORD_BYTES, SupervisorConfig,
};
use plasmosome_core::{CellDir, CellDirError, FileIdentity, InstanceRoot, InstanceRootError};

use crate::cell_args::CellArgs;

const READ_CAP: u64 = MAX_RECORD_BYTES as u64 + 2;

/// Why `membraned` refuses to start a cell, before step 3 of spec 023 §5. Nothing that refuses
/// has created anything.
#[derive(Debug)]
pub enum CellStartRefusal {
    /// Reading descriptor 3 failed.
    Read(io::Error),
    /// The bytes read are not one config record.
    Record(ConfigRecordRefusal),
    /// The record names another cell than argv's `--cell=`.
    CellMismatch { argv: CellId, record: CellId },
    /// `flock(LOCK_EX | LOCK_NB)` on descriptor 4 would block: another open of the file holds
    /// its lock, so this descriptor is not the one the controller locked.
    LockNotHeld,
    /// Locking, flagging or examining descriptor 4 failed for another reason.
    Lock(io::Error),
    /// The record's instance root did not open.
    InstanceRoot(InstanceRootError),
    /// The cell directory or its `supervisor.lock` did not open by name, without following a
    /// symlink. A directory moved to `retired/` is `Missing`.
    CellDirectory(CellDirError),
    /// The file named `supervisor.lock` in the cell directory is not the file descriptor 4 has
    /// open.
    NotThisCellsLock,
}

/// What spec 023 §5 has `membraned` do before its step 1. Reads `config`, descriptor 3, until
/// EOF or until one byte more than the largest record has been read, then closes it, so a writer
/// that never stops cannot make it read more. Decodes what it read with [`SupervisorConfig::decode`] under `limits`, and
/// returns the record only when its `cell` is `args.cell`.
///
/// Refuses a failed read as `Read`, anything `decode` refuses as `Record`, and a record for
/// another cell as `CellMismatch`. It reads nothing else and creates nothing.
pub fn read_config(
    config: OwnedFd,
    args: &CellArgs,
    limits: &PlatformLimits,
) -> Result<SupervisorConfig, CellStartRefusal> {
    let mut record = Vec::new();
    File::from(config)
        .take(READ_CAP)
        .read_to_end(&mut record)
        .map_err(CellStartRefusal::Read)?;
    let config = SupervisorConfig::decode(&record, limits).map_err(CellStartRefusal::Record)?;
    if config.cell != args.cell {
        return Err(CellStartRefusal::CellMismatch {
            argv: args.cell.clone(),
            record: config.cell,
        });
    }
    Ok(config)
}

/// The supervisor lock `membraned` was handed, checked as this cell's, and the cell directory
/// it was checked against. Keep it for the life of `membraned`: dropping it closes descriptor 4,
/// and the lock is released once no other descriptor of that open file remains. Nothing here
/// calls `LOCK_UN`.
#[derive(Debug)]
pub struct AdoptedLock {
    _lock: File,
    cell_dir: CellDir,
}

impl AdoptedLock {
    /// The cell directory, opened by name from the record's instance root without following a
    /// symlink.
    pub fn cell_dir(&self) -> &CellDir {
        &self.cell_dir
    }
}

/// Spec 023 §5, steps 1 and 2. Takes `lock`, descriptor 4, and `config`, the record
/// [`read_config`] returned.
///
/// Takes `flock(LOCK_EX | LOCK_NB)` on the descriptor, which succeeds at once when it shares
/// the open file the controller locked, and sets its close-on-exec flag. Then opens
/// `config.instance_root` with [`InstanceRoot::open`] and `cells/<cell>` with
/// [`InstanceRoot::cell_dir`], which follows no symlink there, and compares the device and inode
/// of its `supervisor.lock` with those of the descriptor.
///
/// Refuses a lock held through another open of the file as `LockNotHeld`, and any other failure
/// to lock, flag or `fstat` the descriptor as `Lock`. A root that does not open is
/// `InstanceRoot`, a cell directory or lock file that does not open is `CellDirectory`, and a
/// different file is `NotThisCellsLock`. On a refusal the descriptor is closed. It creates
/// nothing in any case.
pub fn adopt_lock(
    lock: OwnedFd,
    config: &SupervisorConfig,
) -> Result<AdoptedLock, CellStartRefusal> {
    let lock = File::from(lock);
    lock_exclusive_nonblocking(&lock).map_err(|error| {
        if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
            CellStartRefusal::LockNotHeld
        } else {
            CellStartRefusal::Lock(error)
        }
    })?;
    set_close_on_exec(&lock).map_err(CellStartRefusal::Lock)?;
    let root = InstanceRoot::open(config.instance_root.as_path())
        .map_err(CellStartRefusal::InstanceRoot)?;
    let cell_dir = root
        .cell_dir(&config.cell)
        .map_err(CellStartRefusal::CellDirectory)?;
    let named = cell_dir
        .supervisor_lock_identity()
        .map_err(CellStartRefusal::CellDirectory)?;
    if named != identity_of(&lock).map_err(CellStartRefusal::Lock)? {
        return Err(CellStartRefusal::NotThisCellsLock);
    }
    Ok(AdoptedLock {
        _lock: lock,
        cell_dir,
    })
}

#[allow(unsafe_code)]
fn lock_exclusive_nonblocking(lock: &File) -> io::Result<()> {
    // SAFETY: flock takes integers only, and `lock` keeps its descriptor open for the call.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
fn set_close_on_exec(lock: &File) -> io::Result<()> {
    // SAFETY: fcntl with F_SETFD takes integers only, and `lock` keeps its descriptor open for
    // the call.
    if unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn identity_of(lock: &File) -> io::Result<FileIdentity> {
    let metadata = lock.metadata()?;
    Ok(FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}

impl fmt::Display for CellStartRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CellStartRefusal::Read(error) => {
                write!(f, "cannot read the config record on descriptor 3: {error}")
            }
            CellStartRefusal::Record(refusal) => write!(f, "{refusal}"),
            CellStartRefusal::CellMismatch { argv, record } => write!(
                f,
                "the config record is for {}, but argv names {}",
                record.as_str(),
                argv.as_str()
            ),
            CellStartRefusal::LockNotHeld => f.write_str(
                "descriptor 4 does not hold the supervisor lock: another open of the file holds it",
            ),
            CellStartRefusal::Lock(error) => {
                write!(f, "cannot lock or examine descriptor 4: {error}")
            }
            CellStartRefusal::InstanceRoot(error) => write!(f, "{error}"),
            CellStartRefusal::CellDirectory(error) => write!(f, "{error}"),
            CellStartRefusal::NotThisCellsLock => {
                f.write_str("descriptor 4 is not this cell's supervisor.lock")
            }
        }
    }
}

impl std::error::Error for CellStartRefusal {}
