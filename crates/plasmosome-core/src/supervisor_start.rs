use crate::instance::SupervisorLock;
use crate::runtime::path::RecipePath;
use plasmosome_backend::CellId;
use std::ffi::{CStr, CString};
use std::fmt;
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::time::Instant;

const CONFIG_FD_FLAG: &CStr = c"--config-fd=3";

const LOCK_FD_FLAG: &CStr = c"--lock-fd=4";

const FIRST_UNHANDED: RawFd = 5;

const LAST_SIGNAL: libc::c_int = 64;

const UNWOUND: libc::c_int = 70;

const FAILED: libc::c_int = 127;

/// Darwin's `fcntl` command from `<sys/fcntl.h>` that stops a write to a descriptor raising
/// `SIGPIPE`, so the write fails with `EPIPE` instead. The pinned `libc` does not define it.
#[cfg(target_os = "macos")]
const F_SETNOSIGPIPE: libc::c_int = 73;

/// One cell's `membraned` start for [`start_supervisor`]: what it executes, the lock and log it
/// inherits, and the config record it reads.
pub struct SupervisorStart<'a> {
    /// The `membraned` to execute, already verified under spec 023 §1. It is `argv[0]`.
    pub membraned: &'a RecipePath,
    /// The cell, passed as `--cell=<id>`. An ID holding a NUL byte is refused.
    pub cell: &'a CellId,
    /// The cell's held `supervisor.lock`, from `CellDir::create_supervisor_lock`. It becomes
    /// descriptor 4.
    pub lock: SupervisorLock,
    /// `membraned.log`, from `CellDir::open_supervisor_log`. It becomes descriptor 2.
    pub log: File,
    /// The config record, from `SupervisorConfig::encode`. It is written to descriptor 3 after the
    /// exec, so it may be larger than a pipe holds.
    pub record: Vec<u8>,
}

/// Why [`start_supervisor`] did not hand `membraned` its whole record.
///
/// `Prepare` and `Exec` mean no `membraned` runs, and they return the lock, still held, so the
/// caller can retire the cell. The other three mean `membraned` was executed and the controller
/// no longer holds the lock: `membraned` holds it on descriptor 4 until it exits.
#[derive(Debug)]
pub enum StartError {
    /// The start could not be laid out, and nothing was forked. A cell ID holding a NUL byte is
    /// `InvalidInput`; otherwise `source` is the call that failed.
    Prepare {
        lock: SupervisorLock,
        source: io::Error,
    },
    /// `step` failed with `errno`, in this process or in a child before its exec. A failure
    /// report the children cannot have written, or a failed read of it, reads as `Exec` with
    /// `EPROTO` or the read's errno.
    Exec {
        lock: SupervisorLock,
        step: StartStep,
        errno: i32,
    },
    /// Descriptor 3 was closed before `membraned` read the whole record: `membraned` exited or
    /// closed it. A new process that died before its exec without a report, such as by a signal
    /// sent to the caller's process group before it left that group, also ends here, since the
    /// start reads its silence as a successful exec.
    SupervisorClosedPipe,
    /// `membraned` had not read the whole record by the deadline.
    RecordNotTaken,
    /// Writing the record failed for another reason.
    Write(io::Error),
}

/// The step of a start that failed before `membraned` was executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartStep {
    /// A `fork`: this process's own, or the short-lived intermediate's.
    Fork,
    /// `setsid` in the new process.
    Session,
    /// Copying the five descriptors onto 0 to 4, or closing every descriptor above 4.
    Descriptors,
    /// Setting every signal to its default action, or emptying the signal mask.
    Signals,
    /// The `execve` of `membraned`.
    Exec,
}

