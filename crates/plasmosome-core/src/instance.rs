use std::ffi::{CStr, CString};
use std::fmt;
use std::fs::{File, OpenOptions, Permissions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const LOCK_FILE: &str = "controller.lock";
const PRIVATE_FILE: libc::mode_t = 0o600;

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
            file.set_permissions(Permissions::from_mode(PRIVATE_FILE.into()))
                .map_err(io_error)?;
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
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags,
            libc::c_uint::from(mode),
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
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
    use std::io;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, symlink};
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
}
