use std::ffi::{CString, OsStr};
use std::fmt;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::instance::{DirectoryStream, rename_no_replace};
#[cfg(target_os = "macos")]
use crate::nofollow::darwin_acl;
pub use crate::nofollow::{DirIdentity, effective_uid};
use crate::nofollow::{
    Facts, Kind, Opened, Walk, WalkRefusal, absolute_components, errno_of, judge_ancestor,
    last_errno, refuse_if, refuse_replacing_acl, stat_at,
};

/// A socket's parent directory, opened without following symlinks and judged private: every
/// ancestor is owned by root or the effective UID and writable by neither group nor other, and
/// on macOS carries no ACL allow entry granting add_file, add_subdirectory, delete_child,
/// delete, writesecurity or chown, whoever it names. The directory itself is owned by the
/// effective UID, has no group or other bits and carries no ACL. On macOS, no directory on the
/// path may sit on a volume mounted with ownership ignored, where every user sees itself as
/// the owner. The directory stays open, so a later rename of its path cannot redirect
/// operations made through it.
#[derive(Debug)]
pub struct PrivateDir {
    path: PathBuf,
    dir: OwnedFd,
    identity: DirIdentity,
    walk: Walk,
}

impl PrivateDir {
    /// Walks `path` from "/" without following symlinks, judges every ancestor and the final
    /// directory, and keeps the final directory open. `path` must be absolute and normal: no
    /// `.` or `..` components and no empty components. Returns the first rule broken, naming
    /// the component that broke it. A component this user may not open is judged from a
    /// no-follow `fstatat` in its parent; only when it breaks no rule is the result `Io`.
    pub fn open(path: &Path) -> Result<PrivateDir, PrivateSocketError> {
        Self::from_walk(Walk::from_root(path)?)
    }

    #[cfg(test)]
    pub(crate) fn open_from(
        start: OwnedFd,
        start_path: &Path,
        relative: &Path,
    ) -> Result<PrivateDir, PrivateSocketError> {
        Self::from_walk(Walk::from_dir(start, start_path, relative)?)
    }