/// Starts one cell's `membraned` as spec 023 §4 says, then writes it the config record.
///
/// The new process executes `[membraned, "--config-fd=3", "--lock-fd=4", "--cell=<id>"]` with an
/// empty environment. Its descriptors 0 and 1 are `/dev/null`, 2 is `log`, 3 is the read end of a
/// new pipe and 4 is `lock`; every other descriptor is closed, close-on-exec or not, wherever it
/// sat in this process. Every signal is at its default action and none is blocked. It leads a new
/// session and process group, and its parent is the operating system's init process, because a
/// short-lived intermediate forks it and exits. This function reaps the intermediate and keeps no
/// PID.
///
/// Once the exec succeeded, it closes its lock and its copy of the pipe's read end, so the lock is
/// never free: `membraned` holds the same open file on descriptor 4. Then it writes `record`,
/// waiting no later than `deadline` for `membraned` to read, and closes the pipe. Returns `Ok`
/// once the whole record is written. The wait for the exec itself is not bounded by `deadline`.
///
/// The children run only async-signal-safe calls between `fork` and `exec` and allocate nothing,
/// so the caller may have other threads. The calling thread blocks every signal from just before
/// the fork until it returns, so no handler of this process runs in a child.
///
/// On Darwin a new pipe is close-on-exec only after a second call. A process this one starts at
/// that moment by a spawn that does not close every other descriptor can inherit a pipe end, and
/// this call then waits until that process exits or execs. Spec 023 has every start close them.
/// On Darwin the record's pipe never raises `SIGPIPE`. On Linux a reader that exits gives
/// `SupervisorClosedPipe` only because a Rust `main` ignores `SIGPIPE`; a process that restores
/// it dies instead. On Linux the new process closes descriptors with `close_range`, which needs
/// Linux 5.9 or later.
pub fn start_supervisor(start: SupervisorStart<'_>, deadline: Instant) -> Result<(), StartError> {
    let SupervisorStart {
        membraned,
        cell,
        lock,
        log,
        record,
    } = start;
    let layout = match Layout::new(membraned, cell, &lock, &log) {
        Ok(layout) => layout,
        Err(source) => return Err(StartError::Prepare { lock, source }),
    };
    drop(log);
    let config = match layout.spawn() {
        Ok(config) => config,
        Err(Failure::Prepare(source)) => return Err(StartError::Prepare { lock, source }),
        Err(Failure::Step(step, errno)) => return Err(StartError::Exec { lock, step, errno }),
    };
    drop(lock);
    write_record(&config, &record, deadline)
}

enum Failure {
    Prepare(io::Error),
    Step(StartStep, i32),
}

struct Layout {
    program: CString,
    cell_flag: CString,
    dev_null: OwnedFd,
    log: OwnedFd,
    config_read: OwnedFd,
    config_write: OwnedFd,
    lock: OwnedFd,
    report_read: OwnedFd,
    report_write: OwnedFd,
    closer: Closer,
}

impl Layout {
    fn new(
        membraned: &RecipePath,
        cell: &CellId,
        lock: &SupervisorLock,
        log: &File,
    ) -> io::Result<Layout> {
        let cell_flag = CString::new(format!("--cell={}", cell.as_str())).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "the cell ID holds a NUL byte")
        })?;
        let closer = Closer::prepare()?;
        let dev_null = File::options().read(true).write(true).open("/dev/null")?;
        let dev_null = above_four(dev_null.as_fd())?;
        let log = above_four(log.as_fd())?;
        let lock = above_four(lock.as_fd())?;
        let [config_read, config_write] = pipe_above_four()?;
        let [report_read, report_write] = pipe_above_four()?;
        set_nonblocking(config_write.as_fd())?;
        never_raise_sigpipe(config_write.as_fd())?;
        Ok(Layout {
            program: membraned.as_c_str().to_owned(),
            cell_flag,
            dev_null,
            log,
            config_read,
            config_write,
            lock,
            report_read,
            report_write,
            closer,
        })
    }

    fn spawn(self) -> Result<OwnedFd, Failure> {
        let argv = [
            self.program.as_ptr(),
            CONFIG_FD_FLAG.as_ptr(),
            LOCK_FD_FLAG.as_ptr(),
            self.cell_flag.as_ptr(),
            std::ptr::null(),
        ];
        let envp: [*const libc::c_char; 1] = [std::ptr::null()];
        let child = Child {
            program: self.program.as_ptr(),
            argv: argv.as_ptr(),
            envp: envp.as_ptr(),
            sources: [
                self.dev_null.as_raw_fd(),
                self.dev_null.as_raw_fd(),
                self.log.as_raw_fd(),
                self.config_read.as_raw_fd(),
                self.lock.as_raw_fd(),
            ],
            report: self.report_write.as_raw_fd(),
            closer: self.closer,
        };
        let intermediate = fork_intermediate(&child)?;
        let Layout {
            dev_null,
            log,
            config_read,
            config_write,
            lock,
            report_read,
            report_write,
            ..
        } = self;
        drop((dev_null, log, lock, report_write));
        reap(intermediate);
        match read_report(report_read.as_fd()) {
            None => {
                drop(config_read);
                Ok(config_write)
            }
            Some((step, errno)) => Err(Failure::Step(step, errno)),
        }
    }
}

