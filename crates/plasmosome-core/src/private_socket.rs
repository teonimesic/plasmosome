use std::ffi::{CStr, CString, OsStr};
use std::fmt;
use std::io;
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The device and inode of the directory a [`PrivateDir`] holds open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirIdentity {
    pub dev: u64,
    pub ino: u64,
}

/// A socket's parent directory, opened without following symlinks and judged private: every
/// ancestor is owned by root or the effective UID and writable by neither group nor other, and
/// on macOS carries no ACL allow entry granting add_file, add_subdirectory, delete_child,
/// delete, writesecurity or chown, whoever it names. The directory itself is owned by the
/// effective UID, has no group or other bits and carries no ACL. The directory stays open, so
/// a later rename of its path cannot redirect operations made through it.
#[derive(Debug)]
pub struct PrivateDir {
    path: PathBuf,
    dir: OwnedFd,
    identity: DirIdentity,
    walk: Walk,
}

#[derive(Debug)]
struct Walk {
    start: OwnedFd,
    start_path: PathBuf,
    components: Vec<CString>,
}

impl Walk {
    fn path(&self) -> PathBuf {
        let mut path = self.start_path.clone();
        for component in &self.components {
            path.push(OsStr::from_bytes(component.as_bytes()));
        }
        path
    }

    fn run(&self, euid: u32) -> Result<(OwnedFd, Facts), PrivateSocketError> {
        let mut at = self.start_path.clone();
        let mut facts = stat_fd(&self.start, &at)?;
        let mut held: Option<OwnedFd> = None;
        for (index, component) in self.components.iter().enumerate() {
            let parent = held.as_ref().unwrap_or(&self.start);
            judge_ancestor(&facts, euid, &at)?;
            refuse_replacing_acl(parent, &at)?;
            at.push(OsStr::from_bytes(component.as_bytes()));
            let access = if index + 1 == self.components.len() {
                libc::O_RDONLY
            } else {
                SEARCH_ONLY
            };
            let next = open_child(parent, component, &at, access)?;
            facts = stat_fd(&next, &at)?;
            held = Some(next);
        }
        let dir = match held {
            Some(dir) => dir,
            None => self
                .start
                .try_clone()
                .map_err(|error| io_failure("dup", &at, &error))?,
        };
        Ok((dir, facts))
    }

    fn judged(&self, path: &Path) -> Result<(OwnedFd, Facts), PrivateSocketError> {
        let euid = effective_uid();
        let (dir, facts) = self.run(euid)?;
        judge_private(&facts, euid, path)?;
        refuse_acl(&dir, path)?;
        Ok((dir, facts))
    }
}

impl PrivateDir {
    /// Walks `path` from "/" without following symlinks, judges every ancestor and the final
    /// directory, and keeps the final directory open. `path` must be absolute and normal: no
    /// `.` or `..` components and no empty components. Returns the first rule broken, naming
    /// the component that broke it.
    pub fn open(path: &Path) -> Result<PrivateDir, PrivateSocketError> {
        let components = absolute_components(path)?;
        let root = Path::new("/");
        let start = open_root().map_err(|error| io_failure("open", root, &error))?;
        Self::from_walk(Walk {
            start,
            start_path: root.to_path_buf(),
            components,
        })
    }

    #[cfg(test)]
    pub(crate) fn open_from(
        start: OwnedFd,
        start_path: &Path,
        relative: &Path,
    ) -> Result<PrivateDir, PrivateSocketError> {
        let components = components_of(relative.as_os_str().as_bytes(), relative)?;
        Self::from_walk(Walk {
            start,
            start_path: start_path.to_path_buf(),
            components,
        })
    }

    fn from_walk(walk: Walk) -> Result<PrivateDir, PrivateSocketError> {
        let path = walk.path();
        let (dir, facts) = walk.judged(&path)?;
        Ok(PrivateDir {
            path,
            dir,
            identity: DirIdentity {
                dev: facts.dev,
                ino: facts.ino,
            },
            walk,
        })
    }

    /// The path this directory was opened at.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The device and inode of the directory held open.
    pub fn identity(&self) -> DirIdentity {
        self.identity
    }

    /// Walks the path again and requires the same judgments and the same directory identity.
    /// A directory renamed away and replaced at the path is `Replaced`.
    pub fn reconfirm(&self) -> Result<(), PrivateSocketError> {
        let (_, facts) = self.walk.judged(&self.path)?;
        if facts.dev == self.identity.dev && facts.ino == self.identity.ino {
            Ok(())
        } else {
            Err(PrivateSocketError::Replaced {
                path: self.path.clone(),
            })
        }
    }

    /// Inspects `name` inside this directory without following it, and requires a socket owned
    /// by `trusted_uid` with permission bits exactly 0o600. A symlink at `name` is refused as
    /// `NotASocket`, never followed.
    pub fn socket_entry(
        &self,
        name: &str,
        trusted_uid: u32,
    ) -> Result<SocketEntry, PrivateSocketError> {
        let c_name = entry_name(name)?;
        let path = self.path.join(name);
        let facts = stat_at(&self.dir, &c_name).map_err(|errno| no_socket_or_io(errno, &path))?;
        judge_socket(&facts, trusted_uid, &path)?;
        Ok(SocketEntry {
            path,
            dev: facts.dev,
            ino: facts.ino,
        })
    }
}

/// A socket entry checked inside a [`PrivateDir`]: its full path and its device and inode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketEntry {
    pub path: PathBuf,
    pub dev: u64,
    pub ino: u64,
}

/// A listening socket inside a [`PrivateDir`] that hands out only connections whose kernel peer
/// UID is trusted. Dropping it closes the socket and then removes the entry it created, only
/// while the entry at that name is still the same socket by device and inode.
#[derive(Debug)]
pub struct PrivateListener {
    listener: UnixListener,
    bound: BoundEntry,
    trusted_uid: u32,
}

#[derive(Debug)]
struct BoundEntry {
    dir: PrivateDir,
    name: CString,
    entry: SocketEntry,
}

impl Drop for BoundEntry {
    fn drop(&mut self) {
        if let Ok(facts) = stat_at(&self.dir.dir, &self.name)
            && facts.kind == Kind::Socket
            && facts.dev == self.entry.dev
            && facts.ino == self.entry.ino
        {
            unsafe { libc::unlinkat(self.dir.dir.as_raw_fd(), self.name.as_ptr(), 0) };
        }
    }
}

