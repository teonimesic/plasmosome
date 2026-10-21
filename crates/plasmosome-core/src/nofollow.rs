use std::ffi::{CStr, CString, OsStr};
use std::fmt;
use std::io;
#[cfg(target_os = "macos")]
use std::os::fd::AsFd;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The device and inode of a directory a walk passed or opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirIdentity {
    pub dev: u64,
    pub ino: u64,
}

/// The type of an object, as a stat that does not follow a symlink reports it. A FIFO, a device
/// and every other type are `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Directory,
    Regular,
    Socket,
    Symlink,
    Other,
}

/// What `fstat` on an open descriptor, or `fstatat` without following, reported about one object.
/// `mode` holds only the permission, set-id and sticky bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub kind: Kind,
    pub uid: u32,
    pub mode: u32,
    pub dev: u64,
    pub ino: u64,
}

impl Facts {
    fn of(stat: &libc::stat) -> Facts {
        let kind = match stat.st_mode & libc::S_IFMT {
            libc::S_IFDIR => Kind::Directory,
            libc::S_IFREG => Kind::Regular,
            libc::S_IFSOCK => Kind::Socket,
            libc::S_IFLNK => Kind::Symlink,
            _ => Kind::Other,
        };
        let (mode, dev) = mode_and_device(stat);
        Facts {
            kind,
            uid: stat.st_uid,
            mode: mode & 0o7777,
            dev,
            ino: stat.st_ino,
        }
    }

    fn identity(&self) -> DirIdentity {
        DirIdentity {
            dev: self.dev,
            ino: self.ino,
        }
    }
}

#[cfg(target_os = "macos")]
fn mode_and_device(stat: &libc::stat) -> (u32, u64) {
    (u32::from(stat.st_mode), stat.st_dev as u64)
}

#[cfg(target_os = "linux")]
fn mode_and_device(stat: &libc::stat) -> (u32, u64) {
    (stat.st_mode, stat.st_dev)
}

/// The object at the end of a walk, as the walk shows it to the caller's judge.
#[derive(Debug, Clone, Copy)]
pub struct Reached<'a> {
    /// The start path joined with every component.
    pub path: &'a Path,
    /// From the object's open descriptor, or from `fstatat` in its parent when it cannot be
    /// opened.
    pub facts: Facts,
    /// The directory holding the object, which has already passed the ancestor rule. `None` when
    /// the walk has no component, so the object is the start itself.
    pub parent: Option<Facts>,
}

/// What a walk opened, and the identity of every directory it passed on the way: the start
/// first, then each directory below it. The chain ends with the directory itself after
/// [`Walk::open_directory`], and with the directory holding the file after [`Walk::open_file`].
#[derive(Debug)]
pub struct Opened {
    pub fd: OwnedFd,
    pub facts: Facts,
    pub chain: Vec<DirIdentity>,
}

/// A path walked one component at a time from a start directory, never following a symlink.
///
/// Every directory the walk passes, the start included, must be owned by root or the effective UID
/// and writable by neither group nor other, with no sticky-bit exception. On macOS it must also
/// carry no ACL allow entry that lets anyone replace its entries, and sit on no volume mounted with
/// ownership ignored. The object the walk reaches is decided by the caller's judge, which sees it
/// even when it cannot be opened.
///
/// A walk can be opened again. Each open starts from the same start descriptor, so comparing the
/// identities of two opens shows whether the path still names the same object.
#[derive(Debug)]
pub struct Walk {
    start: OwnedFd,
    start_path: PathBuf,
    components: Vec<CString>,
}

#[derive(Clone, Copy)]
enum Last {
    Directory,
    File,
}