#[derive(Clone, Copy)]
struct Child {
    program: *const libc::c_char,
    argv: *const *const libc::c_char,
    envp: *const *const libc::c_char,
    sources: [RawFd; 5],
    report: RawFd,
    closer: Closer,
}

#[allow(unsafe_code)]
fn fork_intermediate(child: &Child) -> Result<libc::pid_t, Failure> {
    let blocked = BlockedSignals::all().map_err(Failure::Prepare)?;
    // SAFETY: The child runs only `intermediate`, which makes only async-signal-safe calls and ends
    // in `_exit` or `execve`, so it allocates nothing and takes no lock another thread may hold.
    // Every pointer `child` holds points into `Layout::spawn`'s frame and the Layout, which the
    // child's copy of memory keeps, and every descriptor it names is open until this returns.
    let forked = unsafe { libc::fork() };
    if forked == 0 {
        // SAFETY: fork returned 0, so this is the new child, with every signal blocked.
        unsafe { intermediate(child) }
    }
    let errno = errno();
    drop(blocked);
    if forked == -1 {
        Err(Failure::Step(StartStep::Fork, errno))
    } else {
        Ok(forked)
    }
}

/// Forks the process that becomes `membraned` and exits at once, so `membraned`'s parent is
/// init.
///
/// # Safety
///
/// Call only in a child that `fork` has just returned 0 in, with every signal blocked, where
/// `child` was laid out by the parent before the fork. It never returns.
#[allow(unsafe_code)]
unsafe fn intermediate(child: &Child) -> ! {
    let _guard = ExitOnUnwind;
    // SAFETY: This child has one thread. fork is on POSIX.1-2017's list of async-signal-safe
    // calls. Its atfork handlers take only locks the first fork's child handlers released, so it
    // cannot wait on a lock another thread of the parent held.
    match unsafe { libc::fork() } {
        // SAFETY: fork returned 0, so this is the grandchild, with every signal still blocked.
        0 => unsafe { grandchild(child) },
        // SAFETY: This is the intermediate child, with `child`'s report descriptor open.
        -1 => unsafe { child.fail(StartStep::Fork) },
        // SAFETY: _exit is async-signal-safe and runs no Rust destructor or exit handler.
        _ => unsafe { libc::_exit(0) },
    }
}

/// Becomes `membraned`: a new session, descriptors 0 to 4 and no other, default signals, then
/// `execve`. Any failure is reported on `child.report` before `_exit`.
///
/// # Safety
///
/// Call only in the grandchild that the intermediate's `fork` has just returned 0 in, with every
/// signal blocked. It never returns.
#[allow(unsafe_code)]
unsafe fn grandchild(child: &Child) -> ! {
    // SAFETY: setsid takes no arguments and is async-signal-safe. This process is not a group
    // leader, since its group is its grandparent's, so it can start a session.
    if unsafe { libc::setsid() } == -1 {
        // SAFETY: This is the grandchild, with `child`'s report descriptor open.
        unsafe { child.fail(StartStep::Session) }
    }
    // SAFETY: This is the grandchild, so the descriptors it replaces and closes are its own.
    if !unsafe { child.install_descriptors() && child.closer.close_above_four(child.report) } {
        // SAFETY: This is the grandchild, and the close spared `child`'s report descriptor.
        unsafe { child.fail(StartStep::Descriptors) }
    }
    // SAFETY: This is the grandchild, so the signal actions and mask it resets are its own.
    if !unsafe { default_signal_actions() && unblock_every_signal() } {
        // SAFETY: This is the grandchild, with `child`'s report descriptor open.
        unsafe { child.fail(StartStep::Signals) }
    }
    // SAFETY: `program`, `argv` and `envp` point into memory the parent laid out before the fork
    // and this process's copy keeps: NUL-terminated strings, and arrays that end in a null
    // pointer. execve is async-signal-safe.
    unsafe { libc::execve(child.program, child.argv, child.envp) };
    // SAFETY: execve returned, so this is still the grandchild, with its report descriptor open.
    unsafe { child.fail(StartStep::Exec) }
}

impl Child {
    /// Copies the five sources onto descriptors 0 to 4. Every source is 5 or above, so no copy
    /// replaces a source a later one needs, and each copy clears close-on-exec on its target.
    ///
    /// # Safety
    ///
    /// Call only in the grandchild, before `execve`.
    #[allow(unsafe_code)]
    unsafe fn install_descriptors(&self) -> bool {
        self.sources.iter().zip(0..).all(|(&source, target)| {
            // SAFETY: dup2 takes integers only and is async-signal-safe.
            unsafe { libc::dup2(source, target) == target }
        })
    }