impl PrivateListener {
    /// Binds `name` inside `dir` for peers whose effective UID is `trusted_uid`. Production
    /// passes `effective_uid()`; a test passes another value to force a real mismatch. `name` is
    /// one path component. Any existing entry at the name is refused and left untouched. `dir`
    /// is walked and judged again before the socket is created, and again after it is bound; if
    /// that second check fails, its error is returned and the socket at the name in the held
    /// directory is removed, but only if the effective UID owns it. `BindEscaped` means no
    /// socket owned by the effective UID is at the name in the held directory after bind. The
    /// socket is owned by the effective UID and set to mode 0600 before it listens; umask plays
    /// no part. The listener is nonblocking. A caller that forks from another thread must hold
    /// its descriptor lock around this call, because on macOS the socket is created before it
    /// is marked close-on-exec.
    pub fn bind(
        dir: PrivateDir,
        name: &str,
        trusted_uid: u32,
    ) -> Result<PrivateListener, PrivateSocketError> {
        let (socket, bound) = prepare(dir, name)?;
        if unsafe { libc::listen(socket.as_raw_fd(), 16) } != 0 {
            return Err(io_failure(
                "listen",
                &bound.entry.path,
                &io::Error::last_os_error(),
            ));
        }
        let listener = UnixListener::from(socket);
        listener
            .set_nonblocking(true)
            .map_err(|error| io_failure("fcntl", &bound.entry.path, &error))?;
        Ok(PrivateListener {
            listener,
            bound,
            trusted_uid,
        })
    }

    /// The socket entry this listener created.
    pub fn entry(&self) -> &SocketEntry {
        &self.bound.entry
    }

    /// The listening descriptor, for polling. The caller must not close it or accept on it.
    pub fn as_raw_fd(&self) -> RawFd {
        self.listener.as_raw_fd()
    }

    /// Accepts one pending connection without blocking. `Ok(None)`: nothing pending. A peer
    /// whose effective UID is not trusted, or whose credentials cannot be read, is shut down and
    /// closed before any byte is read and reported as `Accepted::Refused`. A trusted stream is
    /// returned in blocking mode; the caller sets its timeouts. A caller that forks from another
    /// thread must hold its descriptor lock around this call, because on macOS the accepted
    /// descriptor is marked close-on-exec only after it exists.
    pub fn accept(&self) -> io::Result<Option<Accepted>> {
        let stream = match self.listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        };
        if let Err(refusal) = check_peer_uid(&stream, self.trusted_uid) {
            let _ = stream.shutdown(Shutdown::Both);
            return Ok(Some(Accepted::Refused(refusal)));
        }
        stream.set_nonblocking(false)?;
        Ok(Some(Accepted::Trusted(stream)))
    }
}

/// One accepted connection: a stream from a trusted peer, or the reason the peer was refused.
#[derive(Debug)]
pub enum Accepted {
    Trusted(UnixStream),
    Refused(PrivateSocketError),
}

/// Reads the connected peer's effective UID from the kernel: `getpeereid` on macOS,
/// `SO_PEERCRED` on Linux. Both report the credentials captured when the connection was made.
/// A socket with no peer credentials fails with `ENOTCONN`.
pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    kernel_peer_uid(stream.as_raw_fd())
}

#[cfg(target_os = "macos")]
fn kernel_peer_uid(fd: RawFd) -> io::Result<u32> {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } == 0 {
        Ok(uid)
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn kernel_peer_uid(fd: RawFd) -> io::Result<u32> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let outcome = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if outcome != 0 {
        return Err(io::Error::last_os_error());
    }
    if credentials.uid == libc::uid_t::MAX {
        return Err(io::Error::from_raw_os_error(libc::ENOTCONN));
    }
    Ok(credentials.uid)
}

/// Requires the kernel peer UID of `stream` to equal `expected`. Call it before sending or
/// reading any byte; on refusal, close the stream unread.
pub fn check_peer_uid(stream: &UnixStream, expected: u32) -> Result<(), PrivateSocketError> {
    let found = peer_uid(stream).map_err(|error| PrivateSocketError::PeerCredentials {
        errno: errno_of(&error),
    })?;
    judge_peer(expected, found)
}

/// The client half of the path boundary, for a caller-owned nonblocking connect: opens the
/// parent of `socket_path` as a [`PrivateDir`], then checks its file name with
/// [`PrivateDir::socket_entry`]. `socket_path` must be absolute and normal, as for
/// [`PrivateDir::open`], and name an entry below `/`. Returns the checked entry. The caller
/// connects afterwards and must still call [`check_peer_uid`] on the connected stream.
pub fn check_private_path(
    socket_path: &Path,
    trusted_uid: u32,
) -> Result<SocketEntry, PrivateSocketError> {
    let mut components = absolute_components(socket_path)?;
    let (Some(name), Some(parent)) = (components.pop(), socket_path.parent()) else {
        return Err(PrivateSocketError::BadPath {
            path: socket_path.to_path_buf(),
        });
    };
    let name = name
        .into_string()
        .map_err(|error| PrivateSocketError::BadName {
            name: error.into_cstring().to_string_lossy().into_owned(),
        })?;
    PrivateDir::open(parent)?.socket_entry(&name, trusted_uid)
}

/// The effective UID of this process.
pub fn effective_uid() -> u32 {
    unsafe { libc::geteuid() }
}

/// Why a private socket path, socket or peer was refused. `Display` names the path and the
/// rule broken; callers branch on the variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivateSocketError {
    BadPath {
        path: PathBuf,
    },
    SymlinkInPath {
        path: PathBuf,
    },
    NoDirectory {
        path: PathBuf,
    },
    NoSocket {
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
    NotPrivate {
        path: PathBuf,
        mode: u32,
    },
    AclPresent {
        path: PathBuf,
    },
    Replaced {
        path: PathBuf,
    },
    BadName {
        name: String,
    },
    PathTooLong {
        path: PathBuf,
        max: usize,
    },
    AddressInUse {
        path: PathBuf,
    },
    NotASocket {
        path: PathBuf,
    },
    SocketOwner {
        path: PathBuf,
        uid: u32,
    },
    SocketMode {
        path: PathBuf,
        mode: u32,
    },
    BindEscaped {
        path: PathBuf,
    },
    PeerCredentials {
        errno: i32,
    },
    PeerMismatch {
        trusted: u32,
        found: u32,
    },
    Io {
        op: &'static str,
        path: PathBuf,
        errno: i32,
    },
}

