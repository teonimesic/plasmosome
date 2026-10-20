use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub use libc::{MS_NODEV, MS_NOEXEC, MS_NOSUID, MS_RDONLY};

fn c_string(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}

fn check(rc: libc::c_int) -> io::Result<()> {
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// mount(2) `source` at `target` as `fstype` with `flags` and no data string.
pub fn mount(source: &str, target: &Path, fstype: &str, flags: libc::c_ulong) -> io::Result<()> {
    let source = c_string(source.as_bytes())?;
    let target = c_string(target.as_os_str().as_bytes())?;
    let fstype = c_string(fstype.as_bytes())?;
    // SAFETY: the three pointers are NUL-terminated strings that outlive the call, and a null
    // data pointer is what mount(2) takes when there are no filesystem options.
    let rc = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            fstype.as_ptr(),
            flags,
            std::ptr::null(),
        )
    };
    check(rc)
}

/// Flush filesystems and power the machine off. Returns only if reboot(2) refused. Any caller
/// with `CAP_SYS_BOOT`, such as a probe running as uid 0, stops the guest, so only `lane-init`
/// calls it.
pub fn power_off() -> io::Error {
    // SAFETY: sync takes no arguments and only flushes filesystem buffers.
    unsafe { libc::sync() };
    // SAFETY: RB_POWER_OFF takes no pointer, so the call touches no memory of this process.
    // reboot(2) checks CAP_SYS_BOOT, not the caller's PID: with it the machine stops and the call
    // does not return, and without it the call fails with EPERM and changes nothing.
    unsafe { libc::reboot(libc::RB_POWER_OFF) };
    io::Error::last_os_error()
}

/// Reap every exited child without blocking, as process 1 must for orphans it inherits.
pub fn reap_exited() {
    loop {
        let mut status = 0;
        // SAFETY: `status` is a valid place for waitpid to store an exit status, and WNOHANG
        // makes the call return at once when no child has exited.
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            return;
        }
    }
}

/// Wait for every child until none is left, and return how many were reaped. It blocks while a
/// child is alive, so call it after `kill_all`.
pub fn reap_all() -> usize {
    let mut reaped = 0;
    loop {
        let mut status = 0;
        // SAFETY: `status` is a valid place for waitpid to store an exit status.
        let pid = unsafe { libc::waitpid(-1, &mut status, 0) };
        if pid > 0 {
            reaped += 1;
        } else if io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return reaped;
        }
    }
}

/// Send SIGKILL to every process this one may signal; process 1 itself is spared.
pub fn kill_all() {
    // SAFETY: kill takes no pointer; -1 addresses every process the caller may signal except
    // itself, and a failure only sets errno.
    unsafe { libc::kill(-1, libc::SIGKILL) };
}

/// The fields uname(2) reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uname {
    pub sysname: String,
    pub nodename: String,
    pub release: String,
    pub version: String,
    pub machine: String,
}

fn field(raw: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = raw
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| c.to_ne_bytes()[0])
        .collect();
    String::from_utf8_lossy(&bytes).to_string()
}

/// uname(2).
pub fn uname() -> io::Result<Uname> {
    // SAFETY: utsname is plain C data, and all-zero bytes are a valid value of it.
    let mut raw: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: `raw` is a valid, writable utsname for the kernel to fill.
    check(unsafe { libc::uname(&mut raw) })?;
    Ok(Uname {
        sysname: field(&raw.sysname),
        nodename: field(&raw.nodename),
        release: field(&raw.release),
        version: field(&raw.version),
        machine: field(&raw.machine),
    })
}

/// A private read-only mapping of a whole file, unmapped on drop.
pub struct Mapping {
    address: *mut libc::c_void,
    length: usize,
}

impl Mapping {
    /// Map all of `file` with `PROT_READ` and `MAP_PRIVATE`. An empty file is refused.
    pub fn map(file: &File) -> io::Result<Mapping> {
        let length = usize::try_from(file.metadata()?.len())
            .map_err(|_| io::Error::from_raw_os_error(libc::EOVERFLOW))?;
        if length == 0 {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        // SAFETY: a null hint lets the kernel choose the address; the descriptor is open for
        // reading for the duration of the call, and the mapping outlives it on its own.
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                file.as_raw_fd(),
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Mapping { address, length })
    }

    /// The mapping's length in bytes.
    pub fn len(&self) -> usize {
        self.length
    }

    /// True for a mapping of no bytes, which `map` never returns.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Copy the mapped bytes out one volatile read at a time, so the bytes are never borrowed
    /// as a `&[u8]` while the file under them can change. A page the file no longer backs
    /// raises SIGBUS, which ends the process.
    pub fn copy_out(&self) -> Vec<u8> {
        let base = self.address.cast::<u8>();
        (0..self.length)
            .map(|offset| {
                // SAFETY: `offset` is below the mapped length, so the address lies inside the
                // live PROT_READ mapping; the read is volatile because the file may change.
                unsafe { std::ptr::read_volatile(base.add(offset)) }
            })
            .collect()
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: `address` and `length` are the values mmap returned and used; the mapping is
        // unmapped once, here.
        unsafe { libc::munmap(self.address, self.length) };
    }
}