    /// Writes `step` and errno to the report pipe and exits 127.
    ///
    /// # Safety
    ///
    /// Call only in a child of the start, where `self.report` is the report pipe's write end.
    #[allow(unsafe_code)]
    unsafe fn fail(&self, step: StartStep) -> ! {
        let report = encode_report(step, errno());
        // SAFETY: `report` is a local of `report.len()` bytes that write only reads. write and
        // _exit are async-signal-safe, and _exit runs no Rust destructor or exit handler.
        unsafe {
            libc::write(self.report, report.as_ptr().cast(), report.len());
            libc::_exit(FAILED)
        }
    }
}

#[derive(Clone, Copy)]
struct Closer {
    #[cfg(target_os = "macos")]
    limit: libc::c_int,
}

impl Closer {
    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn prepare() -> io::Result<Closer> {
        let mut per_process: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>();
        // SAFETY: The name is a static NUL-terminated string. `per_process` and `size` are locals
        // that sysctlbyname may write, `size` holds the bytes `per_process` has, and no new value
        // is passed.
        let read = unsafe {
            libc::sysctlbyname(
                c"kern.maxfilesperproc".as_ptr(),
                (&raw mut per_process).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: getdtablesize takes no arguments.
        let table = unsafe { libc::getdtablesize() };
        Ok(Closer {
            limit: table.max(per_process),
        })
    }

    #[cfg(target_os = "linux")]
    fn prepare() -> io::Result<Closer> {
        Ok(Closer {})
    }

    /// Closes every descriptor from 5 up except `keep`, up to the larger of the descriptor table
    /// size and `kern.maxfilesperproc`, so none opened before a limit was lowered survives.
    ///
    /// # Safety
    ///
    /// Call only in the grandchild, before `execve`.
    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    unsafe fn close_above_four(self, keep: RawFd) -> bool {
        for fd in (FIRST_UNHANDED..self.limit).filter(|fd| *fd != keep) {
            // SAFETY: close takes an integer and is async-signal-safe. Every descriptor from 5 up
            // but `keep` is one membraned must not inherit, whoever made it.
            unsafe { libc::close(fd) };
        }
        true
    }

    /// Closes every descriptor from 5 up except `keep`, with `close_range`.
    ///
    /// # Safety
    ///
    /// Call only in the grandchild, before `execve`.
    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    unsafe fn close_above_four(self, keep: RawFd) -> bool {
        let close_range = |first: RawFd, last: libc::c_uint| {
            // SAFETY: close_range takes integers only and is a raw system call, so it allocates
            // nothing. Every descriptor in the range is one membraned must not inherit.
            unsafe { libc::syscall(libc::SYS_close_range, first as libc::c_uint, last, 0) == 0 }
        };
        (keep == FIRST_UNHANDED || close_range(FIRST_UNHANDED, (keep - 1) as libc::c_uint))
            && close_range(keep + 1, libc::c_uint::MAX)
    }
}

/// Sets every signal but `SIGKILL` and `SIGSTOP` to its default action, so none this process
/// ignored or caught stays so across `execve`.
///
/// # Safety
///
/// Call only in the grandchild, before `execve`, with every signal blocked.
#[allow(unsafe_code)]
unsafe fn default_signal_actions() -> bool {
    // SAFETY: libc::sigaction is a plain C struct, for which all zeroes is a valid value: the
    // default action with an empty mask and no flags.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = libc::SIG_DFL;
    (1..=LAST_SIGNAL)
        .filter(|signal| *signal != libc::SIGKILL && *signal != libc::SIGSTOP)
        .all(|signal| {
            // SAFETY: `action` is a local the call only reads, the old action is not asked for,
            // and sigaction is async-signal-safe.
            let set = unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) };
            set == 0 || errno() == libc::EINVAL
        })
}

/// Empties the signal mask.
///
/// # Safety
///
/// Call only in the grandchild, before `execve`, after every signal action is the default.
#[allow(unsafe_code)]
unsafe fn unblock_every_signal() -> bool {
    // SAFETY: All zeroes is a valid sigset_t, which sigemptyset then empties. The set is a local
    // that outlives the calls. sigemptyset and sigprocmask are async-signal-safe, and this process
    // has one thread, so sigprocmask sets its whole mask.
    unsafe {
        let mut empty: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut empty) == 0
            && libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut()) == 0
    }
}