impl Walk {
    /// A walk of `path` from "/", which this opens now. `path` must be absolute and normal: no
    /// empty, `.` or `..` component and no trailing slash. "/" alone is a walk with no component.
    pub fn from_root(path: &Path) -> Result<Walk, WalkRefusal> {
        let components = absolute_components(path)?;
        let root = Path::new("/");
        let start = open_root().map_err(|error| io_failure("open", root, &error))?;
        Ok(Walk {
            start,
            start_path: root.to_path_buf(),
            components,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_dir(
        start: OwnedFd,
        start_path: &Path,
        relative: &Path,
    ) -> Result<Walk, WalkRefusal> {
        let components = components_of(relative.as_os_str().as_bytes(), relative)?;
        Ok(Walk {
            start,
            start_path: start_path.to_path_buf(),
            components,
        })
    }

    pub(crate) fn path(&self) -> PathBuf {
        let mut path = self.start_path.clone();
        for component in &self.components {
            path.push(OsStr::from_bytes(component.as_bytes()));
        }
        path
    }

    /// Opens the last component as a directory. On macOS a directory on a volume mounted with
    /// ownership ignored is refused; then `judge` decides it. A directory this user may not open
    /// is judged from `fstatat` in its parent, and is `Io` only when the judge passes it. Checks
    /// that need the descriptor, such as an ACL check, are the caller's, on the returned `fd`.
    pub fn open_directory<E>(
        &self,
        judge: impl Fn(&Reached<'_>) -> Result<(), E>,
    ) -> Result<Opened, E>
    where
        E: From<WalkRefusal>,
    {
        self.reach(Last::Directory, judge)
    }

    /// Opens the last component read-only with `O_NOFOLLOW`, `O_NONBLOCK` and `O_NOCTTY` and
    /// without `O_DIRECTORY`, so a FIFO or a device cannot block the open. `judge` decides the
    /// object from `fstat` on that descriptor, and must refuse every type the caller does not
    /// want. A regular file it accepts comes back blocking; any other type stays nonblocking. A
    /// symlink is `Symlink` and a missing name is `Missing`. An object that otherwise cannot be
    /// opened is judged from `fstatat` in its parent, and is `Io` only when the judge passes it.
    pub fn open_file<E>(&self, judge: impl Fn(&Reached<'_>) -> Result<(), E>) -> Result<Opened, E>
    where
        E: From<WalkRefusal>,
    {
        self.reach(Last::File, judge)
    }

    fn reach<E>(
        &self,
        last: Last,
        judge: impl Fn(&Reached<'_>) -> Result<(), E>,
    ) -> Result<Opened, E>
    where
        E: From<WalkRefusal>,
    {
        let euid = effective_uid();
        let mut at = self.start_path.clone();
        let mut facts = stat_fd(&self.start, &at)?;
        let Some((name, above)) = self.components.split_last() else {
            return self.reach_start(last, facts, judge);
        };
        let mut chain = Vec::with_capacity(self.components.len() + 1);
        let mut held: Option<OwnedFd> = None;
        for component in above {
            let parent = held.as_ref().unwrap_or(&self.start);
            pass(parent, &facts, euid, &at)?;
            chain.push(facts.identity());
            at.push(OsStr::from_bytes(component.as_bytes()));
            let next = open_child(parent, component, &at, SEARCH_ONLY, |facts| {
                judge_ancestor(facts, euid, &at).map_err(E::from)
            })?;
            facts = stat_fd(&next, &at)?;
            held = Some(next);
        }
        let parent = held.as_ref().unwrap_or(&self.start);
        pass(parent, &facts, euid, &at)?;
        chain.push(facts.identity());
        let holder = facts;
        at.push(OsStr::from_bytes(name.as_bytes()));
        let reached = |facts: Facts| Reached {
            path: &at,
            facts,
            parent: Some(holder),
        };
        match last {
            Last::Directory => {
                let dir = open_child(parent, name, &at, libc::O_RDONLY, |facts| {
                    judge(&reached(*facts))
                })?;
                let facts = stat_fd(&dir, &at)?;
                refuse_ignored_ownership(&dir, &at)?;
                judge(&reached(facts))?;
                chain.push(facts.identity());
                Ok(Opened {
                    fd: dir,
                    facts,
                    chain,
                })
            }
            Last::File => {
                let file = open_file_at(parent, name, &at, |facts| judge(&reached(*facts)))?;
                let facts = stat_fd(&file, &at)?;
                judge(&reached(facts))?;
                if facts.kind == Kind::Regular {
                    clear_nonblocking(&file, &at)?;
                }
                Ok(Opened {
                    fd: file,
                    facts,
                    chain,
                })
            }
        }
    }

    fn reach_start<E>(
        &self,
        last: Last,
        facts: Facts,
        judge: impl Fn(&Reached<'_>) -> Result<(), E>,
    ) -> Result<Opened, E>
    where
        E: From<WalkRefusal>,
    {
        let at = &self.start_path;
        let fd = self
            .start
            .try_clone()
            .map_err(|error| io_failure("dup", at, &error))?;
        let reached = Reached {
            path: at,
            facts,
            parent: None,
        };
        let chain = match last {
            Last::Directory => {
                refuse_ignored_ownership(&fd, at)?;
                judge(&reached)?;
                vec![facts.identity()]
            }
            Last::File => {
                judge(&reached)?;
                Vec::new()
            }
        };
        Ok(Opened { fd, facts, chain })
    }
}

fn pass(dir: &OwnedFd, facts: &Facts, euid: u32, at: &Path) -> Result<(), WalkRefusal> {
    refuse_ignored_ownership(dir, at)?;
    judge_ancestor(facts, euid, at)?;
    refuse_replacing_acl(dir, at)
}

/// Why a walk refused. Each variant names the path where the walk stopped: the component that
/// broke a rule, or the object it could not open. `Display` names the path and the rule; callers
/// branch on the variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkRefusal {
    NotNormal {
        path: PathBuf,
    },
    Symlink {
        path: PathBuf,
    },
    Missing {
        path: PathBuf,
    },
    NotDirectory {
        path: PathBuf,
    },
    ForeignOwner {
        path: PathBuf,
        uid: u32,
    },
    Replaceable {
        path: PathBuf,
        mode: u32,
    },
    ReplaceableByAcl {
        path: PathBuf,
    },
    OwnershipIgnored {
        path: PathBuf,
    },
    Io {
        op: &'static str,
        path: PathBuf,
        errno: i32,
    },
}

impl fmt::Display for WalkRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalkRefusal::NotNormal { path } => write!(
                f,
                "{} is not an absolute path of normal components (no empty, . or .. components)",
                path.display()
            ),
            WalkRefusal::Symlink { path } => write!(
                f,
                "{} is a symlink, which the walk does not follow",
                path.display()
            ),
            WalkRefusal::Missing { path } => write!(f, "{} does not exist", path.display()),
            WalkRefusal::NotDirectory { path } => {
                write!(f, "{} is not a directory", path.display())
            }
            WalkRefusal::ForeignOwner { path, uid } => write!(
                f,
                "{} is owned by uid {uid}, which is not trusted here",
                path.display()
            ),
            WalkRefusal::Replaceable { path, mode } => write!(
                f,
                "{} has mode {mode:04o}: group or other can replace entries in it",
                path.display()
            ),
            WalkRefusal::ReplaceableByAcl { path } => write!(
                f,
                "{} has an ACL allow entry granting add_file, add_subdirectory, delete_child, \
                 delete, writesecurity or chown; such an entry is refused on an ancestor whoever \
                 it names, even if it applies only to new children",
                path.display()
            ),
            WalkRefusal::OwnershipIgnored { path } => write!(
                f,
                "{} is on a volume mounted with ownership ignored (noowners): its owner and mode \
                 do not keep other users out",
                path.display()
            ),
            WalkRefusal::Io { op, path, errno } => {
                write!(f, "{op} failed at {} (errno {errno})", path.display())
            }
        }
    }
}

impl std::error::Error for WalkRefusal {}

/// The effective UID of this process.
#[allow(unsafe_code)]
pub fn effective_uid() -> u32 {
    // SAFETY: geteuid takes no arguments and cannot fail.
    unsafe { libc::geteuid() }
}

/// Whether the object open at `fd` carries an ACL allow entry granting write data, append data,
/// write attributes, write extended attributes, write security, chown, delete, or generic write
/// or all, whoever the entry names. On Linux it is always false: a POSIX ACL entry that grants
/// write shows in the group bits, which a mode check already reads. An ACL that cannot be read
/// is an error.
pub fn acl_grants_write(fd: BorrowedFd<'_>) -> io::Result<bool> {
    grants_write(fd)
}

#[cfg(target_os = "macos")]
fn grants_write(fd: BorrowedFd<'_>) -> io::Result<bool> {
    darwin_acl::grants_write(fd).map_err(|(_, errno)| io::Error::from_raw_os_error(errno))
}

#[cfg(target_os = "linux")]
fn grants_write(_fd: BorrowedFd<'_>) -> io::Result<bool> {
    Ok(false)
}

pub(crate) fn judge_ancestor(facts: &Facts, euid: u32, at: &Path) -> Result<(), WalkRefusal> {
    if facts.kind != Kind::Directory {
        return Err(WalkRefusal::NotDirectory {
            path: at.to_path_buf(),
        });
    }
    if facts.uid != 0 && facts.uid != euid {
        return Err(WalkRefusal::ForeignOwner {
            path: at.to_path_buf(),
            uid: facts.uid,
        });
    }
    if facts.mode & 0o022 != 0 {
        return Err(WalkRefusal::Replaceable {
            path: at.to_path_buf(),
            mode: facts.mode,
        });
    }
    Ok(())
}

pub(crate) fn absolute_components(path: &Path) -> Result<Vec<CString>, WalkRefusal> {
    let Some(relative) = path.as_os_str().as_bytes().strip_prefix(b"/") else {
        return Err(WalkRefusal::NotNormal {
            path: path.to_path_buf(),
        });
    };
    if relative.is_empty() {
        Ok(Vec::new())
    } else {
        components_of(relative, path)
    }
}

fn components_of(relative: &[u8], whole: &Path) -> Result<Vec<CString>, WalkRefusal> {
    relative
        .split(|byte| *byte == b'/')
        .map(|component| match component {
            b"" | b"." | b".." => None,
            component => CString::new(component).ok(),
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| WalkRefusal::NotNormal {
            path: whole.to_path_buf(),
        })
}

#[allow(unsafe_code)]
fn open_root() -> io::Result<OwnedFd> {
    // SAFETY: The path is a static NUL-terminated string, and open keeps no pointer to it.
    let raw = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: open returned a non-negative descriptor just above, which nothing else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    }
}

#[cfg(target_os = "macos")]
const SEARCH_ONLY: libc::c_int = libc::O_SEARCH;

#[cfg(target_os = "linux")]
const SEARCH_ONLY: libc::c_int = libc::O_PATH;

#[allow(unsafe_code)]
fn open_at(parent: &OwnedFd, name: &CStr, flags: libc::c_int) -> Result<OwnedFd, i32> {
    // SAFETY: `name` is a CStr that outlives the call, and `parent` keeps its descriptor open for
    // it. openat keeps no pointer to the name.
    let raw = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if raw >= 0 {
        // SAFETY: openat returned a non-negative descriptor just above, which nothing else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    } else {
        Err(last_errno())
    }
}

fn open_child<E>(
    parent: &OwnedFd,
    name: &CStr,
    at: &Path,
    access: libc::c_int,
    judge: impl FnOnce(&Facts) -> Result<(), E>,
) -> Result<OwnedFd, E>
where
    E: From<WalkRefusal>,
{
    let flags = access | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let errno = match open_at(parent, name, flags) {
        Ok(dir) => return Ok(dir),
        Err(errno) => errno,
    };
    let path = at.to_path_buf();
    Err(match errno {
        libc::ELOOP => WalkRefusal::Symlink { path }.into(),
        libc::ENOENT => WalkRefusal::Missing { path }.into(),
        libc::ENOTDIR => match stat_at(parent, name) {
            Ok(facts) if facts.kind == Kind::Symlink => WalkRefusal::Symlink { path }.into(),
            _ => WalkRefusal::NotDirectory { path }.into(),
        },
        libc::EACCES => judged_or_io(stat_at(parent, name), judge, path, errno),
        errno => io_at(path, errno).into(),
    })
}

fn open_file_at<E>(
    parent: &OwnedFd,
    name: &CStr,
    at: &Path,
    judge: impl FnOnce(&Facts) -> Result<(), E>,
) -> Result<OwnedFd, E>
where
    E: From<WalkRefusal>,
{
    let flags =
        libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let errno = match open_at(parent, name, flags) {
        Ok(file) => return Ok(file),
        Err(errno) => errno,
    };
    let path = at.to_path_buf();
    Err(match errno {
        libc::ELOOP => WalkRefusal::Symlink { path }.into(),
        libc::ENOENT => WalkRefusal::Missing { path }.into(),
        errno => judged_or_io(stat_at(parent, name), judge, path, errno),
    })
}

fn judged_or_io<E>(
    found: Result<Facts, i32>,
    judge: impl FnOnce(&Facts) -> Result<(), E>,
    path: PathBuf,
    errno: i32,
) -> E
where
    E: From<WalkRefusal>,
{
    match found.map(|facts| judge(&facts)) {
        Ok(Err(refusal)) => refusal,
        _ => io_at(path, errno).into(),
    }
}

fn io_at(path: PathBuf, errno: i32) -> WalkRefusal {
    WalkRefusal::Io {
        op: "openat",
        path,
        errno,
    }
}

#[allow(unsafe_code)]
fn clear_nonblocking(fd: &OwnedFd, at: &Path) -> Result<(), WalkRefusal> {
    // SAFETY: fcntl with F_GETFL takes no pointer, and `fd` keeps its descriptor open for the call.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io_failure("fcntl", at, &io::Error::last_os_error()));
    }
    // SAFETY: fcntl with F_SETFL takes an integer flag set, not a pointer, and `fd` keeps its
    // descriptor open for the call.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) } < 0 {
        return Err(io_failure("fcntl", at, &io::Error::last_os_error()));
    }
    Ok(())
}

#[allow(unsafe_code)]
fn stat_fd(fd: &OwnedFd, at: &Path) -> Result<Facts, WalkRefusal> {
    // SAFETY: libc::stat is a plain C struct, for which all zeroes is a valid value.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `fd` keeps its descriptor open, and `stat` is a local libc::stat that fstat may
    // write.
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } == 0 {
        Ok(Facts::of(&stat))
    } else {
        Err(io_failure("fstat", at, &io::Error::last_os_error()))
    }
}

#[allow(unsafe_code)]
pub(crate) fn stat_at(dir: &OwnedFd, name: &CStr) -> Result<Facts, i32> {
    // SAFETY: libc::stat is a plain C struct, for which all zeroes is a valid value.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `name` is a CStr that outlives the call, `dir` keeps its descriptor open, and `stat`
    // is a local libc::stat that fstatat may write.
    let outcome = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if outcome == 0 {
        Ok(Facts::of(&stat))
    } else {
        Err(last_errno())
    }
}

pub(crate) fn refuse_if<E>(
    found: Result<bool, (&'static str, i32)>,
    at: &Path,
    refusal: impl FnOnce(PathBuf) -> E,
) -> Result<(), E>
where
    E: From<WalkRefusal>,
{
    match found {
        Ok(false) => Ok(()),
        Ok(true) => Err(refusal(at.to_path_buf())),
        Err((op, errno)) => Err(WalkRefusal::Io {
            op,
            path: at.to_path_buf(),
            errno,
        }
        .into()),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn refuse_replacing_acl(dir: &OwnedFd, at: &Path) -> Result<(), WalkRefusal> {
    refuse_if(
        darwin_acl::allows_replacing_entries(dir.as_fd()),
        at,
        |path| WalkRefusal::ReplaceableByAcl { path },
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn refuse_replacing_acl(_dir: &OwnedFd, _at: &Path) -> Result<(), WalkRefusal> {
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn refuse_ignored_ownership(dir: &OwnedFd, at: &Path) -> Result<(), WalkRefusal> {
    // SAFETY: libc::statfs is a plain C struct, for which all zeroes is a valid value.
    let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `dir` keeps its descriptor open, and `stats` is a local libc::statfs that fstatfs may
    // write.
    let ignored = if unsafe { libc::fstatfs(dir.as_raw_fd(), &mut stats) } == 0 {
        Ok(stats.f_flags & libc::MNT_IGNORE_OWNERSHIP as u32 != 0)
    } else {
        Err(("fstatfs", last_errno()))
    };
    refuse_if(ignored, at, |path| WalkRefusal::OwnershipIgnored { path })
}

#[cfg(target_os = "linux")]
fn refuse_ignored_ownership(_dir: &OwnedFd, _at: &Path) -> Result<(), WalkRefusal> {
    Ok(())
}

pub(crate) fn io_failure(op: &'static str, at: &Path, error: &io::Error) -> WalkRefusal {
    WalkRefusal::Io {
        op,
        path: at.to_path_buf(),
        errno: errno_of(error),
    }
}

pub(crate) fn errno_of(error: &io::Error) -> i32 {
    error.raw_os_error().unwrap_or(libc::EIO)
}

pub(crate) fn last_errno() -> i32 {
    errno_of(&io::Error::last_os_error())
}

#[cfg(target_os = "macos")]
pub(crate) mod darwin_acl {
    use std::os::fd::{AsRawFd, BorrowedFd};
    use std::os::raw::{c_int, c_void};

    const ACL_TYPE_EXTENDED: c_int = 0x0000_0100;
    const ACL_FIRST_ENTRY: c_int = 0;
    const ACL_NEXT_ENTRY: c_int = -1;
    const ACL_EXTENDED_ALLOW: c_int = 1;
    const ACL_ADD_FILE: u64 = 1 << 2;
    const ACL_WRITE_DATA: u64 = 1 << 2;
    const ACL_DELETE: u64 = 1 << 4;
    const ACL_ADD_SUBDIRECTORY: u64 = 1 << 5;
    const ACL_APPEND_DATA: u64 = 1 << 5;
    const ACL_DELETE_CHILD: u64 = 1 << 6;
    const ACL_WRITE_ATTRIBUTES: u64 = 1 << 8;
    const ACL_WRITE_EXTATTRIBUTES: u64 = 1 << 10;
    const ACL_WRITE_SECURITY: u64 = 1 << 12;
    const ACL_CHANGE_OWNER: u64 = 1 << 13;
    const KAUTH_ACE_GENERIC_ALL: u64 = 1 << 21;
    const KAUTH_ACE_GENERIC_WRITE: u64 = 1 << 23;
    const REPLACES_ENTRIES: u64 = ACL_ADD_FILE
        | ACL_DELETE
        | ACL_ADD_SUBDIRECTORY
        | ACL_DELETE_CHILD
        | ACL_WRITE_SECURITY
        | ACL_CHANGE_OWNER
        | KAUTH_ACE_GENERIC_ALL
        | KAUTH_ACE_GENERIC_WRITE;
    const WRITES: u64 = ACL_WRITE_DATA
        | ACL_APPEND_DATA
        | ACL_WRITE_ATTRIBUTES
        | ACL_WRITE_EXTATTRIBUTES
        | ACL_WRITE_SECURITY
        | ACL_CHANGE_OWNER
        | ACL_DELETE
        | KAUTH_ACE_GENERIC_ALL
        | KAUTH_ACE_GENERIC_WRITE;

    #[allow(unsafe_code)]
    unsafe extern "C" {
        /// Reads only the descriptor. Returns a new ACL that the caller frees once with acl_free,
        /// or null with errno set.
        fn acl_get_fd_np(fd: c_int, kind: c_int) -> *mut c_void;
        /// Reads `acl`, a live ACL, and writes one entry pointer to `entry`, which points into
        /// `acl` and dies with it. Returns 0 when it found an entry.
        fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
        /// Reads `entry`, an entry of a live ACL, and writes one tag to `tag`. Returns 0 on
        /// success, or -1 with errno set.
        fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_int) -> c_int;
        /// Reads `entry`, an entry of a live ACL, and writes one mask to `mask`. Returns 0 on
        /// success, or -1 with errno set.
        fn acl_get_permset_mask_np(entry: *mut c_void, mask: *mut u64) -> c_int;
        /// Frees an object an acl function allocated. It must be called once per object, and the
        /// object is not used again.
        fn acl_free(object: *mut c_void) -> c_int;
    }

    struct Acl(*mut c_void);

    impl Acl {
        #[allow(unsafe_code)]
        fn of(fd: BorrowedFd<'_>) -> Result<Option<Acl>, (&'static str, i32)> {
            // SAFETY: acl_get_fd_np reads only the descriptor, which `fd` keeps open. A non-null
            // result is a new ACL that this Acl then owns.
            let acl = unsafe { acl_get_fd_np(fd.as_raw_fd(), ACL_TYPE_EXTENDED) };
            if !acl.is_null() {
                return Ok(Some(Acl(acl)));
            }
            let errno = super::last_errno();
            if errno == libc::ENOENT {
                Ok(None)
            } else {
                Err(("acl_get_fd_np", errno))
            }
        }

        #[allow(unsafe_code)]
        fn entries(&self) -> impl Iterator<Item = *mut c_void> + '_ {
            let mut which = ACL_FIRST_ENTRY;
            std::iter::from_fn(move || {
                let mut entry = std::ptr::null_mut();
                // SAFETY: `self.0` is a live ACL from acl_get_fd_np, freed only in Drop, and
                // `entry` is a local the call may write.
                let found = unsafe { acl_get_entry(self.0, which, &mut entry) } == 0;
                which = ACL_NEXT_ENTRY;
                found.then_some(entry)
            })
        }
    }

    impl Drop for Acl {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: `self.0` is the non-null ACL acl_get_fd_np returned, and Drop runs once, so
            // it is freed exactly once.
            unsafe { acl_free(self.0) };
        }
    }

    pub(crate) fn has_entries(fd: BorrowedFd<'_>) -> Result<bool, (&'static str, i32)> {
        Ok(Acl::of(fd)?.is_some_and(|acl| acl.entries().next().is_some()))
    }

    pub(super) fn allows_replacing_entries(
        fd: BorrowedFd<'_>,
    ) -> Result<bool, (&'static str, i32)> {
        allows_any(fd, REPLACES_ENTRIES)
    }

    pub(super) fn grants_write(fd: BorrowedFd<'_>) -> Result<bool, (&'static str, i32)> {
        allows_any(fd, WRITES)
    }

    #[allow(unsafe_code)]
    fn allows_any(fd: BorrowedFd<'_>, rights: u64) -> Result<bool, (&'static str, i32)> {
        let Some(acl) = Acl::of(fd)? else {
            return Ok(false);
        };
        for entry in acl.entries() {
            let mut tag = 0;
            // SAFETY: `entry` came from acl.entries() and points into `acl`, which lives through
            // the loop. `tag` is a local the call may write.
            if unsafe { acl_get_tag_type(entry, &mut tag) } != 0 {
                return Err(("acl_get_tag_type", super::last_errno()));
            }
            if tag != ACL_EXTENDED_ALLOW {
                continue;
            }
            let mut mask = 0;
            // SAFETY: `entry` came from acl.entries() and points into `acl`, which lives through
            // the loop. `mask` is a local the call may write.
            if unsafe { acl_get_permset_mask_np(entry, &mut mask) } != 0 {
                return Err(("acl_get_permset_mask_np", super::last_errno()));
            }
            if mask & rights != 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::fs;
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::{DirIdentity, Facts, Kind, Walk, effective_uid, judge_ancestor};

    pub(crate) struct TestRoot {
        path: PathBuf,
        _dir: TempDir,
    }

    impl TestRoot {
        pub(crate) fn new() -> TestRoot {
            let dir = tempfile::tempdir().expect("tempdir");
            let path = fs::canonicalize(dir.path()).expect("the temp dir resolves");
            set_mode(&path, 0o700);
            TestRoot { path, _dir: dir }
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }

        pub(crate) fn join(&self, relative: &str) -> PathBuf {
            self.path.join(relative)
        }

        pub(crate) fn start(&self) -> OwnedFd {
            OwnedFd::from(fs::File::open(&self.path).expect("the test root opens"))
        }

        pub(crate) fn walk(&self, relative: &str) -> Walk {
            Walk::from_dir(self.start(), &self.path, Path::new(relative))
                .expect("a normal relative path")
        }

        pub(crate) fn walkable_from_root() -> TestRoot {
            let euid = effective_uid();
            let mut refusals = Vec::new();
            for base in root_bases() {
                let dir = match tempfile::Builder::new().prefix("nf").tempdir_in(&base) {
                    Ok(dir) => dir,
                    Err(error) => {
                        refusals.push(format!("{}: {error}", base.display()));
                        continue;
                    }
                };
                let path = fs::canonicalize(dir.path()).expect("the test root resolves");
                set_mode(&path, 0o700);
                let walked = Walk::from_root(&path).and_then(|walk| {
                    walk.open_directory(|reached| {
                        judge_ancestor(&reached.facts, euid, reached.path)
                    })
                });
                match walked {
                    Ok(_) => return TestRoot { path, _dir: dir },
                    Err(refusal) => refusals.push(format!("{}: {refusal}", base.display())),
                }
            }
            panic!(
                "no base holds a test root the walk from / accepts; on macOS the base is the \
                 per-user temp directory that confstr(_CS_DARWIN_USER_TEMP_DIR) names, on Linux \
                 XDG_RUNTIME_DIR when it is set, then the test binary's directory:\n{}",
                refusals.join("\n")
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    pub(crate) fn root_bases() -> Vec<PathBuf> {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let mut buffer = vec![0u8; libc::PATH_MAX as usize];
        // SAFETY: `buffer` is a live Vec of buffer.len() bytes, and confstr writes at most that
        // many.
        let length = unsafe {
            libc::confstr(
                libc::_CS_DARWIN_USER_TEMP_DIR,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        assert!(
            length > 1 && length <= buffer.len(),
            "confstr(_CS_DARWIN_USER_TEMP_DIR) returned {length}"
        );
        buffer.truncate(length - 1);
        let base = PathBuf::from(OsStr::from_bytes(&buffer));
        vec![
            base.canonicalize()
                .unwrap_or_else(|error| panic!("{} does not resolve: {error}", base.display())),
        ]
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn root_bases() -> Vec<PathBuf> {
        let mut bases: Vec<PathBuf> = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|base| !base.is_empty())
            .map(PathBuf::from)
            .into_iter()
            .collect();
        let binary = std::env::current_exe().expect("the test binary knows its own path");
        bases.extend(binary.parent().map(Path::to_path_buf));
        bases
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            open_up(&self.path);
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("/bin/chmod")
                .args(["-R", "-N"])
                .arg(&self.path)
                .stderr(std::process::Stdio::null())
                .status();
        }
    }

    fn open_up(path: &Path) {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return;
        };
        if !metadata.is_dir() {
            return;
        }
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            open_up(&entry.path());
        }
    }

    pub(crate) fn facts(kind: Kind, uid: u32, mode: u32) -> Facts {
        Facts {
            kind,
            uid,
            mode,
            dev: 1,
            ino: 2,
        }
    }

    pub(crate) fn identity_of(path: &Path) -> DirIdentity {
        let metadata = fs::symlink_metadata(path).expect("stat");
        DirIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }

    pub(crate) fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
    }

    pub(crate) fn make_dir(path: &Path, mode: u32) {
        fs::create_dir(path).expect("mkdir");
        set_mode(path, mode);
    }

    pub(crate) fn write_file(path: &Path, bytes: &[u8], mode: u32) {
        fs::write(path, bytes).expect("write");
        set_mode(path, mode);
    }

    #[allow(unsafe_code)]
    pub(crate) fn make_fifo(path: &Path) {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no NUL");
        // SAFETY: `name` is a CString that outlives the call, and mkfifo keeps no pointer to it.
        let outcome = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
        assert_eq!(outcome, 0, "mkfifo: {}", std::io::Error::last_os_error());
    }

    pub(crate) fn within_a_second<T: Send + 'static>(
        fifo: &Path,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = sender.send(work());
        });
        let outcome = receiver.recv_timeout(Duration::from_secs(1)).ok();
        if outcome.is_some() {
            let _ = worker.join();
        } else {
            let _ = fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(fifo);
        }
        outcome
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn add_acl(path: &Path, rule: &str) {
        let status = std::process::Command::new("/bin/chmod")
            .arg("+a")
            .arg(rule)
            .arg(path)
            .status()
            .expect("chmod +a runs");
        assert!(status.success(), "chmod +a {rule:?} failed");
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn add_acl_at(path: &Path, index: usize, rule: &str) {
        let status = std::process::Command::new("/bin/chmod")
            .arg("+a#")
            .arg(index.to_string())
            .arg(rule)
            .arg(path)
            .status()
            .expect("chmod +a# runs");
        assert!(status.success(), "chmod +a# {index} {rule:?} failed");
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    unsafe extern "C" {
        /// Writes 16 bytes to `uuid`, which must point to at least that many writable bytes, and
        /// keeps no pointer. Returns 0 on success.
        fn mbr_gid_to_uuid(gid: libc::gid_t, uuid: *mut u8) -> libc::c_int;
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    pub(crate) fn add_raw_allow_for_everyone(path: &Path, rights: u32) {
        use std::os::unix::ffi::OsStrExt;
        const SYS_CHMOD_EXTENDED: libc::c_int = 282;
        const KAUTH_FILESEC_MAGIC: u32 = 0x012c_c16d;
        const KAUTH_ACE_PERMIT: u32 = 1;
        const KAUTH_ID_NONE: u32 = u32::MAX - 100;
        const EVERYONE_GID: libc::gid_t = 12;
        let mut everyone = [0u8; 16];
        assert_eq!(
            // SAFETY: `everyone` is a local 16-byte array, the size of the UUID mbr_gid_to_uuid
            // writes.
            unsafe { mbr_gid_to_uuid(EVERYONE_GID, everyone.as_mut_ptr()) },
            0,
            "the everyone group has a UUID"
        );
        let mut filesec = Vec::new();
        filesec.extend(KAUTH_FILESEC_MAGIC.to_ne_bytes());
        filesec.extend([0u8; 32]);
        filesec.extend(1u32.to_ne_bytes());
        filesec.extend(0u32.to_ne_bytes());
        filesec.extend(everyone);
        filesec.extend(KAUTH_ACE_PERMIT.to_ne_bytes());
        filesec.extend(rights.to_ne_bytes());
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no NUL");
        #[allow(deprecated)]
        // SAFETY: `c_path` is a CString and `filesec` holds one kauth_filesec with a single entry,
        // laid out field by field above. Both outlive the call, and the kernel only reads them.
        let outcome = unsafe {
            libc::syscall(
                SYS_CHMOD_EXTENDED,
                c_path.as_ptr(),
                KAUTH_ID_NONE,
                KAUTH_ID_NONE,
                -1 as libc::c_int,
                filesec.as_ptr(),
            )
        };
        assert_eq!(
            outcome,
            0,
            "chmod_extended stores an allow entry with rights {rights:#x}: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{
        TestRoot, facts, identity_of, make_dir, make_fifo, set_mode, within_a_second, write_file,
    };
    use super::*;
    use crate::private_socket::PrivateSocketError;
    use std::fs;
    use std::os::fd::AsFd;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    const EUID: u32 = 501;

    fn by_the_ancestor_rule(reached: &Reached<'_>) -> Result<(), WalkRefusal> {
        judge_ancestor(&reached.facts, effective_uid(), reached.path)
    }

    fn accept(_: &Reached<'_>) -> Result<(), WalkRefusal> {
        Ok(())
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Judged {
        Walk(WalkRefusal),
        Refused { path: PathBuf, kind: Kind },
    }

    impl From<WalkRefusal> for Judged {
        fn from(refusal: WalkRefusal) -> Self {
            Judged::Walk(refusal)
        }
    }

    fn refuse_all(reached: &Reached<'_>) -> Result<(), Judged> {
        Err(Judged::Refused {
            path: reached.path.to_path_buf(),
            kind: reached.facts.kind,
        })
    }

    #[allow(unsafe_code)]
    fn is_nonblocking(fd: &OwnedFd) -> bool {
        // SAFETY: fcntl with F_GETFL takes no pointer, and `fd` keeps its descriptor open.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0, "fcntl: {}", io::Error::last_os_error());
        flags & libc::O_NONBLOCK != 0
    }

    #[test]
    fn pass_holds_each_owner_to_the_effective_uid_not_to_itself() {
        let root = TestRoot::new();
        let start = root.start();
        let euid = effective_uid();
        let third = euid.wrapping_add(4242);
        assert_eq!(
            pass(
                &start,
                &facts(Kind::Directory, third, 0o755),
                euid,
                root.path()
            ),
            Err(WalkRefusal::ForeignOwner {
                path: root.path().to_path_buf(),
                uid: third
            }),
            "a 0755 directory a third user owns"
        );
        assert_eq!(
            pass(
                &start,
                &facts(Kind::Directory, euid, 0o755),
                euid,
                root.path()
            ),
            Ok(())
        );
        assert_eq!(
            pass(&start, &facts(Kind::Directory, 0, 0o755), euid, root.path()),
            Ok(())
        );
    }

    #[test]
    fn ancestor_judge_refuses_group_or_other_writable_and_sticky_directories() {
        let at = Path::new("/a");
        assert_eq!(
            judge_ancestor(&facts(Kind::Directory, 0, 0o755), EUID, at),
            Ok(())
        );
        assert_eq!(
            judge_ancestor(&facts(Kind::Directory, EUID, 0o700), EUID, at),
            Ok(())
        );
        for mode in [0o775, 0o757, 0o1777] {
            assert_eq!(
                judge_ancestor(&facts(Kind::Directory, 0, mode), EUID, at),
                Err(WalkRefusal::Replaceable {
                    path: at.to_path_buf(),
                    mode
                })
            );
        }
        assert_eq!(
            judge_ancestor(&facts(Kind::Directory, 4242, 0o755), EUID, at),
            Err(WalkRefusal::ForeignOwner {
                path: at.to_path_buf(),
                uid: 4242
            })
        );
        for kind in [Kind::Regular, Kind::Symlink, Kind::Socket, Kind::Other] {
            assert_eq!(
                judge_ancestor(&facts(kind, 0, 0o755), EUID, at),
                Err(WalkRefusal::NotDirectory {
                    path: at.to_path_buf()
                })
            );
        }
    }

    #[test]
    fn open_directory_from_the_root_returns_every_directory_identity_down_to_itself() {
        let walk = Walk::from_root(Path::new("/usr/bin")).expect("an absolute path");
        let opened = walk
            .open_directory(by_the_ancestor_rule)
            .expect("/usr/bin passes the ancestor rule");
        assert_eq!(
            opened.chain,
            vec![
                identity_of(Path::new("/")),
                identity_of(Path::new("/usr")),
                identity_of(Path::new("/usr/bin")),
            ]
        );
        assert_eq!(opened.facts.kind, Kind::Directory);
        let bin = identity_of(Path::new("/usr/bin"));
        assert_eq!((opened.facts.dev, opened.facts.ino), (bin.dev, bin.ino));
    }

    #[test]
    fn open_directory_of_the_root_alone_returns_the_root() {
        let walk = Walk::from_root(Path::new("/")).expect("an absolute path");
        assert_eq!(walk.path(), Path::new("/"));
        let opened = walk
            .open_directory(by_the_ancestor_rule)
            .expect("/ passes the ancestor rule");
        assert_eq!(opened.chain, vec![identity_of(Path::new("/"))]);
    }

    #[test]
    fn open_file_of_the_root_alone_shows_the_judge_a_directory_with_no_parent() {
        let walk = Walk::from_root(Path::new("/")).expect("an absolute path");
        let seen = std::cell::RefCell::new(None);
        let opened = walk
            .open_file(|reached: &Reached<'_>| -> Result<(), WalkRefusal> {
                *seen.borrow_mut() = Some((
                    reached.path.to_path_buf(),
                    reached.facts.kind,
                    reached.parent,
                ));
                Ok(())
            })
            .expect("the judge accepts it");
        assert_eq!(
            seen.into_inner(),
            Some((PathBuf::from("/"), Kind::Directory, None))
        );
        assert_eq!(opened.chain, Vec::new());
    }

    #[test]
    fn open_directory_returns_the_start_and_every_directory_down_to_itself() {
        let root = TestRoot::new();
        make_dir(&root.join("a"), 0o755);
        make_dir(&root.join("a/b"), 0o700);
        make_dir(&root.join("a/b/c"), 0o700);
        let walk = root.walk("a/b/c");
        assert_eq!(walk.path(), root.join("a/b/c"));
        let opened = walk.open_directory(by_the_ancestor_rule).expect("opens");
        assert_eq!(
            opened.chain,
            vec![
                identity_of(root.path()),
                identity_of(&root.join("a")),
                identity_of(&root.join("a/b")),
                identity_of(&root.join("a/b/c")),
            ]
        );
    }

    #[test]
    fn open_directory_returns_the_judge_refusal_of_the_last_directory() {
        let root = TestRoot::new();
        make_dir(&root.join("a"), 0o700);
        assert_eq!(
            root.walk("a")
                .open_directory(refuse_all)
                .map(|opened| opened.chain),
            Err(Judged::Refused {
                path: root.join("a"),
                kind: Kind::Directory
            })
        );
    }

    #[test]
    fn open_file_returns_the_file_and_the_chain_down_to_its_parent() {
        let root = TestRoot::new();
        make_dir(&root.join("a"), 0o700);
        write_file(&root.join("a/file"), b"bytes", 0o600);
        let opened = root.walk("a/file").open_file(accept).expect("opens");
        assert_eq!(
            opened.chain,
            vec![identity_of(root.path()), identity_of(&root.join("a"))]
        );
        let file = identity_of(&root.join("a/file"));
        assert_eq!(opened.facts.kind, Kind::Regular);
        assert_eq!((opened.facts.dev, opened.facts.ino), (file.dev, file.ino));
        assert_eq!(opened.facts.uid, effective_uid());
        assert_eq!(opened.facts.mode, 0o600);
        let mut text = String::new();
        io::Read::read_to_string(&mut fs::File::from(opened.fd), &mut text).expect("read");
        assert_eq!(text, "bytes");
    }

    #[test]
    fn open_file_shows_the_judge_the_file_and_the_directory_holding_it() {
        let root = TestRoot::new();
        make_dir(&root.join("a"), 0o750);
        write_file(&root.join("a/file"), b"", 0o640);
        let seen = std::cell::RefCell::new(None);
        root.walk("a/file")
            .open_file(|reached: &Reached<'_>| -> Result<(), WalkRefusal> {
                *seen.borrow_mut() = Some((
                    reached.path.to_path_buf(),
                    reached.facts.mode,
                    reached
                        .parent
                        .map(|parent| (parent.kind, parent.mode, parent.ino)),
                ));
                Ok(())
            })
            .expect("opens");
        let parent = identity_of(&root.join("a"));
        assert_eq!(
            seen.into_inner(),
            Some((
                root.join("a/file"),
                0o640,
                Some((Kind::Directory, 0o750, parent.ino))
            ))
        );
    }

    #[test]
    fn open_file_returns_the_judge_refusal() {
        let root = TestRoot::new();
        write_file(&root.join("file"), b"", 0o600);
        assert_eq!(
            root.walk("file")
                .open_file(refuse_all)
                .map(|opened| opened.chain),
            Err(Judged::Refused {
                path: root.join("file"),
                kind: Kind::Regular
            })
        );
    }

    #[test]
    fn open_file_refuses_a_final_symlink_to_a_valid_file() {
        let root = TestRoot::new();
        write_file(&root.join("file"), b"", 0o600);
        symlink("file", root.join("link")).expect("symlink");
        assert_eq!(
            root.walk("link")
                .open_file(accept)
                .map(|opened| opened.facts),
            Err(WalkRefusal::Symlink {
                path: root.join("link")
            })
        );
    }

    #[test]
    fn open_file_refuses_a_symlinked_directory_above_a_valid_file() {
        let root = TestRoot::new();
        make_dir(&root.join("real"), 0o700);
        write_file(&root.join("real/file"), b"", 0o600);
        symlink("real", root.join("link")).expect("symlink");
        assert_eq!(
            root.walk("link/file")
                .open_file(accept)
                .map(|opened| opened.facts),
            Err(WalkRefusal::Symlink {
                path: root.join("link")
            })
        );
    }

    #[test]
    fn open_file_refuses_a_missing_file() {
        let root = TestRoot::new();
        assert_eq!(
            root.walk("absent")
                .open_file(accept)
                .map(|opened| opened.facts),
            Err(WalkRefusal::Missing {
                path: root.join("absent")
            })
        );
    }

    #[test]
    fn open_file_judges_the_directory_holding_it_by_the_ancestor_rule() {
        let root = TestRoot::new();
        make_dir(&root.join("a"), 0o700);
        write_file(&root.join("a/file"), b"", 0o600);
        set_mode(&root.join("a"), 0o770);
        assert_eq!(
            root.walk("a/file")
                .open_file(refuse_all)
                .map(|opened| opened.facts),
            Err(Judged::Walk(WalkRefusal::Replaceable {
                path: root.join("a"),
                mode: 0o770
            })),
            "the directory holding the file is judged before the file"
        );
    }

    #[test]
    fn open_directory_judges_the_directory_holding_it_by_the_ancestor_rule() {
        let root = TestRoot::new();
        make_dir(&root.join("a"), 0o700);
        make_dir(&root.join("a/dir"), 0o700);
        set_mode(&root.join("a"), 0o770);
        assert_eq!(
            root.walk("a/dir")
                .open_directory(refuse_all)
                .map(|opened| opened.facts),
            Err(Judged::Walk(WalkRefusal::Replaceable {
                path: root.join("a"),
                mode: 0o770
            })),
            "the directory holding the directory is judged before it"
        );
    }

    #[test]
    fn open_file_does_not_block_on_a_fifo_and_leaves_it_nonblocking() {
        let root = TestRoot::new();
        let fifo = root.join("fifo");
        make_fifo(&fifo);
        let walk = root.walk("fifo");
        let refused = within_a_second(&fifo, move || {
            walk.open_file(refuse_all).map(|opened| opened.facts)
        });
        assert_eq!(
            refused,
            Some(Err(Judged::Refused {
                path: fifo.clone(),
                kind: Kind::Other
            })),
            "the open of a FIFO with no writer returns within a second"
        );
        let walk = root.walk("fifo");
        let accepted = within_a_second(&fifo, move || {
            walk.open_file(accept)
                .map(|opened| is_nonblocking(&opened.fd))
        });
        assert_eq!(accepted, Some(Ok(true)));
    }

    #[test]
    fn open_file_clears_nonblocking_on_a_regular_file() {
        let root = TestRoot::new();
        write_file(&root.join("file"), b"", 0o600);
        let opened = root.walk("file").open_file(accept).expect("opens");
        assert!(!is_nonblocking(&opened.fd));
    }

    #[test]
    fn open_file_judges_a_socket_it_cannot_open() {
        let root = TestRoot::new();
        let _socket = UnixListener::bind(root.join("sock")).expect("bind");
        assert_eq!(
            root.walk("sock")
                .open_file(refuse_all)
                .map(|opened| opened.facts),
            Err(Judged::Refused {
                path: root.join("sock"),
                kind: Kind::Socket
            })
        );
        assert!(matches!(
            root.walk("sock").open_file(accept),
            Err(WalkRefusal::Io { op: "openat", .. })
        ));
    }

    #[test]
    fn open_file_judges_a_file_it_cannot_read() {
        if effective_uid() == 0 {
            println!("skipped: root reads every file");
            return;
        }
        let root = TestRoot::new();
        write_file(&root.join("file"), b"", 0o200);
        assert_eq!(
            root.walk("file")
                .open_file(refuse_all)
                .map(|opened| opened.facts),
            Err(Judged::Refused {
                path: root.join("file"),
                kind: Kind::Regular
            })
        );
        let seen = std::cell::RefCell::new(None);
        let outcome = root.walk("file").open_file(|reached: &Reached<'_>| {
            *seen.borrow_mut() =
                Some((reached.facts.kind, reached.parent.map(|parent| parent.ino)));
            Ok::<(), WalkRefusal>(())
        });
        assert_eq!(
            outcome.map(|opened| opened.facts),
            Err(WalkRefusal::Io {
                op: "openat",
                path: root.join("file"),
                errno: libc::EACCES
            })
        );
        assert_eq!(
            seen.into_inner(),
            Some((Kind::Regular, Some(identity_of(root.path()).ino))),
            "the judge of a file it cannot open sees the directory holding it"
        );
    }

    #[test]
    fn a_walk_judges_its_start_by_the_ancestor_rule() {
        let root = TestRoot::new();
        make_dir(&root.join("dir"), 0o700);
        write_file(&root.join("file"), b"", 0o600);
        write_file(&root.join("dir/file"), b"", 0o600);
        for mode in [0o770, 0o707] {
            set_mode(root.path(), mode);
            let replaceable = Err(WalkRefusal::Replaceable {
                path: root.path().to_path_buf(),
                mode,
            });
            assert_eq!(
                root.walk("file")
                    .open_file(accept)
                    .map(|opened| opened.facts),
                replaceable,
                "{mode:04o}"
            );
            assert_eq!(
                root.walk("dir/file")
                    .open_file(accept)
                    .map(|opened| opened.facts),
                replaceable,
                "{mode:04o}"
            );
            assert_eq!(
                root.walk("dir")
                    .open_directory(accept)
                    .map(|opened| opened.facts),
                replaceable,
                "{mode:04o}"
            );
        }
        set_mode(root.path(), 0o700);
        assert!(root.walk("dir/file").open_file(accept).is_ok());
    }

    #[test]
    fn a_walk_opens_again_from_its_start_and_sees_a_replaced_file() {
        let root = TestRoot::new();
        write_file(&root.join("file"), b"first", 0o600);
        let walk = root.walk("file");
        let first = walk.open_file(accept).expect("opens");
        fs::rename(root.join("file"), root.join("moved")).expect("rename");
        write_file(&root.join("file"), b"second", 0o600);
        let second = walk.open_file(accept).expect("opens again");
        assert_ne!(first.facts.ino, second.facts.ino);
        assert_eq!(second.facts.ino, identity_of(&root.join("file")).ino);
    }

    #[test]
    fn walks_take_only_absolute_normal_paths() {
        for path in [
            "relative/file",
            "/a/../file",
            "/a/./file",
            "/a//file",
            "/a/file/",
        ] {
            assert_eq!(
                Walk::from_root(Path::new(path)).map(|walk| walk.path()),
                Err(WalkRefusal::NotNormal {
                    path: PathBuf::from(path)
                })
            );
        }
        let root = TestRoot::new();
        for relative in ["", "a/../file", "/file", "a/"] {
            assert_eq!(
                Walk::from_dir(root.start(), root.path(), Path::new(relative))
                    .map(|walk| walk.path()),
                Err(WalkRefusal::NotNormal {
                    path: PathBuf::from(relative)
                }),
                "{relative:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn acl_grants_write_finds_an_allow_entry_that_lets_anyone_change_the_file() {
        use super::fixtures::{add_acl, add_acl_at, add_raw_allow_for_everyone};
        let root = TestRoot::new();
        let granted = |name: &str| {
            let path = root.join(name);
            let file = fs::File::open(&path).expect("open");
            acl_grants_write(file.as_fd()).expect("the ACL reads")
        };
        write_file(&root.join("plain"), b"", 0o600);
        assert!(!granted("plain"));
        for (index, rule) in [
            "everyone allow write",
            "everyone allow append",
            "everyone allow writeattr",
            "everyone allow writeextattr",
            "everyone allow writesecurity",
            "everyone allow chown",
            "everyone allow delete",
        ]
        .iter()
        .enumerate()
        {
            let name = format!("granted{index}");
            write_file(&root.join(&name), b"", 0o600);
            add_acl(&root.join(&name), rule);
            assert!(granted(&name), "{rule:?}");
        }
        write_file(&root.join("second"), b"", 0o600);
        add_acl(&root.join("second"), "staff allow read");
        add_acl_at(&root.join("second"), 1, "everyone allow write");
        assert!(
            granted("second"),
            "an allow-write entry after another allow entry"
        );
        for (index, rule) in [
            "everyone allow read,readattr,readextattr,readsecurity,execute",
            "everyone deny write,append,writeattr,writeextattr,writesecurity,chown,delete",
        ]
        .iter()
        .enumerate()
        {
            let name = format!("kept{index}");
            write_file(&root.join(&name), b"", 0o600);
            add_acl(&root.join(&name), rule);
            assert!(!granted(&name), "{rule:?}");
        }
        for (name, rights) in [("generic_write", 1u32 << 23), ("generic_all", 1u32 << 21)] {
            write_file(&root.join(name), b"", 0o600);
            add_raw_allow_for_everyone(&root.join(name), rights);
            assert!(granted(name), "{name}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_walk_finds_a_replacing_acl_entry_after_another_allow_entry() {
        use super::fixtures::{add_acl, add_acl_at};
        let root = TestRoot::new();
        make_dir(&root.join("held"), 0o700);
        write_file(&root.join("held/file"), b"", 0o600);
        add_acl(&root.join("held"), "staff allow list");
        assert!(root.walk("held/file").open_file(accept).is_ok());
        add_acl_at(&root.join("held"), 1, "everyone allow add_file");
        assert_eq!(
            root.walk("held/file")
                .open_file(accept)
                .map(|opened| opened.facts),
            Err(WalkRefusal::ReplaceableByAcl {
                path: root.join("held")
            })
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn acl_grants_write_leaves_posix_acls_to_the_group_bits_on_linux() {
        let root = TestRoot::new();
        write_file(&root.join("file"), b"", 0o600);
        let file = fs::File::open(root.join("file")).expect("open");
        assert_eq!(acl_grants_write(file.as_fd()).ok(), Some(false));
    }

    #[test]
    fn walk_refusals_name_the_path_and_the_rule() {
        let at = || PathBuf::from("/r/file");
        let cases = [
            (
                WalkRefusal::NotNormal { path: at() },
                "/r/file is not an absolute path of normal components (no empty, . or .. components)",
            ),
            (
                WalkRefusal::Symlink { path: at() },
                "/r/file is a symlink, which the walk does not follow",
            ),
            (
                WalkRefusal::Missing { path: at() },
                "/r/file does not exist",
            ),
            (
                WalkRefusal::NotDirectory { path: at() },
                "/r/file is not a directory",
            ),
            (
                WalkRefusal::ForeignOwner {
                    path: at(),
                    uid: 42,
                },
                "/r/file is owned by uid 42, which is not trusted here",
            ),
            (
                WalkRefusal::Replaceable {
                    path: at(),
                    mode: 0o1777,
                },
                "/r/file has mode 1777: group or other can replace entries in it",
            ),
            (
                WalkRefusal::ReplaceableByAcl { path: at() },
                "/r/file has an ACL allow entry granting add_file, add_subdirectory, delete_child, \
                 delete, writesecurity or chown; such an entry is refused on an ancestor whoever it \
                 names, even if it applies only to new children",
            ),
            (
                WalkRefusal::OwnershipIgnored { path: at() },
                "/r/file is on a volume mounted with ownership ignored (noowners): its owner and \
                 mode do not keep other users out",
            ),
            (
                WalkRefusal::Io {
                    op: "openat",
                    path: at(),
                    errno: 13,
                },
                "openat failed at /r/file (errno 13)",
            ),
        ];
        for (refusal, text) in cases {
            assert_eq!(refusal.to_string(), text);
        }
    }

    #[test]
    fn a_private_socket_error_keeps_the_walk_refusal_it_came_from() {
        let at = || PathBuf::from("/r/cell");
        let cases = [
            (
                WalkRefusal::NotNormal { path: at() },
                PrivateSocketError::BadPath { path: at() },
            ),
            (
                WalkRefusal::Symlink { path: at() },
                PrivateSocketError::SymlinkInPath { path: at() },
            ),
            (
                WalkRefusal::Missing { path: at() },
                PrivateSocketError::NoDirectory { path: at() },
            ),
            (
                WalkRefusal::NotDirectory { path: at() },
                PrivateSocketError::NotDirectory { path: at() },
            ),
            (
                WalkRefusal::ForeignOwner { path: at(), uid: 7 },
                PrivateSocketError::ForeignOwner { path: at(), uid: 7 },
            ),
            (
                WalkRefusal::Replaceable {
                    path: at(),
                    mode: 0o775,
                },
                PrivateSocketError::Replaceable {
                    path: at(),
                    mode: 0o775,
                },
            ),
            (
                WalkRefusal::ReplaceableByAcl { path: at() },
                PrivateSocketError::ReplaceableByAcl { path: at() },
            ),
            (
                WalkRefusal::OwnershipIgnored { path: at() },
                PrivateSocketError::OwnershipIgnored { path: at() },
            ),
            (
                WalkRefusal::Io {
                    op: "fstat",
                    path: at(),
                    errno: 5,
                },
                PrivateSocketError::Io {
                    op: "fstat",
                    path: at(),
                    errno: 5,
                },
            ),
        ];
        for (refusal, expected) in cases {
            assert_eq!(PrivateSocketError::from(refusal), expected);
        }
    }
}
