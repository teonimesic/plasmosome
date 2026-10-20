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