impl fmt::Display for PrivateSocketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrivateSocketError::BadPath { path } => write!(
                f,
                "{} is not an absolute path of normal components (no empty, . or .. components)",
                path.display()
            ),
            PrivateSocketError::SymlinkInPath { path } => write!(
                f,
                "{} is a symlink; a private socket path must not pass through one",
                path.display()
            ),
            PrivateSocketError::NoDirectory { path } => {
                write!(f, "{} does not exist", path.display())
            }
            PrivateSocketError::NoSocket { path } => write!(
                f,
                "{} does not exist: no socket is bound at that name",
                path.display()
            ),
            PrivateSocketError::NotDirectory { path } => {
                write!(f, "{} is not a directory", path.display())
            }
            PrivateSocketError::ForeignOwner { path, uid } => write!(
                f,
                "{} is owned by uid {uid}, which is not trusted here",
                path.display()
            ),
            PrivateSocketError::Replaceable { path, mode } => write!(
                f,
                "{} has mode {mode:04o}: group or other can replace entries in it",
                path.display()
            ),
            PrivateSocketError::ReplaceableByAcl { path } => write!(
                f,
                "{} has an ACL allow entry granting add_file, add_subdirectory, delete_child, \
                 delete, writesecurity or chown; such an entry is refused on an ancestor whoever \
                 it names, even if it applies only to new children",
                path.display()
            ),
            PrivateSocketError::OwnershipIgnored { path } => write!(
                f,
                "{} is on a volume mounted with ownership ignored (noowners): its owner and mode \
                 do not keep other users out",
                path.display()
            ),
            PrivateSocketError::NotPrivate { path, mode } => write!(
                f,
                "{} has mode {mode:04o}: a private socket directory grants group and other nothing",
                path.display()
            ),
            PrivateSocketError::AclPresent { path } => write!(
                f,
                "{} carries an ACL: a private socket directory has none",
                path.display()
            ),
            PrivateSocketError::Replaced { path } => write!(
                f,
                "{} no longer names the directory that was opened",
                path.display()
            ),
            PrivateSocketError::BadName { name } => {
                write!(f, "{name:?} is not a single path component")
            }
            PrivateSocketError::PathTooLong { path, max } => write!(
                f,
                "{} is longer than the {max} bytes a socket address holds",
                path.display()
            ),
            PrivateSocketError::AddressInUse { path } => {
                write!(f, "{} already exists and is left untouched", path.display())
            }
            PrivateSocketError::NotASocket { path } => {
                write!(f, "{} is not a socket", path.display())
            }
            PrivateSocketError::SocketOwner { path, uid } => write!(
                f,
                "{} is owned by uid {uid}, not by the trusted uid",
                path.display()
            ),
            PrivateSocketError::SocketMode { path, mode } => write!(
                f,
                "{} has mode {mode:04o}: a private socket is exactly 0600",
                path.display()
            ),
            PrivateSocketError::BindEscaped { path } => write!(
                f,
                "the socket bound at {} cannot be shown to be inside the directory that was opened",
                path.display()
            ),
            PrivateSocketError::PeerCredentials { errno } => write!(
                f,
                "the kernel did not report the peer's credentials (errno {errno})"
            ),
            PrivateSocketError::PeerMismatch { trusted, found } => write!(
                f,
                "the peer runs as uid {found}, not the trusted uid {trusted}"
            ),
            PrivateSocketError::Io { op, path, errno } => {
                write!(f, "{op} failed at {} (errno {errno})", path.display())
            }
        }
    }
}

impl std::error::Error for PrivateSocketError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Directory,
    Socket,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Facts {
    kind: Kind,
    uid: u32,
    mode: u32,
    dev: u64,
    ino: u64,
}

impl Facts {
    fn of(stat: &libc::stat) -> Facts {
        let kind = match stat.st_mode & libc::S_IFMT {
            libc::S_IFDIR => Kind::Directory,
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
}

#[cfg(target_os = "macos")]
fn mode_and_device(stat: &libc::stat) -> (u32, u64) {
    (u32::from(stat.st_mode), stat.st_dev as u64)
}

#[cfg(target_os = "linux")]
fn mode_and_device(stat: &libc::stat) -> (u32, u64) {
    (stat.st_mode, stat.st_dev)
}

fn judge_ancestor(facts: &Facts, euid: u32, at: &Path) -> Result<(), PrivateSocketError> {
    if facts.kind != Kind::Directory {
        return Err(PrivateSocketError::NotDirectory {
            path: at.to_path_buf(),
        });
    }
    if facts.uid != 0 && facts.uid != euid {
        return Err(PrivateSocketError::ForeignOwner {
            path: at.to_path_buf(),
            uid: facts.uid,
        });
    }
    if facts.mode & 0o022 != 0 {
        return Err(PrivateSocketError::Replaceable {
            path: at.to_path_buf(),
            mode: facts.mode,
        });
    }
    Ok(())
}

fn judge_private(facts: &Facts, euid: u32, path: &Path) -> Result<(), PrivateSocketError> {
    if facts.kind != Kind::Directory {
        return Err(PrivateSocketError::NotDirectory {
            path: path.to_path_buf(),
        });
    }
    if facts.uid != euid {
        return Err(PrivateSocketError::ForeignOwner {
            path: path.to_path_buf(),
            uid: facts.uid,
        });
    }
    if facts.mode & 0o077 != 0 {
        return Err(PrivateSocketError::NotPrivate {
            path: path.to_path_buf(),
            mode: facts.mode,
        });
    }
    Ok(())
}

fn judge_socket(facts: &Facts, trusted_uid: u32, path: &Path) -> Result<(), PrivateSocketError> {
    if facts.kind != Kind::Socket {
        return Err(PrivateSocketError::NotASocket {
            path: path.to_path_buf(),
        });
    }
    if facts.uid != trusted_uid {
        return Err(PrivateSocketError::SocketOwner {
            path: path.to_path_buf(),
            uid: facts.uid,
        });
    }
    if facts.mode & 0o777 != 0o600 {
        return Err(PrivateSocketError::SocketMode {
            path: path.to_path_buf(),
            mode: facts.mode,
        });
    }
    Ok(())
}

fn judge_peer(trusted: u32, found: u32) -> Result<(), PrivateSocketError> {
    if trusted == found {
        Ok(())
    } else {
        Err(PrivateSocketError::PeerMismatch { trusted, found })
    }
}

fn prepare(dir: PrivateDir, name: &str) -> Result<(OwnedFd, BoundEntry), PrivateSocketError> {
    prepare_with(dir, name, || {})
}

fn prepare_with(
    dir: PrivateDir,
    name: &str,
    after_bind: impl FnOnce(),
) -> Result<(OwnedFd, BoundEntry), PrivateSocketError> {
    prepare_as(dir, name, effective_uid(), after_bind)
}

fn prepare_as(
    dir: PrivateDir,
    name: &str,
    owner: u32,
    after_bind: impl FnOnce(),
) -> Result<(OwnedFd, BoundEntry), PrivateSocketError> {
    let c_name = entry_name(name)?;
    let path = dir.path.join(name);
    let (address, length) = address_for(&path)?;
    dir.reconfirm()?;
    match stat_at(&dir.dir, &c_name) {
        Ok(_) => return Err(PrivateSocketError::AddressInUse { path }),
        Err(errno) if errno == libc::ENOENT => {}
        Err(errno) => {
            return Err(PrivateSocketError::Io {
                op: "fstatat",
                path,
                errno,
            });
        }
    }
    let socket = unix_socket().map_err(|error| io_failure("socket", &path, &error))?;
    let outcome = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length,
        )
    };
    if outcome != 0 {
        let error = io::Error::last_os_error();
        return Err(if error.raw_os_error() == Some(libc::EADDRINUSE) {
            PrivateSocketError::AddressInUse { path }
        } else {
            io_failure("bind", &path, &error)
        });
    }
    after_bind();
    let recheck = dir.reconfirm();
    let created = match stat_at(&dir.dir, &c_name) {
        Ok(facts) if facts.kind == Kind::Socket && facts.uid == owner => facts,
        _ => return Err(PrivateSocketError::BindEscaped { path }),
    };
    let bound = BoundEntry {
        dir,
        name: c_name,
        entry: SocketEntry {
            path,
            dev: created.dev,
            ino: created.ino,
        },
    };
    recheck?;
    restrict_to_owner(&bound)?;
    let facts = stat_at(&bound.dir.dir, &bound.name)
        .map_err(|errno| no_socket_or_io(errno, &bound.entry.path))?;
    if facts.dev != created.dev || facts.ino != created.ino {
        return Err(PrivateSocketError::BindEscaped {
            path: bound.entry.path.clone(),
        });
    }
    judge_socket(&facts, owner, &bound.entry.path)?;
    Ok((socket, bound))
}

