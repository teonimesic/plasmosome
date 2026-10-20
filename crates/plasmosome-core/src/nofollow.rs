use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::private_socket::{FinalRule, PrivateSocketError, effective_uid, io_failure, last_errno};

#[derive(Debug)]
pub(crate) struct Walk {
    pub(crate) start: OwnedFd,
    pub(crate) start_path: PathBuf,
    pub(crate) components: Vec<CString>,
}

impl Walk {
    pub(crate) fn from_root(path: &Path) -> Result<Walk, PrivateSocketError> {
        let components = absolute_components(path)?;
        let root = Path::new("/");
        let start = open_root().map_err(|error| io_failure("open", root, &error))?;
        Ok(Walk {
            start,
            start_path: root.to_path_buf(),
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

    fn run(&self, euid: u32, rule: FinalRule) -> Result<(OwnedFd, Facts), PrivateSocketError> {
        let mut at = self.start_path.clone();
        let mut facts = stat_fd(&self.start, &at)?;
        let mut held: Option<OwnedFd> = None;
        for (index, component) in self.components.iter().enumerate() {
            let parent = held.as_ref().unwrap_or(&self.start);
            refuse_ignored_ownership(parent, &at)?;
            judge_ancestor(&facts, euid, &at)?;
            refuse_replacing_acl(parent, &at)?;
            at.push(OsStr::from_bytes(component.as_bytes()));
            let last = index + 1 == self.components.len();
            let access = if last { libc::O_RDONLY } else { SEARCH_ONLY };
            let next = open_child(parent, component, &at, access, |facts, at| {
                if last {
                    rule.judge(facts, euid, at)
                } else {
                    judge_ancestor(facts, euid, at)
                }
            })?;
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

    pub(crate) fn judged(
        &self,
        path: &Path,
        rule: FinalRule,
    ) -> Result<(OwnedFd, Facts), PrivateSocketError> {
        let euid = effective_uid();
        let (dir, facts) = self.run(euid, rule)?;
        refuse_ignored_ownership(&dir, path)?;
        rule.judge(&facts, euid, path)?;
        rule.refuse_final_acl(&dir, path)?;
        Ok((dir, facts))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Directory,
    Socket,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Facts {
    pub(crate) kind: Kind,
    pub(crate) uid: u32,
    pub(crate) mode: u32,
    pub(crate) dev: u64,
    pub(crate) ino: u64,
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

pub(crate) fn judge_ancestor(
    facts: &Facts,
    euid: u32,
    at: &Path,
) -> Result<(), PrivateSocketError> {
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

pub(crate) fn absolute_components(path: &Path) -> Result<Vec<CString>, PrivateSocketError> {
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

pub(crate) fn components_of(
    relative: &[u8],
    whole: &Path,
) -> Result<Vec<CString>, PrivateSocketError> {
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
fn open_child(
    parent: &OwnedFd,
    name: &CStr,
    at: &Path,
    access: libc::c_int,
    judge: impl FnOnce(&Facts, &Path) -> Result<(), PrivateSocketError>,
) -> Result<OwnedFd, PrivateSocketError> {
    // SAFETY: `name` is a CStr that outlives the call, and `parent` keeps its descriptor open for
    // it. openat keeps no pointer to the name.
    let raw = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            access | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw >= 0 {
        // SAFETY: openat returned a non-negative descriptor just above, which nothing else owns.
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
        libc::EACCES => match stat_at(parent, name).map(|facts| judge(&facts, &at)) {
            Ok(Err(refusal)) => refusal,
            _ => PrivateSocketError::Io {
                op: "openat",
                path: at,
                errno,
            },
        },
        errno => PrivateSocketError::Io {
            op: "openat",
            path: at,
            errno,
        },
    })
}

#[allow(unsafe_code)]
fn stat_fd(fd: &OwnedFd, at: &Path) -> Result<Facts, PrivateSocketError> {
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

pub(crate) fn refuse_if(
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

#[cfg(target_os = "macos")]
pub(crate) fn refuse_replacing_acl(dir: &OwnedFd, at: &Path) -> Result<(), PrivateSocketError> {
    refuse_if(darwin_acl::allows_replacing_entries(dir), at, |at| {
        PrivateSocketError::ReplaceableByAcl { path: at }
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn refuse_replacing_acl(_dir: &OwnedFd, _at: &Path) -> Result<(), PrivateSocketError> {
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn refuse_ignored_ownership(dir: &OwnedFd, at: &Path) -> Result<(), PrivateSocketError> {
    // SAFETY: libc::statfs is a plain C struct, for which all zeroes is a valid value.
    let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `dir` keeps its descriptor open, and `stats` is a local libc::statfs that fstatfs may
    // write.
    let ignored = if unsafe { libc::fstatfs(dir.as_raw_fd(), &mut stats) } == 0 {
        Ok(stats.f_flags & libc::MNT_IGNORE_OWNERSHIP as u32 != 0)
    } else {
        Err(("fstatfs", last_errno()))
    };
    refuse_if(ignored, at, |path| PrivateSocketError::OwnershipIgnored {
        path,
    })
}

#[cfg(target_os = "linux")]
fn refuse_ignored_ownership(_dir: &OwnedFd, _at: &Path) -> Result<(), PrivateSocketError> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) mod darwin_acl {
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
        fn of(dir: &OwnedFd) -> Result<Option<Acl>, (&'static str, i32)> {
            // SAFETY: acl_get_fd_np reads only the descriptor, which `dir` keeps open. A non-null
            // result is a new ACL that this Acl then owns.
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

    pub(crate) fn has_entries(dir: &OwnedFd) -> Result<bool, (&'static str, i32)> {
        Ok(Acl::of(dir)?.is_some_and(|acl| acl.entries().next().is_some()))
    }

    #[allow(unsafe_code)]
    pub(super) fn allows_replacing_entries(dir: &OwnedFd) -> Result<bool, (&'static str, i32)> {
        let Some(acl) = Acl::of(dir)? else {
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
            if mask & REPLACES_ENTRIES != 0 {
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
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::Walk;

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
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            open_up(&self.path);
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
    use super::fixtures::{TestRoot, make_dir, make_fifo, set_mode, within_a_second, write_file};
    use super::*;
    use crate::private_socket::PrivateSocketError;
    use std::fs;
    use std::os::fd::AsFd;
    use std::os::unix::fs::{MetadataExt, symlink};
    use std::os::unix::net::UnixListener;

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

    fn identity_of(path: &Path) -> DirIdentity {
        let metadata = fs::symlink_metadata(path).expect("stat");
        DirIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }

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
                .open_file(accept)
                .map(|opened| opened.facts),
            Err(WalkRefusal::Replaceable {
                path: root.join("a"),
                mode: 0o770
            })
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
        assert_eq!(
            root.walk("file")
                .open_file(accept)
                .map(|opened| opened.facts),
            Err(WalkRefusal::Io {
                op: "openat",
                path: root.join("file"),
                errno: libc::EACCES
            })
        );
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
        use super::fixtures::{add_acl, add_raw_allow_for_everyone};
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
            "everyone allow read,readattr\neveryone allow write",
        ]
        .iter()
        .enumerate()
        {
            let name = format!("granted{index}");
            write_file(&root.join(&name), b"", 0o600);
            for line in rule.lines() {
                add_acl(&root.join(&name), line);
            }
            assert!(granted(&name), "{rule:?}");
        }
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
            assert_eq!(refusal.path(), at());
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