struct ExitOnUnwind;

impl Drop for ExitOnUnwind {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: Only the start's children make an ExitOnUnwind. _exit is async-signal-safe and
        // ends the child before an unwind goes further.
        unsafe { libc::_exit(UNWOUND) }
    }
}

struct BlockedSignals(libc::sigset_t);

impl BlockedSignals {
    #[allow(unsafe_code)]
    fn all() -> io::Result<BlockedSignals> {
        // SAFETY: All zeroes is a valid sigset_t, and sigfillset then fills `every`. Both sets are
        // locals that outlive the calls, and pthread_sigmask changes only this thread's mask.
        unsafe {
            let mut every: libc::sigset_t = std::mem::zeroed();
            let mut previous: libc::sigset_t = std::mem::zeroed();
            libc::sigfillset(&mut every);
            match libc::pthread_sigmask(libc::SIG_BLOCK, &every, &mut previous) {
                0 => Ok(BlockedSignals(previous)),
                code => Err(io::Error::from_raw_os_error(code)),
            }
        }
    }
}

impl Drop for BlockedSignals {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: `self.0` is the mask saved when every signal was blocked, which pthread_sigmask
        // only reads.
        unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &self.0, std::ptr::null_mut()) };
    }
}

#[allow(unsafe_code)]
fn reap(intermediate: libc::pid_t) {
    let mut status = 0;
    // SAFETY: `status` is a local that waitpid may write. `intermediate` is a child this process
    // forked and has not reaped, so its pid still names it.
    while unsafe { libc::waitpid(intermediate, &mut status, 0) } == -1 && errno() == libc::EINTR {}
}

#[allow(unsafe_code)]
fn read_report(report: BorrowedFd<'_>) -> Option<(StartStep, i32)> {
    let mut bytes = [0u8; 8];
    let mut filled = 0;
    while let Some(rest) = bytes.get_mut(filled..).filter(|rest| !rest.is_empty()) {
        // SAFETY: `rest` is `rest.len()` writable bytes of a local buffer, and `report` keeps its
        // descriptor open.
        let got = unsafe { libc::read(report.as_raw_fd(), rest.as_mut_ptr().cast(), rest.len()) };
        match usize::try_from(got) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(_) if errno() == libc::EINTR => {}
            Err(_) => return Some((StartStep::Exec, errno())),
        }
    }
    match filled {
        0 => None,
        8 => Some(decode_report(bytes)),
        _ => Some((StartStep::Exec, libc::EPROTO)),
    }
}

fn encode_report(step: StartStep, errno: i32) -> [u8; 8] {
    let code: i32 = match step {
        StartStep::Fork => 1,
        StartStep::Session => 2,
        StartStep::Descriptors => 3,
        StartStep::Signals => 4,
        StartStep::Exec => 5,
    };
    let [a, b, c, d] = code.to_ne_bytes();
    let [e, f, g, h] = errno.to_ne_bytes();
    [a, b, c, d, e, f, g, h]
}

fn decode_report(bytes: [u8; 8]) -> (StartStep, i32) {
    let [a, b, c, d, e, f, g, h] = bytes;
    let errno = i32::from_ne_bytes([e, f, g, h]);
    match i32::from_ne_bytes([a, b, c, d]) {
        1 => (StartStep::Fork, errno),
        2 => (StartStep::Session, errno),
        3 => (StartStep::Descriptors, errno),
        4 => (StartStep::Signals, errno),
        5 => (StartStep::Exec, errno),
        _ => (StartStep::Exec, libc::EPROTO),
    }
}

#[allow(unsafe_code)]
fn write_record(config: &OwnedFd, record: &[u8], deadline: Instant) -> Result<(), StartError> {
    let mut written = 0;
    while let Some(rest) = record.get(written..).filter(|rest| !rest.is_empty()) {
        // SAFETY: `rest` borrows `rest.len()` live bytes of `record`, which write only reads, and
        // `config` keeps its descriptor open.
        let wrote = unsafe { libc::write(config.as_raw_fd(), rest.as_ptr().cast(), rest.len()) };
        match usize::try_from(wrote) {
            Ok(count) if count > 0 => written += count,
            Ok(_) => wait_writable(config.as_fd(), deadline)?,
            Err(_) => match errno() {
                libc::EINTR => {}
                libc::EAGAIN => wait_writable(config.as_fd(), deadline)?,
                libc::EPIPE => return Err(StartError::SupervisorClosedPipe),
                errno => return Err(StartError::Write(io::Error::from_raw_os_error(errno))),
            },
        }
    }
    Ok(())
}

