use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::Path;

pub use libc::{
    CLONE_FILES, CLONE_FS, CLONE_NEWNS, MNT_DETACH, MS_BIND, MS_NODEV, MS_NOEXEC, MS_NOSUID,
    MS_PRIVATE, MS_RDONLY, MS_SHARED, MS_SLAVE,
};

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

/// umount2(2) of `target` with `flags`, such as `MNT_DETACH` for a lazy unmount.
pub fn umount2(target: &Path, flags: libc::c_int) -> io::Result<()> {
    let target = c_string(target.as_os_str().as_bytes())?;
    // SAFETY: the pointer is a NUL-terminated string that outlives the call.
    check(unsafe { libc::umount2(target.as_ptr(), flags) })
}

/// unshare(2) with `flags`: `CLONE_FS | CLONE_FILES` gives the calling thread its own working
/// directory and descriptor table, and `CLONE_NEWNS` gives the process its own mount table.
pub fn unshare(flags: libc::c_int) -> io::Result<()> {
    // SAFETY: unshare takes one integer and touches no memory of this process; a failure only
    // sets errno.
    check(unsafe { libc::unshare(flags) })
}

/// The calling process's effective uid, as `id -u` prints it.
pub fn effective_uid() -> u32 {
    // SAFETY: geteuid takes no arguments, always succeeds and touches no memory.
    unsafe { libc::geteuid() }
}

const ONE_FD: libc::c_uint = size_of::<libc::c_int>() as libc::c_uint;

struct Message {
    byte: [u8; 1],
    iov: libc::iovec,
    control: [u64; 4],
    header: libc::msghdr,
}

impl Message {
    fn new() -> Box<Message> {
        let mut message = Box::new(Message {
            byte: [0],
            iov: libc::iovec {
                iov_base: std::ptr::null_mut(),
                iov_len: 1,
            },
            control: [0; 4],
            // SAFETY: msghdr is plain C data, and all-zero bytes are a valid value of it: null
            // pointers and zero lengths.
            header: unsafe { std::mem::zeroed() },
        });
        message.iov.iov_base = message.byte.as_mut_ptr().cast();
        message.header.msg_iov = &mut message.iov;
        message.header.msg_iovlen = 1;
        message.header.msg_control = message.control.as_mut_ptr().cast();
        // SAFETY: CMSG_SPACE only computes a size from its argument.
        message.header.msg_controllen = unsafe { libc::CMSG_SPACE(ONE_FD) } as _;
        message
    }
}

/// Send `fd` over the connected Unix socket `socket` as one `SCM_RIGHTS` message of one byte.
/// The descriptor is then in flight: it is in no process's table until a receiver takes it.
pub fn send_fd(socket: &UnixStream, fd: BorrowedFd<'_>) -> io::Result<()> {
    let message = Message::new();
    // SAFETY: msg_control points at `control`, 32 bytes aligned for a cmsghdr, and
    // msg_controllen is CMSG_SPACE of one int, which fits in it, so CMSG_FIRSTHDR returns a
    // pointer inside `control` and CMSG_DATA one with room for one int after the header.
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message.header);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(ONE_FD) as _;
        std::ptr::write_unaligned(
            libc::CMSG_DATA(header).cast::<libc::c_int>(),
            fd.as_raw_fd(),
        );
    }
    // SAFETY: the message's pointers point into `message`, which is boxed and outlives the call,
    // and the socket's descriptor is open for the duration of the call.
    let sent = unsafe { libc::sendmsg(socket.as_raw_fd(), &message.header, 0) };
    match sent {
        1 => Ok(()),
        -1 => Err(io::Error::last_os_error()),
        _ => Err(io::Error::other("sendmsg sent no byte")),
    }
}

/// Receive one descriptor `send_fd` sent over `socket`, close-on-exec.
pub fn recv_fd(socket: &UnixStream) -> io::Result<OwnedFd> {
    let mut message = Message::new();
    // SAFETY: the message's pointers point into `message`, which is boxed and outlives the call,
    // and its buffers are as large as its lengths say.
    let received = unsafe {
        libc::recvmsg(
            socket.as_raw_fd(),
            &mut message.header,
            libc::MSG_CMSG_CLOEXEC,
        )
    };
    if received == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: recvmsg filled `control` and set msg_controllen to the bytes it wrote, so
    // CMSG_FIRSTHDR returns null or a header inside `control`.
    let header = unsafe { libc::CMSG_FIRSTHDR(&message.header) };
    // SAFETY: a non-null header points inside `control`, which recvmsg wrote.
    let rights = !header.is_null()
        && unsafe {
            (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS
        };
    if !rights {
        return Err(io::Error::other("the message carried no descriptor"));
    }
    // SAFETY: an SCM_RIGHTS header carries at least one int after it, inside `control`.
    let fd = unsafe { std::ptr::read_unaligned(libc::CMSG_DATA(header).cast::<libc::c_int>()) };
    // SAFETY: the kernel installed `fd` in this process's table for this call, and nothing else
    // owns it, so the OwnedFd is its only owner.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
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