fn restrict_to_owner(bound: &BoundEntry) -> Result<(), PrivateSocketError> {
    let dir = bound.dir.dir.as_raw_fd();
    let name = bound.name.as_ptr();
    if unsafe { libc::fchmodat(dir, name, 0o600, libc::AT_SYMLINK_NOFOLLOW) } == 0 {
        return Ok(());
    }
    let errno = last_errno();
    if errno != libc::EOPNOTSUPP && errno != libc::ENOTSUP {
        return Err(PrivateSocketError::Io {
            op: "fchmodat",
            path: bound.entry.path.clone(),
            errno,
        });
    }
    match stat_at(&bound.dir.dir, &bound.name) {
        Ok(facts)
            if facts.kind == Kind::Socket
                && facts.dev == bound.entry.dev
                && facts.ino == bound.entry.ino => {}
        _ => {
            return Err(PrivateSocketError::BindEscaped {
                path: bound.entry.path.clone(),
            });
        }
    }
    if unsafe { libc::fchmodat(dir, name, 0o600, 0) } == 0 {
        return Ok(());
    }
    let errno = last_errno();
    Err(PrivateSocketError::Io {
        op: "fchmodat",
        path: bound.entry.path.clone(),
        errno,
    })
}

fn absolute_components(path: &Path) -> Result<Vec<CString>, PrivateSocketError> {
    let Some(relative) = path.as_os_str().as_bytes().strip_prefix(b"/") else {
        return Err(PrivateSocketError::BadPath {
            path: path.to_path_buf(),
        });
    };
    if relative.is_empty() {
        Ok(Vec::new())
    } else {
        components_of(relative, path)
    }
}

fn components_of(relative: &[u8], whole: &Path) -> Result<Vec<CString>, PrivateSocketError> {
    relative
        .split(|byte| *byte == b'/')
        .map(|component| match component {
            b"" | b"." | b".." => None,
            component => CString::new(component).ok(),
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| PrivateSocketError::BadPath {
            path: whole.to_path_buf(),
        })
}

fn entry_name(name: &str) -> Result<CString, PrivateSocketError> {
    let bad = || PrivateSocketError::BadName {
        name: name.to_string(),
    };
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(bad());
    }
    CString::new(name).map_err(|_| bad())
}

fn address_for(path: &Path) -> Result<(libc::sockaddr_un, libc::socklen_t), PrivateSocketError> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let max = address.sun_path.len() - 1;
    if bytes.len() > max {
        return Err(PrivateSocketError::PathTooLong {
            path: path.to_path_buf(),
            max,
        });
    }
    address.sun_family = libc::AF_UNIX as _;
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
    Ok((address, length as libc::socklen_t))
}

fn open_root() -> io::Result<OwnedFd> {
    let raw = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    }
}

#[cfg(target_os = "macos")]
const SEARCH_ONLY: libc::c_int = libc::O_SEARCH;

#[cfg(target_os = "linux")]
const SEARCH_ONLY: libc::c_int = libc::O_PATH;

fn open_child(
    parent: &OwnedFd,
    name: &CStr,
    at: &Path,
    access: libc::c_int,
) -> Result<OwnedFd, PrivateSocketError> {
    let raw = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            access | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw >= 0 {
        return Ok(unsafe { OwnedFd::from_raw_fd(raw) });
    }
    let errno = last_errno();
    let at = at.to_path_buf();
    Err(match errno {
        libc::ELOOP => PrivateSocketError::SymlinkInPath { path: at },
        libc::ENOENT => PrivateSocketError::NoDirectory { path: at },
        libc::ENOTDIR => match stat_at(parent, name) {
            Ok(facts) if facts.kind == Kind::Symlink => {
                PrivateSocketError::SymlinkInPath { path: at }
            }
            _ => PrivateSocketError::NotDirectory { path: at },
        },
        errno => PrivateSocketError::Io {
            op: "openat",
            path: at,
            errno,
        },
    })
}

fn stat_fd(fd: &OwnedFd, at: &Path) -> Result<Facts, PrivateSocketError> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } == 0 {
        Ok(Facts::of(&stat))
    } else {
        Err(io_failure("fstat", at, &io::Error::last_os_error()))
    }
}

fn stat_at(dir: &OwnedFd, name: &CStr) -> Result<Facts, i32> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
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

fn unix_socket() -> io::Result<OwnedFd> {
    #[cfg(target_os = "linux")]
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    #[cfg(target_os = "macos")]
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    #[cfg(target_os = "macos")]
    if unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(socket)
}