#[allow(unsafe_code)]
fn wait_writable(config: BorrowedFd<'_>, deadline: Instant) -> Result<(), StartError> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(StartError::RecordNotTaken);
        }
        let milliseconds = left.as_nanos().div_ceil(1_000_000);
        let timeout = libc::c_int::try_from(milliseconds).unwrap_or(libc::c_int::MAX);
        let mut watched = libc::pollfd {
            fd: config.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        // SAFETY: `watched` is one local pollfd, matching the count of 1, and `config` keeps its
        // descriptor open.
        match unsafe { libc::poll(&mut watched, 1, timeout) } {
            ready if ready > 0 => return Ok(()),
            0 => {}
            _ if errno() == libc::EINTR => {}
            _ => return Err(StartError::Write(io::Error::from_raw_os_error(errno()))),
        }
    }
}

#[allow(unsafe_code)]
fn above_four(fd: BorrowedFd<'_>) -> io::Result<OwnedFd> {
    // SAFETY: fcntl with F_DUPFD_CLOEXEC takes integers only, and `fd` is open for the call.
    match unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, FIRST_UNHANDED) } {
        -1 => Err(io::Error::last_os_error()),
        // SAFETY: fcntl returned a new descriptor, which nothing else owns.
        copied => Ok(unsafe { OwnedFd::from_raw_fd(copied) }),
    }
}

#[allow(unsafe_code)]
fn pipe_above_four() -> io::Result<[OwnedFd; 2]> {
    let mut ends: [RawFd; 2] = [-1; 2];
    #[cfg(target_os = "linux")]
    // SAFETY: `ends` is a local array of two c_int, the size pipe2 writes.
    let made = unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) };
    #[cfg(not(target_os = "linux"))]
    // SAFETY: `ends` is a local array of two c_int, the size pipe writes.
    let made = unsafe { libc::pipe(ends.as_mut_ptr()) };
    if made == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: pipe returned 0, so both entries are new descriptors, which nothing else owns.
    let [read, write] = ends.map(|end| unsafe { OwnedFd::from_raw_fd(end) });
    Ok([above_four(read.as_fd())?, above_four(write.as_fd())?])
}

#[allow(unsafe_code)]
fn set_nonblocking(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: fcntl with F_GETFL takes no pointer, and `fd` is open for the call.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl with F_SETFL takes an integer flag set, not a pointer, and `fd` is open.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn never_raise_sigpipe(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: fcntl with F_SETNOSIGPIPE takes an integer, and `fd` is open for the call.
    if unsafe { libc::fcntl(fd.as_raw_fd(), F_SETNOSIGPIPE, 1) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn never_raise_sigpipe(_fd: BorrowedFd<'_>) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn errno() -> i32 {
    // SAFETY: __error takes no arguments and returns this thread's errno, which lives as long as
    // the thread. Reading it is async-signal-safe.
    unsafe { *libc::__error() }
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn errno() -> i32 {
    // SAFETY: __errno_location takes no arguments and returns this thread's errno, which lives as
    // long as the thread. Reading it is async-signal-safe.
    unsafe { *libc::__errno_location() }
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::Prepare { source, .. } => {
                write!(f, "the supervisor's start could not be laid out: {source}")
            }
            StartError::Exec { step, errno, .. } => {
                write!(f, "starting the supervisor failed at {step}: errno {errno}")
            }
            StartError::SupervisorClosedPipe => {
                f.write_str("the supervisor closed its config pipe before reading the whole record")
            }
            StartError::RecordNotTaken => {
                f.write_str("the supervisor had not read its whole config record by the deadline")
            }
            StartError::Write(source) => {
                write!(
                    f,
                    "the supervisor's config record could not be written: {source}"
                )
            }
        }
    }
}

impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StartError::Prepare { source, .. } | StartError::Write(source) => Some(source),
            _ => None,
        }
    }
}

impl fmt::Display for StartStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StartStep::Fork => "fork",
            StartStep::Session => "setsid",
            StartStep::Descriptors => "descriptor setup",
            StartStep::Signals => "signal reset",
            StartStep::Exec => "execve",
        })
    }
}