    fn from_walk(walk: Walk) -> Result<PrivateDir, PrivateSocketError> {
        let path = walk.path();
        let opened = open_private(&walk, &path)?;
        Ok(PrivateDir {
            path,
            dir: opened.fd,
            identity: DirIdentity {
                dev: opened.facts.dev,
                ino: opened.facts.ino,
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
        let facts = open_private(&self.walk, &self.path)?.facts;
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

/// The backlog [`PrivateListener::bind`] passes to `listen`. Once about this many connections
/// wait unaccepted, macOS refuses the next connect with `ECONNREFUSED`, the error a closed
/// listener gives too. Linux makes a blocking connect wait and a nonblocking one fail with
/// `EAGAIN`.
pub const LISTEN_BACKLOG: i32 = 16;

/// A listening socket inside a [`PrivateDir`] that hands out only connections whose kernel peer
/// UID is trusted. Dropping it does what [`PrivateListener::close`] says and discards the
/// outcome.
///
/// It lends its descriptor only as a raw one, for polling:
///
/// ```
/// use plasmosome_core::private_socket::PrivateListener;
/// let polled: fn(&PrivateListener) -> std::os::fd::RawFd = PrivateListener::as_raw_fd;
/// ```
///
/// It does not implement `AsFd`, so safe code cannot copy the descriptor. A copy could accept a
/// peer whose UID is not trusted, make the listener block, or keep the socket listening after
/// [`PrivateListener::close`]:
///
/// ```compile_fail,E0277
/// fn lends<T: std::os::fd::AsFd>() {}
/// lends::<plasmosome_core::private_socket::PrivateListener>();
/// ```
///
/// Stable rustdoc does not check the `E0277`, so that block passes on any compile error and is
/// kept this small.
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
    removed: bool,
}

impl BoundEntry {
    #[allow(unsafe_code)]
    fn remove(&mut self) -> RemoveOutcome {
        self.removed = true;
        let facts = match stat_at(&self.dir.dir, &self.name) {
            Ok(facts) => facts,
            Err(errno) => return gone_or_failed(errno),
        };
        if facts.kind != Kind::Socket || facts.dev != self.entry.dev || facts.ino != self.entry.ino
        {
            return RemoveOutcome::LeftReplacement;
        }
        // SAFETY: `self.name` is a CString this entry owns, and `self.dir` keeps the directory
        // descriptor open for the call. unlinkat keeps no pointer to the name.
        if unsafe { libc::unlinkat(self.dir.dir.as_raw_fd(), self.name.as_ptr(), 0) } == 0 {
            RemoveOutcome::Removed
        } else {
            gone_or_failed(last_errno())
        }
    }
}

impl Drop for BoundEntry {
    fn drop(&mut self) {
        if !self.removed {
            let _ = self.remove();
        }
    }
}

fn gone_or_failed(errno: i32) -> RemoveOutcome {
    if errno == libc::ENOENT {
        RemoveOutcome::AlreadyGone
    } else {
        RemoveOutcome::Failed { errno }
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
        let seam = Seam {
            draw: &mut random_temporary,
            observe: &mut |_| {},
        };
        let (listener, bound) = publish(dir, name, effective_uid(), seam)?;
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

    /// The listening descriptor, for polling. The caller must not close it, accept on it,
    /// change its flags or keep a copy of it.
    pub fn as_raw_fd(&self) -> RawFd {
        self.listener.as_raw_fd()
    }

    /// Closes the socket, then removes the entry it created only while the entry at that name
    /// in the held directory is still the same socket by device and inode, and says what it
    /// found there. Dropping the listener does the same and discards the outcome. A child that
    /// another thread is spawning holds a copy of every descriptor until its exec, so the socket
    /// can take connections until that exec.
    pub fn close(self) -> RemoveOutcome {
        let PrivateListener {
            listener,
            mut bound,
            ..
        } = self;
        drop(listener);
        bound.remove()
    }

    /// Accepts one pending connection without blocking. `Ok(None)`: nothing pending. A peer
    /// whose effective UID is not trusted, or whose credentials cannot be read, is reported as
    /// `Accepted::Refused`. Its stream is closed inside this call, and no code here reads from it.
    /// No shutdown comes before the close. On Linux, a refused peer that sent bytes then fails its
    /// next read with `ECONNRESET`, every time. The reset shows that the stream was closed, not
    /// shut down, while some of the peer's bytes were still queued. It does not show that none of
    /// them was read. A shutdown first would let a read that runs between the shutdown and the
    /// close return end of file. A refused peer that sent nothing reads end of file on Linux. On
    /// macOS a refused peer reads end of file either way. A trusted stream is returned in
    /// blocking mode; the caller sets its timeouts.
    ///
    /// A caller that forks from another thread must hold its descriptor lock around this call,
    /// for two reasons. On macOS the accepted descriptor is marked close-on-exec only after it
    /// exists. On both platforms, a copy of a refused stream's descriptor, such as one held by a
    /// child forked without exec, keeps the refused peer waiting until that copy closes, because
    /// the close here ends only this descriptor. No caller forks without exec today.
    pub fn accept(&self) -> io::Result<Option<Accepted>> {
        let stream = match self.listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        };
        if let Err(refusal) = check_peer_uid(&stream, self.trusted_uid) {
            drop(stream);
            return Ok(Some(Accepted::Refused(refusal)));
        }
        stream.set_nonblocking(false)?;
        Ok(Some(Accepted::Trusted(stream)))
    }
}

/// The result of removing a path that was created, after checking that the path still names
/// the object that was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// The path still named the created object, and it is gone.
    Removed,
    /// Nothing was at the path.
    AlreadyGone,
    /// Another object now occupies the path. It was left in place.
    LeftReplacement,
    /// The removal failed with this errno; the object may remain.
    Failed { errno: i32 },
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
#[allow(unsafe_code)]
fn kernel_peer_uid(fd: RawFd) -> io::Result<u32> {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: `uid` and `gid` are locals that getpeereid may write, and it keeps neither pointer.
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } == 0 {
        Ok(uid)
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn kernel_peer_uid(fd: RawFd) -> io::Result<u32> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `credentials` is a local libc::ucred and `length` holds its exact size, so getsockopt
    // writes at most that many bytes into it.
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

/// What a listed entry is, read from a no-follow stat. A symbolic link is `Symlink` whatever it
/// points to. Sockets, regular files and every other type are `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    Symlink,
    Other,
}

/// One entry of a directory listed by [`list_trusted_dir`]: its name as the bytes on disk, which
/// need not be UTF-8, and its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedEntry {
    pub name: Vec<u8>,
    pub kind: EntryKind,
}

/// Walks `path` from "/" without following symlinks, judges every component, the final one
/// included, by the ancestor rule, and lists the final directory. `path` must be absolute and
/// normal, as for [`PrivateDir::open`]. The final directory passes when its ancestors would: it
/// is owned by root or the effective UID and writable by neither group nor other. On macOS it
/// also carries no ACL allow entry that lets anyone replace its entries, and sits on no volume
/// mounted with ownership ignored. It need not be 0700, and an ACL that only reads or denies
/// passes. A symlink anywhere in the path is `SymlinkInPath` and a missing component is
/// `NoDirectory`; every refusal names the component that broke the rule.
///
/// Returns every entry but `.` and `..`, sorted by name bytes. Each kind comes from a no-follow
/// `fstatat` in the opened directory, never from readdir, and no entry is followed. An entry gone
/// before its `fstatat` is left out. Names are raw bytes: the caller applies its own name rules.
/// A failure to read the directory or to stat an entry is `Io`.
pub fn list_trusted_dir(path: &Path) -> Result<Vec<ListedEntry>, PrivateSocketError> {
    let walk = Walk::from_root(path)?;
    let path = walk.path();
    let euid = effective_uid();
    let opened = walk.open_directory(|reached| {
        judge_ancestor(&reached.facts, euid, reached.path).map_err(PrivateSocketError::from)
    })?;
    refuse_replacing_acl(&opened.fd, &path)?;
    list_entries(&opened.fd, &path)
}

fn open_private(walk: &Walk, path: &Path) -> Result<Opened, PrivateSocketError> {
    let euid = effective_uid();
    let opened =
        walk.open_directory(|reached| judge_private(&reached.facts, euid, reached.path))?;
    refuse_acl(&opened.fd, path)?;
    Ok(opened)
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

impl From<WalkRefusal> for PrivateSocketError {
    fn from(refusal: WalkRefusal) -> Self {
        match refusal {
            WalkRefusal::NotNormal { path } => PrivateSocketError::BadPath { path },
            WalkRefusal::Symlink { path } => PrivateSocketError::SymlinkInPath { path },
            WalkRefusal::Missing { path } => PrivateSocketError::NoDirectory { path },
            WalkRefusal::NotDirectory { path } => PrivateSocketError::NotDirectory { path },
            WalkRefusal::ForeignOwner { path, uid } => {
                PrivateSocketError::ForeignOwner { path, uid }
            }
            WalkRefusal::Replaceable { path, mode } => {
                PrivateSocketError::Replaceable { path, mode }
            }
            WalkRefusal::ReplaceableByAcl { path } => PrivateSocketError::ReplaceableByAcl { path },
            WalkRefusal::OwnershipIgnored { path } => PrivateSocketError::OwnershipIgnored { path },
            WalkRefusal::Io { op, path, errno } => PrivateSocketError::Io { op, path, errno },
        }
    }
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

const TEMPORARY_LENGTH: usize = 16;

const TEMPORARY_CHARACTERS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

#[cfg(target_os = "macos")]
const RENAME_OP: &str = "renameatx_np";
#[cfg(target_os = "linux")]
const RENAME_OP: &str = "renameat2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Bound,
    Restricted,
    Listening,
}

struct Seam<'a> {
    draw: &'a mut dyn FnMut(usize) -> String,
    observe: &'a mut dyn FnMut(Step),
}

#[allow(unsafe_code)]
fn publish(
    dir: PrivateDir,
    name: &str,
    owner: u32,
    seam: Seam<'_>,
) -> Result<(UnixListener, BoundEntry), PrivateSocketError> {
    let c_name = entry_name(name)?;
    let path = dir.path.join(name);
    address_for(&path)?;
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
    let temporary = choose_temporary(
        &dir.path,
        name,
        temporary_room(&dir.path),
        seam.draw,
        |at| bind_path(&socket, at),
    )?;
    let c_temporary = entry_name(&temporary)?;
    let temporary = dir.path.join(temporary);
    (seam.observe)(Step::Bound);
    let recheck = dir.reconfirm();
    let created = match stat_at(&dir.dir, &c_temporary) {
        Ok(facts) if facts.kind == Kind::Socket && facts.uid == owner => facts,
        _ => return Err(PrivateSocketError::BindEscaped { path: temporary }),
    };
    let mut bound = BoundEntry {
        dir,
        name: c_temporary,
        entry: SocketEntry {
            path: temporary,
            dev: created.dev,
            ino: created.ino,
        },
        removed: false,
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
    (seam.observe)(Step::Restricted);
    // SAFETY: listen takes no pointer, and `socket` keeps its descriptor open for the call.
    if unsafe { libc::listen(socket.as_raw_fd(), LISTEN_BACKLOG) } != 0 {
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
    (seam.observe)(Step::Listening);
    let held = bound.dir.dir.as_fd();
    match rename_no_replace(held, &bound.name, held, &c_name) {
        Ok(()) => {}
        Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
            return Err(PrivateSocketError::AddressInUse { path });
        }
        Err(error) => return Err(io_failure(RENAME_OP, &bound.entry.path, &error)),
    }
    bound.name = c_name;
    bound.entry.path = path;
    Ok((listener, bound))
}

fn choose_temporary(
    dir: &Path,
    _name: &str,
    room: usize,
    draw: &mut dyn FnMut(usize) -> String,
    mut bind_at: impl FnMut(&Path) -> Result<(), PrivateSocketError>,
) -> Result<String, PrivateSocketError> {
    let candidate = draw(room);
    bind_at(&dir.join(&candidate))?;
    Ok(candidate)
}

#[allow(unsafe_code)]
fn bind_path(socket: &OwnedFd, at: &Path) -> Result<(), PrivateSocketError> {
    let (address, length) = address_for(at)?;
    // SAFETY: address_for filled `address`, and `length` covers only its used part, so bind reads
    // initialized bytes of a live local.
    let outcome = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length,
        )
    };
    if outcome == 0 {
        Ok(())
    } else {
        Err(io_failure("bind", at, &io::Error::last_os_error()))
    }
}

fn temporary_room(dir: &Path) -> usize {
    let usable = std::mem::size_of::<libc::sockaddr_un>()
        - std::mem::offset_of!(libc::sockaddr_un, sun_path)
        - 1;
    usable.saturating_sub(dir.as_os_str().len() + 1)
}

fn random_temporary(room: usize) -> String {
    static DRAWN: AtomicU64 = AtomicU64::new(0);
    let length = room.min(TEMPORARY_LENGTH);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(DRAWN.fetch_add(1, Ordering::Relaxed));
    let mut characters = String::with_capacity(length);
    if length >= 2 {
        characters.push('.');
    }
    while characters.len() < length {
        hasher.write_usize(characters.len());
        let index = (hasher.finish() % TEMPORARY_CHARACTERS.len() as u64) as usize;
        characters.push(char::from(TEMPORARY_CHARACTERS[index]));
    }
    characters
}

#[allow(unsafe_code)]
fn restrict_to_owner(bound: &BoundEntry) -> Result<(), PrivateSocketError> {
    let dir = bound.dir.dir.as_raw_fd();
    let name = bound.name.as_ptr();
    // SAFETY: `name` points into `bound.name`, a CString that outlives this function, and
    // `bound.dir` keeps `dir` open. fchmodat keeps no pointer.
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
    // SAFETY: `name` points into `bound.name`, a CString that outlives this function, and
    // `bound.dir` keeps `dir` open. fchmodat keeps no pointer.
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

fn entry_name(name: &str) -> Result<CString, PrivateSocketError> {
    let bad = || PrivateSocketError::BadName {
        name: name.to_string(),
    };
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(bad());
    }
    CString::new(name).map_err(|_| bad())
}

#[allow(unsafe_code)]
fn address_for(path: &Path) -> Result<(libc::sockaddr_un, libc::socklen_t), PrivateSocketError> {
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: sockaddr_un is a plain C struct, for which all zeroes is a valid value.
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

fn list_entries(dir: &OwnedFd, path: &Path) -> Result<Vec<ListedEntry>, PrivateSocketError> {
    let names = DirectoryStream::open(dir.as_fd())
        .map_err(|error| io_failure("fdopendir", path, &error))?;
    listed_from(dir, path, names)
}

fn listed_from(
    dir: &OwnedFd,
    path: &Path,
    names: impl Iterator<Item = io::Result<Vec<u8>>>,
) -> Result<Vec<ListedEntry>, PrivateSocketError> {
    let mut listed = Vec::new();
    for name in names {
        let name = name.map_err(|error| io_failure("readdir", path, &error))?;
        let name = CString::new(name).map_err(|error| PrivateSocketError::BadName {
            name: String::from_utf8_lossy(&error.into_vec()).into_owned(),
        })?;
        let kind = match stat_at(dir, &name) {
            Ok(facts) => match facts.kind {
                Kind::Directory => EntryKind::Directory,
                Kind::Symlink => EntryKind::Symlink,
                Kind::Regular | Kind::Socket | Kind::Other => EntryKind::Other,
            },
            Err(errno) if errno == libc::ENOENT => continue,
            Err(errno) => {
                return Err(PrivateSocketError::Io {
                    op: "fstatat",
                    path: path.join(OsStr::from_bytes(name.as_bytes())),
                    errno,
                });
            }
        };
        listed.push(ListedEntry {
            name: name.into_bytes(),
            kind,
        });
    }
    listed.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    Ok(listed)
}

#[allow(unsafe_code)]
fn unix_socket() -> io::Result<OwnedFd> {
    #[cfg(target_os = "linux")]
    // SAFETY: socket takes no pointer.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    #[cfg(target_os = "macos")]
    // SAFETY: socket takes no pointer.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: socket returned a non-negative descriptor just above, which nothing else owns.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    #[cfg(target_os = "macos")]
    // SAFETY: fcntl with F_SETFD takes an integer flag, not a pointer, and `socket` keeps its
    // descriptor open.
    if unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(socket)
}

fn refuse_acl(dir: &OwnedFd, path: &Path) -> Result<(), PrivateSocketError> {
    refuse_if(acl_present(dir), path, |path| {
        PrivateSocketError::AclPresent { path }
    })
}

#[cfg(target_os = "macos")]
fn acl_present(dir: &OwnedFd) -> Result<bool, (&'static str, i32)> {
    darwin_acl::has_entries(dir.as_fd())
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn acl_present(dir: &OwnedFd) -> Result<bool, (&'static str, i32)> {
    for attribute in [c"system.posix_acl_access", c"system.posix_acl_default"] {
        // SAFETY: `attribute` is a static NUL-terminated name, and `dir` keeps its descriptor open.
        // A null buffer of size 0 asks only for the size, so nothing is written.
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

    const DRAWN: &str = ".0123456789abcde";

    fn publish_drawing(
        dir: PrivateDir,
        name: &str,
        owner: u32,
        temporaries: &[&str],
        mut observe: impl FnMut(Step),
    ) -> Result<(UnixListener, BoundEntry), PrivateSocketError> {
        let mut drawn = temporaries.iter();
        let mut draw = |_room: usize| match drawn.next() {
            Some(temporary) => (*temporary).to_string(),
            None => panic!("a draw past the {} temporaries given", temporaries.len()),
        };
        let seam = Seam {
            draw: &mut draw,
            observe: &mut observe,
        };
        publish(dir, name, owner, seam)
    }

    fn refusal_of(
        outcome: Result<(UnixListener, BoundEntry), PrivateSocketError>,
    ) -> PrivateSocketError {
        match outcome {
            Ok((_, bound)) => panic!("publish bound {:?}", bound.entry),
            Err(refusal) => refusal,
        }
    }

    fn entries(path: &Path) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(path)
            .expect("readdir")
            .map(|entry| entry.expect("an entry").path())
            .collect();
        found.sort();
        found
    }

    fn private_cell() -> (TestRoot, PathBuf, PrivateDir) {
        let root = test_root();
        let cell = root.path().join("cell");
        make_dir(&cell, 0o700);
        let dir = open_in(&root, "cell").expect("a private directory opens");
        (root, cell, dir)
    }

    #[derive(Debug, PartialEq, Eq)]
    enum View {
        Absent,
        Refused(PrivateSocketError),
        Unreachable,
        Ready,
        Unexpected(Option<i32>),
    }

    fn view_of(root: &TestRoot, cell: &str, name: &str) -> View {
        let checked = open_in(root, cell).and_then(|dir| dir.socket_entry(name, effective_uid()));
        let connected = UnixStream::connect(root.path().join(cell).join(name))
            .map(drop)
            .map_err(|error| error.raw_os_error());
        match (checked, connected) {
            (Err(PrivateSocketError::NoSocket { .. }), Err(Some(libc::ENOENT))) => View::Absent,
            (Err(refusal), _) => View::Refused(refusal),
            (Ok(_), Ok(())) => View::Ready,
            (Ok(_), Err(Some(libc::ECONNREFUSED))) => View::Unreachable,
            (Ok(_), Err(errno)) => View::Unexpected(errno),
        }
    }

    fn listening(listener: UnixListener, bound: BoundEntry) -> PrivateListener {
        PrivateListener {
            listener,
            bound,
            trusted_uid: effective_uid(),
        }
    }

    fn accept_pending(listener: &PrivateListener) -> Accepted {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(accepted) = listener.accept().expect("accept") {
                return accepted;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no connection arrived"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn a_client_finds_no_entry_at_the_name_until_the_socket_is_private_and_listening() {
        let (root, _, dir) = private_cell();
        let mut views = Vec::new();
        let (listener, bound) = publish_drawing(dir, "sock", effective_uid(), &[DRAWN], |step| {
            views.push((step, view_of(&root, "cell", "sock")));
        })
        .expect("publish binds");
        assert_eq!(
            views,
            vec![
                (Step::Bound, View::Absent),
                (Step::Restricted, View::Absent),
                (Step::Listening, View::Absent),
            ]
        );
        let listener = listening(listener, bound);
        assert_eq!(view_of(&root, "cell", "sock"), View::Ready);
        assert!(
            matches!(accept_pending(&listener), Accepted::Trusted(_)),
            "the client that found the socket is trusted"
        );
    }

    #[test]
    fn the_temporary_is_private_and_listening_before_it_is_renamed() {
        let (_root, cell, dir) = private_cell();
        let temporary = cell.join(DRAWN);
        let mut seen = None;
        let mut client = None;
        let (listener, bound) = publish_drawing(dir, "sock", effective_uid(), &[DRAWN], |step| {
            if step == Step::Listening {
                seen = Some(fs::symlink_metadata(&temporary).expect("the temporary exists"));
                client = Some(UnixStream::connect(&temporary).expect("the temporary accepts"));
            }
        })
        .expect("publish binds");
        let seen = seen.expect("the last observation point ran");
        assert!(seen.file_type().is_socket());
        assert_eq!(seen.uid(), effective_uid());
        assert_eq!(seen.mode() & 0o7777, 0o600);
        let published = fs::symlink_metadata(cell.join("sock")).expect("the socket has its name");
        assert_eq!((published.dev(), published.ino()), (seen.dev(), seen.ino()));
        assert_eq!(
            bound.entry,
            SocketEntry {
                path: cell.join("sock"),
                dev: seen.dev(),
                ino: seen.ino()
            }
        );
        assert_eq!(entries(&cell), vec![cell.join("sock")]);
        let listener = listening(listener, bound);
        assert!(
            matches!(accept_pending(&listener), Accepted::Trusted(_)),
            "the connection made at the temporary is accepted"
        );
        drop(client);
    }

    #[test]
    fn an_entry_that_appears_at_the_name_before_the_rename_is_refused_and_left() {
        for kind in ["file", "dangling symlink", "socket"] {
            let (_root, cell, dir) = private_cell();
            let sock = cell.join("sock");
            let mut other = None;
            let mut planted = None;
            let refusal = refusal_of(publish_drawing(
                dir,
                "sock",
                effective_uid(),
                &[DRAWN],
                |step| {
                    if step != Step::Listening {
                        return;
                    }
                    match kind {
                        "file" => fs::write(&sock, b"planted").expect("write"),
                        "dangling symlink" => symlink(cell.join("absent"), &sock).expect("symlink"),
                        _ => other = Some(UnixListener::bind(&sock).expect("bind another socket")),
                    }
                    planted = Some(fs::symlink_metadata(&sock).expect("the planted entry"));
                },
            ));
            assert_eq!(
                refusal,
                PrivateSocketError::AddressInUse { path: sock.clone() },
                "{kind}"
            );
            let before = planted.expect("the last observation point ran");
            let after = fs::symlink_metadata(&sock).expect("the planted entry stays");
            assert_eq!(
                (after.dev(), after.ino(), after.file_type()),
                (before.dev(), before.ino(), before.file_type()),
                "{kind}"
            );
            match kind {
                "file" => assert_eq!(fs::read(&sock).expect("read"), b"planted"),
                "dangling symlink" => {
                    assert_eq!(fs::read_link(&sock).expect("readlink"), cell.join("absent"))
                }
                _ => assert!(after.file_type().is_socket()),
            }
            assert_eq!(
                entries(&cell),
                vec![sock.clone()],
                "{kind}: the temporary is gone"
            );
            drop(other);
        }
    }

    #[test]
    fn a_failure_before_the_rename_removes_only_the_temporary_it_created() {
        let (_root, cell, dir) = private_cell();
        let sock = cell.join("sock");
        let refusal = refusal_of(publish_drawing(
            dir,
            "sock",
            effective_uid(),
            &[DRAWN],
            |step| {
                if step == Step::Bound {
                    fs::write(&sock, b"planted").expect("write");
                    set_mode(&cell, 0o750);
                }
            },
        ));
        assert_eq!(
            refusal,
            PrivateSocketError::NotPrivate {
                path: cell.clone(),
                mode: 0o750
            }
        );
        assert_eq!(fs::read(&sock).expect("the planted file stays"), b"planted");
        assert_eq!(entries(&cell), vec![sock]);
    }

    #[test]
    fn publish_refuses_a_directory_that_stopped_being_private_without_binding() {
        let (_root, cell, dir) = private_cell();
        set_mode(&cell, 0o750);
        let mut steps = Vec::new();
        let refusal = refusal_of(publish_drawing(dir, "sock", effective_uid(), &[], |step| {
            steps.push(step)
        }));
        assert_eq!(
            refusal,
            PrivateSocketError::NotPrivate {
                path: cell.clone(),
                mode: 0o750
            }
        );
        assert_eq!(
            steps,
            Vec::<Step>::new(),
            "the socket was bound in a directory no longer private"
        );
        assert_eq!(entries(&cell), Vec::<PathBuf>::new());
    }

    #[test]
    fn publish_reports_the_recheck_and_removes_a_temporary_bound_before_the_directory_changed() {
        let (_root, cell, dir) = private_cell();
        let mut bound_at = Vec::new();
        let refusal = refusal_of(publish_drawing(
            dir,
            "sock",
            effective_uid(),
            &[DRAWN],
            |step| {
                if step == Step::Bound {
                    bound_at = entries(&cell);
                    set_mode(&cell, 0o750);
                }
            },
        ));
        assert_eq!(
            refusal,
            PrivateSocketError::NotPrivate {
                path: cell.clone(),
                mode: 0o750
            }
        );
        assert_eq!(bound_at, vec![cell.join(DRAWN)]);
        assert_eq!(entries(&cell), Vec::<PathBuf>::new());
    }

    #[test]
    fn publish_reports_a_directory_replaced_after_bind_and_removes_the_temporary() {
        let (root, cell, dir) = private_cell();
        let moved = root.path().join("moved");
        let mut bound_at = Vec::new();
        let refusal = refusal_of(publish_drawing(
            dir,
            "sock",
            effective_uid(),
            &[DRAWN],
            |step| {
                if step == Step::Bound {
                    bound_at = entries(&cell);
                    fs::rename(&cell, &moved).expect("rename");
                    make_dir(&cell, 0o700);
                }
            },
        ));
        assert_eq!(refusal, PrivateSocketError::Replaced { path: cell.clone() });
        assert_eq!(bound_at, vec![cell.join(DRAWN)]);
        assert_eq!(entries(&moved), Vec::<PathBuf>::new());
        assert_eq!(entries(&cell), Vec::<PathBuf>::new());
    }

    #[test]
    fn publish_reports_bind_escaped_and_leaves_the_temporary_when_no_socket_is_there() {
        let (root, cell, dir) = private_cell();
        let temporary = cell.join(DRAWN);
        let refusal = refusal_of(publish_drawing(
            dir,
            "sock",
            effective_uid(),
            &[DRAWN],
            |step| {
                if step == Step::Bound {
                    fs::remove_file(&temporary).expect("unlink");
                }
            },
        ));
        assert_eq!(
            refusal,
            PrivateSocketError::BindEscaped {
                path: temporary.clone()
            }
        );
        assert_eq!(entries(&cell), Vec::<PathBuf>::new());

        let dir = open_in(&root, "cell").expect("a private directory opens");
        let refusal = refusal_of(publish_drawing(
            dir,
            "sock",
            effective_uid(),
            &[DRAWN],
            |step| {
                if step == Step::Bound {
                    fs::remove_file(&temporary).expect("unlink");
                    fs::write(&temporary, b"planted").expect("write");
                }
            },
        ));
        assert_eq!(
            refusal,
            PrivateSocketError::BindEscaped {
                path: temporary.clone()
            }
        );
        assert_eq!(
            fs::read(&temporary).expect("the planted file stays"),
            b"planted"
        );
        assert_eq!(entries(&cell), vec![temporary]);
    }

    #[test]
    fn publish_leaves_a_socket_it_did_not_bind_when_the_recheck_fails() {
        let (_root, cell, dir) = private_cell();
        let temporary = cell.join(DRAWN);
        let mut other = None;
        let stand_in = effective_uid().wrapping_add(1);
        let refusal = refusal_of(publish_drawing(dir, "sock", stand_in, &[DRAWN], |step| {
            if step == Step::Bound {
                fs::remove_file(&temporary).expect("unlink");
                other = Some(UnixListener::bind(&temporary).expect("bind another socket"));
                set_mode(&cell, 0o770);
            }
        }));
        let left =
            fs::symlink_metadata(&temporary).expect("the socket this call did not bind stays");
        assert!(left.file_type().is_socket());
        assert_eq!(
            refusal,
            PrivateSocketError::BindEscaped {
                path: temporary.clone()
            }
        );
        assert_eq!(entries(&cell), vec![temporary]);
        drop(other);
    }

    #[test]
    fn a_taken_temporary_or_one_matching_the_name_ignoring_case_is_drawn_again() {
        let (_root, cell, dir) = private_cell();
        let taken = cell.join(".aaaa");
        fs::write(&taken, b"taken").expect("write");
        let taken_before = fs::symlink_metadata(&taken).expect("stat");
        let mut bound_at = Vec::new();
        let (_listener, bound) = publish_drawing(
            dir,
            ".SOCK",
            effective_uid(),
            &[".aaaa", ".sock", ".good"],
            |step| {
                if step == Step::Bound {
                    bound_at = entries(&cell);
                }
            },
        )
        .expect("publish binds");
        assert_eq!(bound_at, vec![taken.clone(), cell.join(".good")]);
        assert_eq!(bound.entry.path, cell.join(".SOCK"));
        assert_eq!(entries(&cell), vec![cell.join(".SOCK"), taken.clone()]);
        let taken_after = fs::symlink_metadata(&taken).expect("the taken entry stays");
        assert_eq!(
            (taken_after.dev(), taken_after.ino()),
            (taken_before.dev(), taken_before.ino())
        );
        assert_eq!(fs::read(&taken).expect("read"), b"taken");
    }

    #[test]
    fn eight_taken_temporaries_give_up_with_the_last_one_named() {
        let (_root, cell, dir) = private_cell();
        let names = [".t1", ".t2", ".t3", ".t4", ".t5", ".t6", ".t7", ".t8"];
        let taken: Vec<PathBuf> = names.iter().map(|name| cell.join(name)).collect();
        for path in &taken {
            fs::write(path, b"taken").expect("write");
        }
        let mut steps = Vec::new();
        let refusal = refusal_of(publish_drawing(
            dir,
            "sock",
            effective_uid(),
            &names,
            |step| steps.push(step),
        ));
        assert_eq!(
            refusal,
            PrivateSocketError::Io {
                op: "bind",
                path: cell.join(".t8"),
                errno: libc::EADDRINUSE
            }
        );
        assert_eq!(steps, Vec::<Step>::new());
        assert_eq!(entries(&cell), taken);
        for path in &taken {
            assert_eq!(fs::read(path).expect("read"), b"taken");
        }
    }

    #[test]
    fn a_bind_the_directory_refuses_is_io_naming_the_temporary_and_is_not_drawn_again() {
        let (_root, cell, dir) = private_cell();
        let mut draws = 0;
        let mut draw = |_room: usize| {
            draws += 1;
            assert_eq!(draws, 1, "a refused bind drew a second temporary");
            set_mode(&cell, 0o500);
            DRAWN.to_string()
        };
        let seam = Seam {
            draw: &mut draw,
            observe: &mut |_| {},
        };
        let outcome = publish(dir, "sock", effective_uid(), seam);
        set_mode(&cell, 0o700);
        if effective_uid() == 0 {
            assert!(outcome.is_ok(), "root ignores the mode");
            return;
        }
        assert_eq!(
            refusal_of(outcome),
            PrivateSocketError::Io {
                op: "bind",
                path: cell.join(DRAWN),
                errno: libc::EACCES
            }
        );
        assert_eq!(entries(&cell), Vec::<PathBuf>::new());
    }

    #[test]
    fn the_temporary_fits_the_room_left_and_has_one_shape() {
        for (name, room) in [
            ("a", 1),
            ("sock", 2),
            ("sock", 15),
            ("sock", 16),
            ("sock", 90),
        ] {
            for _ in 0..1000 {
                let temporary = choose_temporary(
                    Path::new("/r/cell"),
                    name,
                    room,
                    &mut random_temporary,
                    |_| Ok(()),
                )
                .expect("a free temporary");
                assert_eq!(
                    temporary.len(),
                    room.min(16),
                    "{temporary:?} at room {room}"
                );
                let rest = if room >= 2 {
                    assert!(temporary.starts_with('.'), "{temporary:?} at room {room}");
                    &temporary[1..]
                } else {
                    &temporary[..]
                };
                assert!(
                    rest.bytes()
                        .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase()),
                    "{temporary:?} at room {room}"
                );
                assert!(
                    !temporary.eq_ignore_ascii_case(name),
                    "{temporary:?} names the entry {name:?}"
                );
            }
        }
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
    #[allow(unsafe_code)]
    fn check_peer_uid_refuses_a_socket_without_peer_credentials() {
        // SAFETY: socket takes no pointer.
        let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        assert!(raw >= 0, "socket: {}", std::io::Error::last_os_error());
        // SAFETY: The assert above shows `raw` is a non-negative descriptor, and nothing else owns
        // it.
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

    fn listed(name: &str, kind: EntryKind) -> ListedEntry {
        ListedEntry {
            name: name.as_bytes().to_vec(),
            kind,
        }
    }

    fn names(given: &[&str]) -> std::vec::IntoIter<io::Result<Vec<u8>>> {
        given
            .iter()
            .map(|name| Ok(name.as_bytes().to_vec()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn names_with_no_type_get_their_kind_from_a_no_follow_stat_and_are_sorted() {
        let root = test_root();
        let at = |name: &str| root.path().join(name);
        make_dir(&at("dir"), 0o700);
        fs::write(at("file"), b"").expect("write");
        symlink("dir", at("link")).expect("symlink");
        let _socket = UnixListener::bind(at("sock")).expect("bind a socket");
        assert_eq!(
            listed_from(
                &start(&root),
                root.path(),
                names(&["sock", "link", "file", "dir"])
            ),
            Ok(vec![
                listed("dir", EntryKind::Directory),
                listed("file", EntryKind::Other),
                listed("link", EntryKind::Symlink),
                listed("sock", EntryKind::Other),
            ])
        );
    }

    #[test]
    fn a_name_gone_before_its_stat_is_left_out() {
        let root = test_root();
        make_dir(&root.path().join("kept"), 0o700);
        assert_eq!(
            listed_from(&start(&root), root.path(), names(&["gone", "kept"])),
            Ok(vec![listed("kept", EntryKind::Directory)])
        );
    }

    #[test]
    fn a_readdir_error_after_some_names_is_io_naming_the_directory() {
        let root = test_root();
        make_dir(&root.path().join("kept"), 0o700);
        let failing = vec![
            Ok(b"kept".to_vec()),
            Err(io::Error::from_raw_os_error(libc::EIO)),
        ];
        assert_eq!(
            listed_from(&start(&root), root.path(), failing.into_iter()),
            Err(PrivateSocketError::Io {
                op: "readdir",
                path: root.path().to_path_buf(),
                errno: libc::EIO
            })
        );
    }

    #[test]
    fn a_name_holding_a_nul_byte_is_a_bad_name() {
        let root = test_root();
        let name = vec![b'a', 0, b'b'];
        let spelled = String::from_utf8(name.clone()).expect("ASCII");
        assert_eq!(
            listed_from(&start(&root), root.path(), vec![Ok(name)].into_iter()),
            Err(PrivateSocketError::BadName { name: spelled })
        );
    }

    #[test]
    fn listing_reads_the_held_directory_not_the_one_its_path_now_names() {
        let root = test_root();
        let held = root.path().join("held");
        make_dir(&held, 0o700);
        make_dir(&held.join("x"), 0o700);
        let named = root.path().join("named");
        make_dir(&named, 0o700);
        fs::write(named.join("y"), b"").expect("write");
        let dir = OwnedFd::from(fs::File::open(&held).expect("open"));
        assert_eq!(
            list_entries(&dir, &named),
            Ok(vec![listed("x", EntryKind::Directory)])
        );
    }

    #[test]
    fn listing_a_descriptor_that_is_not_a_directory_is_io() {
        let root = test_root();
        let file = root.path().join("file");
        fs::write(&file, b"").expect("write");
        let held = OwnedFd::from(fs::File::open(&file).expect("open"));
        assert_eq!(
            list_entries(&held, &file),
            Err(PrivateSocketError::Io {
                op: "fdopendir",
                path: file,
                errno: libc::ENOTDIR
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