fn refuse_if(
    found: Result<bool, (&'static str, i32)>,
    at: &Path,
    refusal: impl FnOnce(PathBuf) -> PrivateSocketError,
) -> Result<(), PrivateSocketError> {
    match found {
        Ok(false) => Ok(()),
        Ok(true) => Err(refusal(at.to_path_buf())),
        Err((op, errno)) => Err(PrivateSocketError::Io {
            op,
            path: at.to_path_buf(),
            errno,
        }),
    }
}

fn refuse_acl(dir: &OwnedFd, path: &Path) -> Result<(), PrivateSocketError> {
    refuse_if(acl_present(dir), path, |path| {
        PrivateSocketError::AclPresent { path }
    })
}

#[cfg(target_os = "macos")]
fn refuse_replacing_acl(dir: &OwnedFd, at: &Path) -> Result<(), PrivateSocketError> {
    refuse_if(darwin_acl::allows_replacing_entries(dir), at, |at| {
        PrivateSocketError::ReplaceableByAcl { path: at }
    })
}

#[cfg(target_os = "linux")]
fn refuse_replacing_acl(_dir: &OwnedFd, _at: &Path) -> Result<(), PrivateSocketError> {
    Ok(())
}

#[cfg(target_os = "macos")]
mod darwin_acl {
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::raw::{c_int, c_void};

    const ACL_TYPE_EXTENDED: c_int = 0x0000_0100;
    const ACL_FIRST_ENTRY: c_int = 0;
    const ACL_NEXT_ENTRY: c_int = -1;
    const ACL_EXTENDED_ALLOW: c_int = 1;
    const ACL_ADD_FILE: u64 = 1 << 2;
    const ACL_DELETE: u64 = 1 << 4;
    const ACL_ADD_SUBDIRECTORY: u64 = 1 << 5;
    const ACL_DELETE_CHILD: u64 = 1 << 6;
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

    unsafe extern "C" {
        fn acl_get_fd_np(fd: c_int, kind: c_int) -> *mut c_void;
        fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
        fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_int) -> c_int;
        fn acl_get_permset_mask_np(entry: *mut c_void, mask: *mut u64) -> c_int;
        fn acl_free(object: *mut c_void) -> c_int;
    }

    struct Acl(*mut c_void);

    impl Acl {
        fn of(dir: &OwnedFd) -> Result<Option<Acl>, (&'static str, i32)> {
            let acl = unsafe { acl_get_fd_np(dir.as_raw_fd(), ACL_TYPE_EXTENDED) };
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

        fn entries(&self) -> impl Iterator<Item = *mut c_void> + '_ {
            let mut which = ACL_FIRST_ENTRY;
            std::iter::from_fn(move || {
                let mut entry = std::ptr::null_mut();
                let found = unsafe { acl_get_entry(self.0, which, &mut entry) } == 0;
                which = ACL_NEXT_ENTRY;
                found.then_some(entry)
            })
        }
    }

    impl Drop for Acl {
        fn drop(&mut self) {
            unsafe { acl_free(self.0) };
        }
    }

    pub(super) fn has_entries(dir: &OwnedFd) -> Result<bool, (&'static str, i32)> {
        Ok(Acl::of(dir)?.is_some_and(|acl| acl.entries().next().is_some()))
    }

    pub(super) fn allows_replacing_entries(dir: &OwnedFd) -> Result<bool, (&'static str, i32)> {
        let Some(acl) = Acl::of(dir)? else {
            return Ok(false);
        };
        for entry in acl.entries() {
            let mut tag = 0;
            if unsafe { acl_get_tag_type(entry, &mut tag) } != 0 {
                return Err(("acl_get_tag_type", super::last_errno()));
            }
            if tag != ACL_EXTENDED_ALLOW {
                continue;
            }
            let mut mask = 0;
            if unsafe { acl_get_permset_mask_np(entry, &mut mask) } != 0 {
                return Err(("acl_get_permset_mask_np", super::last_errno()));
            }
            if mask & REPLACES_ENTRIES != 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(target_os = "macos")]
fn acl_present(dir: &OwnedFd) -> Result<bool, (&'static str, i32)> {
    darwin_acl::has_entries(dir)
}

#[cfg(target_os = "linux")]
fn acl_present(dir: &OwnedFd) -> Result<bool, (&'static str, i32)> {
    for attribute in [c"system.posix_acl_access", c"system.posix_acl_default"] {
        let size = unsafe {
            libc::fgetxattr(dir.as_raw_fd(), attribute.as_ptr(), std::ptr::null_mut(), 0)
        };
        if size >= 0 {
            return Ok(true);
        }
        let errno = last_errno();
        if errno != libc::ENODATA && errno != libc::ENOTSUP {
            return Err(("fgetxattr", errno));
        }
    }
    Ok(false)
}

fn no_socket_or_io(errno: i32, path: &Path) -> PrivateSocketError {
    if errno == libc::ENOENT {
        PrivateSocketError::NoSocket {
            path: path.to_path_buf(),
        }
    } else {
        PrivateSocketError::Io {
            op: "fstatat",
            path: path.to_path_buf(),
            errno,
        }
    }
}

fn io_failure(op: &'static str, at: &Path, error: &io::Error) -> PrivateSocketError {
    PrivateSocketError::Io {
        op,
        path: at.to_path_buf(),
        errno: errno_of(error),
    }
}

fn errno_of(error: &io::Error) -> i32 {
    error.raw_os_error().unwrap_or(libc::EIO)
}

fn last_errno() -> i32 {
    errno_of(&io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::fd::FromRawFd;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};
    use tempfile::TempDir;

    const EUID: u32 = 501;

    fn facts(kind: Kind, uid: u32, mode: u32) -> Facts {
        Facts {
            kind,
            uid,
            mode,
            dev: 1,
            ino: 2,
        }
    }

    struct TestRoot(TempDir);

    impl TestRoot {
        fn path(&self) -> &Path {
            self.0.path()
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            open_up(self.path());
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

    fn test_root() -> TestRoot {
        let root = tempfile::tempdir().expect("tempdir");
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).expect("chmod");
        TestRoot(root)
    }

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
    }

    fn start(root: &TestRoot) -> OwnedFd {
        OwnedFd::from(fs::File::open(root.path()).expect("the test root opens"))
    }

    fn make_dir(path: &Path, mode: u32) {
        fs::create_dir(path).expect("mkdir");
        set_mode(path, mode);
    }

    fn open_in(root: &TestRoot, relative: &str) -> Result<PrivateDir, PrivateSocketError> {
        PrivateDir::open_from(start(root), root.path(), Path::new(relative))
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
                Err(PrivateSocketError::Replaceable {
                    path: at.to_path_buf(),
                    mode
                })
            );
        }
        assert_eq!(
            judge_ancestor(&facts(Kind::Directory, 4242, 0o755), EUID, at),
            Err(PrivateSocketError::ForeignOwner {
                path: at.to_path_buf(),
                uid: 4242
            })
        );
        assert_eq!(
            judge_ancestor(&facts(Kind::Other, 0, 0o755), EUID, at),
            Err(PrivateSocketError::NotDirectory {
                path: at.to_path_buf()
            })
        );
    }

    #[test]
    fn private_judge_refuses_any_group_or_other_bit() {
        let path = Path::new("/a/cell");
        for mode in [0o700, 0o500] {
            assert_eq!(
                judge_private(&facts(Kind::Directory, EUID, mode), EUID, path),
                Ok(())
            );
        }
        for mode in [0o710, 0o701, 0o750] {
            assert_eq!(
                judge_private(&facts(Kind::Directory, EUID, mode), EUID, path),
                Err(PrivateSocketError::NotPrivate {
                    path: path.to_path_buf(),
                    mode
                })
            );
        }
        assert_eq!(
            judge_private(&facts(Kind::Directory, 0, 0o700), EUID, path),
            Err(PrivateSocketError::ForeignOwner {
                path: path.to_path_buf(),
                uid: 0
            })
        );
        assert_eq!(
            judge_private(&facts(Kind::Symlink, EUID, 0o700), EUID, path),
            Err(PrivateSocketError::NotDirectory {
                path: path.to_path_buf()
            })
        );
    }

    #[test]
    fn socket_judge_requires_owner_and_exact_0600() {
        let path = Path::new("/a/cell/sock");
        assert_eq!(
            judge_socket(&facts(Kind::Socket, EUID, 0o600), EUID, path),
            Ok(())
        );
        for mode in [0o660, 0o700, 0o644, 0o400] {
            assert_eq!(
                judge_socket(&facts(Kind::Socket, EUID, mode), EUID, path),
                Err(PrivateSocketError::SocketMode {
                    path: path.to_path_buf(),
                    mode
                })
            );
        }
        assert_eq!(
            judge_socket(&facts(Kind::Socket, 4242, 0o600), EUID, path),
            Err(PrivateSocketError::SocketOwner {
                path: path.to_path_buf(),
                uid: 4242
            })
        );
        for kind in [Kind::Other, Kind::Symlink, Kind::Directory] {
            assert_eq!(
                judge_socket(&facts(kind, EUID, 0o600), EUID, path),
                Err(PrivateSocketError::NotASocket {
                    path: path.to_path_buf()
                })
            );
        }
    }

    #[test]
    fn peer_judge_requires_equal_uids() {
        assert_eq!(judge_peer(EUID, EUID), Ok(()));
        assert_eq!(
            judge_peer(EUID, 0),
            Err(PrivateSocketError::PeerMismatch {
                trusted: EUID,
                found: 0
            })
        );
    }

    #[test]
    fn open_from_refuses_a_symlinked_component_without_following_it() {
        let root = test_root();
        make_dir(&root.path().join("a"), 0o700);
        make_dir(&root.path().join("b"), 0o700);
        make_dir(&root.path().join("b/cell"), 0o700);
        symlink("../b", root.path().join("a/link")).expect("symlink");
        symlink("../b/cell", root.path().join("a/last")).expect("symlink");
        assert_eq!(
            open_in(&root, "a/link/cell").unwrap_err(),
            PrivateSocketError::SymlinkInPath {
                path: root.path().join("a/link")
            }
        );
        assert_eq!(
            open_in(&root, "a/last").unwrap_err(),
            PrivateSocketError::SymlinkInPath {
                path: root.path().join("a/last")
            }
        );
    }

    #[test]
    fn open_from_refuses_a_group_writable_intermediate() {
        let root = test_root();
        make_dir(&root.path().join("a"), 0o775);
        make_dir(&root.path().join("a/cell"), 0o700);
        assert_eq!(
            open_in(&root, "a/cell").unwrap_err(),
            PrivateSocketError::Replaceable {
                path: root.path().join("a"),
                mode: 0o775
            }
        );
    }

    #[test]
    fn open_from_refuses_a_group_writable_ancestor_two_levels_up() {
        let root = test_root();
        make_dir(&root.path().join("a"), 0o775);
        make_dir(&root.path().join("a/b"), 0o755);
        make_dir(&root.path().join("a/b/cell"), 0o700);
        assert_eq!(
            open_in(&root, "a/b/cell").map(|dir| dir.path().to_path_buf()),
            Err(PrivateSocketError::Replaceable {
                path: root.path().join("a"),
                mode: 0o775
            })
        );
    }

    #[test]
    fn open_from_refuses_a_0750_final_directory() {
        let root = test_root();
        make_dir(&root.path().join("cell"), 0o750);
        assert_eq!(
            open_in(&root, "cell").unwrap_err(),
            PrivateSocketError::NotPrivate {
                path: root.path().join("cell"),
                mode: 0o750
            }
        );
    }

    #[test]
    fn open_from_refuses_a_missing_component_and_a_file_in_the_path() {
        let root = test_root();
        fs::write(root.path().join("file"), b"").expect("write");
        assert_eq!(
            open_in(&root, "absent/cell").unwrap_err(),
            PrivateSocketError::NoDirectory {
                path: root.path().join("absent")
            }
        );
        assert_eq!(
            open_in(&root, "file/cell").unwrap_err(),
            PrivateSocketError::NotDirectory {
                path: root.path().join("file")
            }
        );
        assert_eq!(
            open_in(&root, "file").unwrap_err(),
            PrivateSocketError::NotDirectory {
                path: root.path().join("file")
            }
        );
    }

    #[test]
    fn open_from_accepts_a_0700_final_directory_and_reconfirm_passes() {
        let root = test_root();
        make_dir(&root.path().join("a"), 0o755);
        make_dir(&root.path().join("a/cell"), 0o700);
        let dir = open_in(&root, "a/cell").expect("a private directory opens");
        let metadata = fs::metadata(root.path().join("a/cell")).expect("stat");
        assert_eq!(dir.path(), root.path().join("a/cell"));
        assert_eq!(
            dir.identity(),
            DirIdentity {
                dev: metadata.dev(),
                ino: metadata.ino()
            }
        );
        assert_eq!(dir.reconfirm(), Ok(()));
    }

    #[test]
    fn open_from_passes_an_intermediate_it_can_search_but_not_read() {
        let root = test_root();
        let a = root.path().join("a");
        make_dir(&a, 0o755);
        make_dir(&a.join("cell"), 0o700);
        set_mode(&a, 0o311);
        let opened = open_in(&root, "a/cell");
        let reconfirmed = opened.as_ref().ok().map(PrivateDir::reconfirm);
        set_mode(&a, 0o755);
        assert_eq!(
            opened.expect("a search-only intermediate passes").path(),
            a.join("cell")
        );
        assert_eq!(reconfirmed, Some(Ok(())));
    }

    #[test]
    fn open_from_judges_a_directory_it_cannot_open() {
        let root = test_root();
        let group = root.path().join("group");
        make_dir(&group, 0o700);
        make_dir(&group.join("cell"), 0o700);
        set_mode(&group, 0o020);
        let shared = root.path().join("shared");
        make_dir(&shared, 0o070);
        let locked = root.path().join("locked");
        make_dir(&locked, 0o000);
        assert_eq!(
            open_in(&root, "group/cell").map(|dir| dir.path().to_path_buf()),
            Err(PrivateSocketError::Replaceable {
                path: group.clone(),
                mode: 0o020
            })
        );
        assert_eq!(
            open_in(&root, "shared").map(|dir| dir.path().to_path_buf()),
            Err(PrivateSocketError::NotPrivate {
                path: shared,
                mode: 0o070
            })
        );
        assert_eq!(
            open_in(&root, "locked").map(|dir| dir.path().to_path_buf()),
            Err(PrivateSocketError::Io {
                op: "openat",
                path: locked,
                errno: libc::EACCES
            }),
            "a directory this user owns and keeps private is only unreachable"
        );
    }

    #[test]
    fn reconfirm_fails_after_the_directory_is_replaced() {
        let root = test_root();
        make_dir(&root.path().join("cell"), 0o700);
        let dir = open_in(&root, "cell").expect("a private directory opens");
        fs::rename(root.path().join("cell"), root.path().join("moved")).expect("rename");
        make_dir(&root.path().join("cell"), 0o700);
        assert_eq!(
            dir.reconfirm(),
            Err(PrivateSocketError::Replaced {
                path: root.path().join("cell")
            })
        );
    }

    #[test]
    fn reconfirm_fails_when_the_directory_is_no_longer_private() {
        let root = test_root();
        make_dir(&root.path().join("cell"), 0o700);
        let dir = open_in(&root, "cell").expect("a private directory opens");
        fs::set_permissions(root.path().join("cell"), fs::Permissions::from_mode(0o750))
            .expect("chmod");
        assert_eq!(
            dir.reconfirm(),
            Err(PrivateSocketError::NotPrivate {
                path: root.path().join("cell"),
                mode: 0o750
            })
        );
    }

    #[test]
    fn open_refuses_a_relative_or_unnormal_path() {
        for path in [
            "relative/cell",
            "/a/../cell",
            "/a/./cell",
            "/a//cell",
            "/a/cell/",
        ] {
            assert_eq!(
                PrivateDir::open(Path::new(path)).unwrap_err(),
                PrivateSocketError::BadPath {
                    path: PathBuf::from(path)
                }
            );
        }
    }

    #[test]
    fn prepare_leaves_a_0600_socket_that_refuses_connections() {
        let root = test_root();
        make_dir(&root.path().join("cell"), 0o700);
        let dir = open_in(&root, "cell").expect("a private directory opens");
        let (socket, bound) = prepare(dir, "sock").expect("prepare binds");
        let path = root.path().join("cell/sock");
        let metadata = fs::symlink_metadata(&path).expect("the socket exists");
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.uid(), effective_uid());
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(
            bound.entry,
            SocketEntry {
                path: path.clone(),
                dev: metadata.dev(),
                ino: metadata.ino()
            }
        );
        let refused = UnixStream::connect(&path).expect_err("nothing listens yet");
        assert_eq!(refused.raw_os_error(), Some(libc::ECONNREFUSED));
        drop(socket);
        drop(bound);
        assert!(fs::symlink_metadata(&path).is_err());
    }

    fn refusal_of(
        outcome: Result<(OwnedFd, BoundEntry), PrivateSocketError>,
    ) -> PrivateSocketError {
        match outcome {
            Ok((_, bound)) => panic!("prepare bound {:?}", bound.entry),
            Err(refusal) => refusal,
        }
    }

    fn entries(path: &Path) -> Vec<PathBuf> {
        fs::read_dir(path)
            .expect("readdir")
            .map(|entry| entry.expect("an entry").path())
            .collect()
    }

    #[test]
    fn prepare_refuses_a_directory_that_stopped_being_private_without_binding() {
        let root = test_root();
        let cell = root.path().join("cell");
        make_dir(&cell, 0o700);
        let dir = open_in(&root, "cell").expect("a private directory opens");
        set_mode(&cell, 0o750);
        let mut bound = false;
        let refusal = refusal_of(prepare_with(dir, "sock", || bound = true));
        assert_eq!(
            refusal,
            PrivateSocketError::NotPrivate {
                path: cell.clone(),
                mode: 0o750
            }
        );
        assert!(
            !bound,
            "the socket was bound in a directory no longer private"
        );
        assert_eq!(entries(&cell), Vec::<PathBuf>::new());
    }

    #[test]
    fn prepare_reports_the_recheck_and_removes_a_socket_bound_before_the_directory_changed() {
        let root = test_root();
        let cell = root.path().join("cell");
        make_dir(&cell, 0o700);
        let dir = open_in(&root, "cell").expect("a private directory opens");
        let refusal = refusal_of(prepare_with(dir, "sock", || set_mode(&cell, 0o750)));
        assert_eq!(
            refusal,
            PrivateSocketError::NotPrivate {
                path: cell.clone(),
                mode: 0o750
            }
        );
        assert_eq!(entries(&cell), Vec::<PathBuf>::new());
    }

    #[test]
    fn prepare_reports_a_directory_replaced_after_bind_and_removes_the_socket() {
        let root = test_root();
        let cell = root.path().join("cell");
        let moved = root.path().join("moved");
        make_dir(&cell, 0o700);
        let dir = open_in(&root, "cell").expect("a private directory opens");
        let refusal = refusal_of(prepare_with(dir, "sock", || {
            fs::rename(&cell, &moved).expect("rename");
            make_dir(&cell, 0o700);
        }));
        assert_eq!(refusal, PrivateSocketError::Replaced { path: cell.clone() });
        assert_eq!(entries(&moved), Vec::<PathBuf>::new());
        assert_eq!(entries(&cell), Vec::<PathBuf>::new());
    }

    #[test]
    fn prepare_reports_bind_escaped_and_leaves_the_name_when_no_socket_is_there() {
        let root = test_root();
        let cell = root.path().join("cell");
        make_dir(&cell, 0o700);
        let sock = cell.join("sock");
        let dir = open_in(&root, "cell").expect("a private directory opens");
        let refusal = refusal_of(prepare_with(dir, "sock", || {
            fs::remove_file(&sock).expect("unlink");
        }));
        assert_eq!(
            refusal,
            PrivateSocketError::BindEscaped { path: sock.clone() }
        );
        assert_eq!(entries(&cell), Vec::<PathBuf>::new());

        let dir = open_in(&root, "cell").expect("a private directory opens");
        let refusal = refusal_of(prepare_with(dir, "sock", || {
            fs::remove_file(&sock).expect("unlink");
            fs::write(&sock, b"planted").expect("write");
        }));
        assert_eq!(
            refusal,
            PrivateSocketError::BindEscaped { path: sock.clone() }
        );
        assert_eq!(fs::read(&sock).expect("the planted file stays"), b"planted");
    }

    #[test]
    fn prepare_leaves_a_socket_it_did_not_bind_when_the_recheck_fails() {
        let root = test_root();
        let cell = root.path().join("cell");
        make_dir(&cell, 0o700);
        let sock = cell.join("sock");
        let dir = open_in(&root, "cell").expect("a private directory opens");
        let mut other = None;
        let stand_in = effective_uid().wrapping_add(1);
        let refusal = refusal_of(prepare_as(dir, "sock", stand_in, || {
            fs::remove_file(&sock).expect("unlink");
            other = Some(UnixListener::bind(&sock).expect("bind another socket"));
            set_mode(&cell, 0o770);
        }));
        let left = fs::symlink_metadata(&sock).expect("the socket this call did not bind stays");
        assert!(left.file_type().is_socket());
        assert_eq!(refusal, PrivateSocketError::BindEscaped { path: sock });
        drop(other);
    }

    #[test]
    fn a_test_root_is_removed_even_when_its_test_panics() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let root = test_root();
            sender
                .send(root.path().to_path_buf())
                .expect("the root path is sent");
            let a = root.path().join("a");
            make_dir(&a, 0o755);
            make_dir(&a.join("cell"), 0o700);
            set_mode(&a, 0o311);
            panic!("deliberate panic: the root must still be removed");
        }));
        assert!(outcome.is_err(), "the closure panics");
        let path = receiver.recv().expect("the root path");
        assert!(
            fs::symlink_metadata(&path).is_err(),
            "{} was left behind",
            path.display()
        );
    }

    #[test]
    fn check_peer_uid_refuses_a_socket_without_peer_credentials() {
        let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        assert!(raw >= 0, "socket: {}", std::io::Error::last_os_error());
        let stream = UnixStream::from(unsafe { OwnedFd::from_raw_fd(raw) });
        assert_eq!(
            peer_uid(&stream).unwrap_err().raw_os_error(),
            Some(libc::ENOTCONN)
        );
        assert_eq!(
            check_peer_uid(&stream, effective_uid()),
            Err(PrivateSocketError::PeerCredentials {
                errno: libc::ENOTCONN
            })
        );
    }

    #[test]
    fn errors_name_the_path_and_the_rule() {
        let at = || PathBuf::from("/r/cell");
        let cases = [
            (
                PrivateSocketError::BadPath { path: at() },
                "/r/cell is not an absolute path of normal components (no empty, . or .. components)",
            ),
            (
                PrivateSocketError::SymlinkInPath { path: at() },
                "/r/cell is a symlink; a private socket path must not pass through one",
            ),
            (
                PrivateSocketError::NoDirectory { path: at() },
                "/r/cell does not exist",
            ),
            (
                PrivateSocketError::NoSocket { path: at() },
                "/r/cell does not exist: no socket is bound at that name",
            ),
            (
                PrivateSocketError::NotDirectory { path: at() },
                "/r/cell is not a directory",
            ),
            (
                PrivateSocketError::ForeignOwner {
                    path: at(),
                    uid: 42,
                },
                "/r/cell is owned by uid 42, which is not trusted here",
            ),
            (
                PrivateSocketError::Replaceable {
                    path: at(),
                    mode: 0o1777,
                },
                "/r/cell has mode 1777: group or other can replace entries in it",
            ),
            (
                PrivateSocketError::ReplaceableByAcl { path: at() },
                "/r/cell has an ACL allow entry granting add_file, add_subdirectory, delete_child, \
                 delete, writesecurity or chown; such an entry is refused on an ancestor whoever it \
                 names, even if it applies only to new children",
            ),
            (
                PrivateSocketError::OwnershipIgnored { path: at() },
                "/r/cell is on a volume mounted with ownership ignored (noowners): its owner and \
                 mode do not keep other users out",
            ),
            (
                PrivateSocketError::NotPrivate {
                    path: at(),
                    mode: 0o750,
                },
                "/r/cell has mode 0750: a private socket directory grants group and other nothing",
            ),
            (
                PrivateSocketError::AclPresent { path: at() },
                "/r/cell carries an ACL: a private socket directory has none",
            ),
            (
                PrivateSocketError::Replaced { path: at() },
                "/r/cell no longer names the directory that was opened",
            ),
            (
                PrivateSocketError::BadName {
                    name: "a/b".to_string(),
                },
                "\"a/b\" is not a single path component",
            ),
            (
                PrivateSocketError::PathTooLong {
                    path: at(),
                    max: 103,
                },
                "/r/cell is longer than the 103 bytes a socket address holds",
            ),
            (
                PrivateSocketError::AddressInUse { path: at() },
                "/r/cell already exists and is left untouched",
            ),
            (
                PrivateSocketError::NotASocket { path: at() },
                "/r/cell is not a socket",
            ),
            (
                PrivateSocketError::SocketOwner {
                    path: at(),
                    uid: 42,
                },
                "/r/cell is owned by uid 42, not by the trusted uid",
            ),
            (
                PrivateSocketError::SocketMode {
                    path: at(),
                    mode: 0o660,
                },
                "/r/cell has mode 0660: a private socket is exactly 0600",
            ),
            (
                PrivateSocketError::BindEscaped { path: at() },
                "the socket bound at /r/cell cannot be shown to be inside the directory that was opened",
            ),
            (
                PrivateSocketError::PeerCredentials { errno: 57 },
                "the kernel did not report the peer's credentials (errno 57)",
            ),
            (
                PrivateSocketError::PeerMismatch {
                    trusted: 501,
                    found: 0,
                },
                "the peer runs as uid 0, not the trusted uid 501",
            ),
            (
                PrivateSocketError::Io {
                    op: "fstatat",
                    path: at(),
                    errno: 13,
                },
                "fstatat failed at /r/cell (errno 13)",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
        }
    }
}
