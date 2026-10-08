use std::ffi::CString;
use std::fmt;
use std::os::fd::{OwnedFd, RawFd};
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
/// the directory itself is owned by the effective UID, has no group or other bits and carries
/// no ACL. The directory stays open, so a later rename of its path cannot redirect operations
/// made through it.
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
    judge_start: bool,
}

impl PrivateDir {
    /// Walks `path` from "/" without following symlinks, judges every ancestor and the final
    /// directory, and keeps the final directory open. `path` must be absolute and normal: no
    /// `.` or `..` components and no empty components. Returns the first rule broken, naming
    /// the component that broke it.
    pub fn open(path: &Path) -> Result<PrivateDir, PrivateSocketError> {
        let _ = path;
        todo!()
    }

    pub(crate) fn open_from(
        start: OwnedFd,
        start_path: &Path,
        relative: &Path,
    ) -> Result<PrivateDir, PrivateSocketError> {
        let _ = (start, start_path, relative);
        todo!()
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
        todo!()
    }

    /// Inspects `name` inside this directory without following it, and requires a socket owned
    /// by `trusted_uid` with permission bits exactly 0o600. A symlink at `name` is refused as
    /// `NotASocket`, never followed.
    pub fn socket_entry(
        &self,
        name: &str,
        trusted_uid: u32,
    ) -> Result<SocketEntry, PrivateSocketError> {
        let _ = (name, trusted_uid);
        todo!()
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

impl PrivateListener {
    /// Binds `name` inside `dir` for peers whose effective UID is `trusted_uid`. Production
    /// passes `effective_uid()`; a test passes another value to force a real mismatch. `name` is
    /// one path component. Any existing entry at the name is refused and left untouched. The
    /// socket is owned by the effective UID and set to mode 0600 before it listens; umask plays
    /// no part. The listener is nonblocking. A caller that forks from another thread must hold
    /// its descriptor lock around this call, because on macOS the socket is created before it is
    /// marked close-on-exec.
    pub fn bind(
        dir: PrivateDir,
        name: &str,
        trusted_uid: u32,
    ) -> Result<PrivateListener, PrivateSocketError> {
        let _ = (dir, name, trusted_uid);
        todo!()
    }

    /// The socket entry this listener created.
    pub fn entry(&self) -> &SocketEntry {
        &self.bound.entry
    }

    /// The listening descriptor, for polling. The caller must not close it or accept on it.
    pub fn as_raw_fd(&self) -> RawFd {
        todo!()
    }

    /// Accepts one pending connection without blocking. `Ok(None)`: nothing pending. A peer
    /// whose effective UID is not trusted, or whose credentials cannot be read, is shut down and
    /// closed before any byte is read and reported as `Accepted::Refused`. A trusted stream is
    /// returned in blocking mode; the caller sets its timeouts. A caller that forks from another
    /// thread must hold its descriptor lock around this call, because on macOS the accepted
    /// descriptor is marked close-on-exec only after it exists.
    pub fn accept(&self) -> std::io::Result<Option<Accepted>> {
        todo!()
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
pub fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    let _ = stream;
    todo!()
}

/// Requires the kernel peer UID of `stream` to equal `expected`. Call it before sending or
/// reading any byte; on refusal, close the stream unread.
pub fn check_peer_uid(stream: &UnixStream, expected: u32) -> Result<(), PrivateSocketError> {
    let _ = (stream, expected);
    todo!()
}

/// The client half of the path boundary, for a caller-owned nonblocking connect: opens the
/// parent of `socket_path` as a [`PrivateDir`], then checks its file name with
/// [`PrivateDir::socket_entry`]. Returns the checked entry. The caller connects afterwards and
/// must still call [`check_peer_uid`] on the connected stream.
pub fn check_private_path(
    socket_path: &Path,
    trusted_uid: u32,
) -> Result<SocketEntry, PrivateSocketError> {
    let _ = (socket_path, trusted_uid);
    todo!()
}

/// The effective UID of this process.
pub fn effective_uid() -> u32 {
    todo!()
}

/// Why a private socket path, socket or peer was refused. `Display` names the path and the
/// rule broken; callers branch on the variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivateSocketError {
    NotAbsolute { path: PathBuf },
    SymlinkInPath { at: PathBuf },
    Missing { at: PathBuf },
    NotDirectory { at: PathBuf },
    ForeignOwner { at: PathBuf, uid: u32 },
    Replaceable { at: PathBuf, mode: u32 },
    NotPrivate { path: PathBuf, mode: u32 },
    AclPresent { path: PathBuf },
    Replaced { path: PathBuf },
    BadName { name: String },
    PathTooLong { path: PathBuf, max: usize },
    AddressInUse { path: PathBuf },
    NotASocket { path: PathBuf },
    SocketOwner { path: PathBuf, uid: u32 },
    SocketMode { path: PathBuf, mode: u32 },
    BindEscaped { path: PathBuf },
    PeerCredentials { errno: i32 },
    PeerMismatch { trusted: u32, found: u32 },
    Io { op: &'static str, at: PathBuf, errno: i32 },
}

impl fmt::Display for PrivateSocketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
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

fn judge_ancestor(facts: &Facts, euid: u32, at: &Path) -> Result<(), PrivateSocketError> {
    let _ = (facts, euid, at);
    todo!()
}

fn judge_private(facts: &Facts, euid: u32, path: &Path) -> Result<(), PrivateSocketError> {
    let _ = (facts, euid, path);
    todo!()
}

fn judge_socket(facts: &Facts, trusted_uid: u32, path: &Path) -> Result<(), PrivateSocketError> {
    let _ = (facts, trusted_uid, path);
    todo!()
}

fn judge_peer(trusted: u32, found: u32) -> Result<(), PrivateSocketError> {
    let _ = (trusted, found);
    todo!()
}

fn prepare(dir: PrivateDir, name: &str) -> Result<(OwnedFd, BoundEntry), PrivateSocketError> {
    let _ = (dir, name);
    todo!()
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

    fn start(root: &TempDir) -> OwnedFd {
        OwnedFd::from(fs::File::open(root.path()).expect("the test root opens"))
    }

    fn make_dir(path: &Path, mode: u32) {
        fs::create_dir(path).expect("mkdir");
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
    }

    fn open_in(root: &TempDir, relative: &str) -> Result<PrivateDir, PrivateSocketError> {
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
                    at: at.to_path_buf(),
                    mode
                })
            );
        }
        assert_eq!(
            judge_ancestor(&facts(Kind::Directory, 4242, 0o755), EUID, at),
            Err(PrivateSocketError::ForeignOwner {
                at: at.to_path_buf(),
                uid: 4242
            })
        );
        assert_eq!(
            judge_ancestor(&facts(Kind::Other, 0, 0o755), EUID, at),
            Err(PrivateSocketError::NotDirectory {
                at: at.to_path_buf()
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
                at: path.to_path_buf(),
                uid: 0
            })
        );
        assert_eq!(
            judge_private(&facts(Kind::Symlink, EUID, 0o700), EUID, path),
            Err(PrivateSocketError::NotDirectory {
                at: path.to_path_buf()
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
        let root = tempfile::tempdir().expect("tempdir");
        make_dir(&root.path().join("a"), 0o700);
        make_dir(&root.path().join("b"), 0o700);
        make_dir(&root.path().join("b/cell"), 0o700);
        symlink("../b", root.path().join("a/link")).expect("symlink");
        symlink("../b/cell", root.path().join("a/last")).expect("symlink");
        assert_eq!(
            open_in(&root, "a/link/cell").unwrap_err(),
            PrivateSocketError::SymlinkInPath {
                at: root.path().join("a/link")
            }
        );
        assert_eq!(
            open_in(&root, "a/last").unwrap_err(),
            PrivateSocketError::SymlinkInPath {
                at: root.path().join("a/last")
            }
        );
    }

    #[test]
    fn open_from_refuses_a_group_writable_intermediate() {
        let root = tempfile::tempdir().expect("tempdir");
        make_dir(&root.path().join("a"), 0o775);
        make_dir(&root.path().join("a/cell"), 0o700);
        assert_eq!(
            open_in(&root, "a/cell").unwrap_err(),
            PrivateSocketError::Replaceable {
                at: root.path().join("a"),
                mode: 0o775
            }
        );
    }

    #[test]
    fn open_from_refuses_a_0750_final_directory() {
        let root = tempfile::tempdir().expect("tempdir");
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
        let root = tempfile::tempdir().expect("tempdir");
        fs::write(root.path().join("file"), b"").expect("write");
        assert_eq!(
            open_in(&root, "absent/cell").unwrap_err(),
            PrivateSocketError::Missing {
                at: root.path().join("absent")
            }
        );
        assert_eq!(
            open_in(&root, "file/cell").unwrap_err(),
            PrivateSocketError::NotDirectory {
                at: root.path().join("file")
            }
        );
        assert_eq!(
            open_in(&root, "file").unwrap_err(),
            PrivateSocketError::NotDirectory {
                at: root.path().join("file")
            }
        );
    }

    #[test]
    fn open_from_accepts_a_0700_final_directory_and_reconfirm_passes() {
        let root = tempfile::tempdir().expect("tempdir");
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
    fn reconfirm_fails_after_the_directory_is_replaced() {
        let root = tempfile::tempdir().expect("tempdir");
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
        let root = tempfile::tempdir().expect("tempdir");
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
                PrivateSocketError::NotAbsolute {
                    path: PathBuf::from(path)
                }
            );
        }
    }

    #[test]
    fn prepare_leaves_a_0600_socket_that_refuses_connections() {
        let root = tempfile::tempdir().expect("tempdir");
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
                PrivateSocketError::NotAbsolute { path: at() },
                "/r/cell is not an absolute path of normal components (no empty, . or .. components)",
            ),
            (
                PrivateSocketError::SymlinkInPath { at: at() },
                "/r/cell is a symlink; a private socket path must not pass through one",
            ),
            (
                PrivateSocketError::Missing { at: at() },
                "/r/cell does not exist",
            ),
            (
                PrivateSocketError::NotDirectory { at: at() },
                "/r/cell is not a directory",
            ),
            (
                PrivateSocketError::ForeignOwner { at: at(), uid: 42 },
                "/r/cell is owned by uid 42, which is not trusted here",
            ),
            (
                PrivateSocketError::Replaceable {
                    at: at(),
                    mode: 0o1777,
                },
                "/r/cell has mode 1777: group or other can replace entries in it",
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
                PrivateSocketError::SocketOwner { path: at(), uid: 42 },
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
                    at: at(),
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
