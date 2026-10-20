use crate::runtime::recipe::RuntimeRecipe;
use crate::spawn_lock::DescriptorsHeld;
use crate::vmm::Launch;
use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

const RUNTIME_FD_FLAG: &CStr = c"--runtime-fd=3";

const FIRST_KEPT: libc::c_int = 10;

const LAST_SIGNAL: libc::c_int = 64;

/// The exit code of a launched child whose setup before `execve` failed: the
/// signal reset, a `dup2` onto descriptors 0 to 3, closing the descriptors
/// above 3, or `chdir("/")`.
pub const SETUP_FAILED: i32 = 125;

/// The exit code of a launched child whose `execve` of the helper returned.
pub const EXEC_FAILED: i32 = 127;

/// The pinned VMM helper, laid out in the parent so that a child forked by
/// `VmmChild::spawn` execs it with exactly what spec 001 §4.2 names: argv
/// `[helper.path, "--runtime-fd=3"]`, an empty environment, descriptors 0
/// and 1 on `/dev/null`, 2 on the diagnostic pipe, 3 on a pipe that holds the
/// recipe's one ndjson line and then end of file, and no other descriptor.
///
/// Get one from [`HelperExec::prepare`] and pass it to `VmmChild::spawn`,
/// which forks, starts a new session and runs `launch` in the child. Never
/// call `launch` yourself: in the calling process it replaces descriptors 0
/// to 3, closes every other descriptor and execs the helper in place of the
/// caller. Dropping it closes the parent's copies of what the child would
/// have received, which `VmmChild::spawn` does after the fork.
///
/// The child resets every signal to its default action and unblocks them
/// all, sets umask 077 and the working directory `/`, and execs. It
/// allocates nothing and takes no lock, so the parent may have other
/// threads. It exits [`SETUP_FAILED`] when its setup fails and
/// [`EXEC_FAILED`] when `execve` returns; both differ from the helper's own
/// codes, 64 to 69 and 86, and from `VmmChild`'s 70 and 71.
pub struct HelperExec {
    program: CString,
    dev_null: OwnedFd,
    diagnostics_write: OwnedFd,
    recipe_read: OwnedFd,
    closer: Closer,
}

/// A helper ready to launch, and the read end of its diagnostic pipe.
#[derive(Debug)]
pub struct PreparedHelper {
    /// Pass to `VmmChild::spawn`.
    pub exec: HelperExec,
    /// The read end of the pipe the helper gets as descriptor 2, close-on-exec
    /// and nonblocking. It reaches end of file once the helper and everything
    /// it gave descriptor 2 to have exited or closed it. Keep reading it: a
    /// helper that fills the pipe blocks on its next diagnostic.
    pub diagnostics: OwnedFd,
}

/// Why a helper could not be prepared. Nothing was forked, and every
/// descriptor the preparation made is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchRefusal {
    /// Opening or reading the status of `/dev/null` failed with `errno`.
    DevNull { errno: i32 },
    /// `/dev/null` opened, but it is not the character device that
    /// `/dev/null` names.
    NotDevNull,
    /// Creating the two pipes and moving their ends above descriptor 9 failed
    /// with `errno`.
    Pipe { errno: i32 },
    /// A pipe end was still inheritable when creation returned, so every end
    /// was closed.
    PipeInheritable,
    /// The call `op` on a descriptor failed with `errno`.
    Descriptor { op: &'static str, errno: i32 },
    /// The recipe pipe took `accepted` of the `bytes` of the recipe line
    /// without blocking.
    RecipeExceedsPipe { bytes: usize, accepted: usize },
    /// The child could not be given a way to close every descriptor above 3:
    /// on Linux the kernel refused `close_range`, and on Darwin
    /// `kern.maxfilesperproc` could not be read; `errno` says why.
    CloseLimit { errno: i32 },
}

impl HelperExec {
    /// Lays out the exec of `recipe`'s helper. Every allocation and every
    /// call that can fail happens here, in the parent.
    ///
    /// It opens `/dev/null`, creates the diagnostic and recipe pipes holding
    /// the write side of the spawn lock, moves every descriptor it keeps to
    /// 10 or above, writes `recipe.to_ndjson()` into the recipe pipe without
    /// blocking and closes that pipe's write end, so the helper reads the
    /// line and then end of file. A line the pipe cannot take whole refuses
    /// with [`LaunchRefusal::RecipeExceedsPipe`]. A pipe takes about 64 KiB
    /// on the supported hosts, so this also keeps the line within the
    /// 1,048,576 bytes, newline included, that the helper reads, the bound
    /// `RuntimeRecipe::to_ndjson` leaves to its launcher.
    ///
    /// It does not open, check or hash the helper or any artifact: verify
    /// them before preparing. On Darwin, run no fork outside
    /// `VmmChild::spawn` while it runs, since a pipe end exists for a moment
    /// before it is close-on-exec and only `VmmChild::spawn` waits for that.
    pub fn prepare(recipe: &RuntimeRecipe) -> Result<PreparedHelper, LaunchRefusal> {
        let closer = Closer::prepare()?;
        let line = recipe.to_ndjson();
        let dev_null = moved_above_stdio(open_null(c"/dev/null")?).map_err(|errno| {
            LaunchRefusal::Descriptor {
                op: "F_DUPFD_CLOEXEC",
                errno,
            }
        })?;
        let [diagnostics, diagnostics_write, recipe_read, recipe_write] =
            crate::spawn_lock::with_each_descriptor_held(create_pipes).map_err(|error| {
                error
                    .raw_os_error()
                    .map_or(LaunchRefusal::PipeInheritable, |errno| {
                        LaunchRefusal::Pipe { errno }
                    })
            })?;
        set_nonblocking(&diagnostics)?;
        set_nonblocking(&recipe_write)?;
        preload(recipe_write, &line)?;
        Ok(PreparedHelper {
            exec: HelperExec {
                program: recipe.helper().path().as_c_str().to_owned(),
                dev_null,
                diagnostics_write,
                recipe_read,
                closer,
            },
            diagnostics,
        })
    }

    #[allow(unsafe_code)]
    fn install_descriptors(&self) -> bool {
        let dev_null = self.dev_null.as_raw_fd();
        [
            (dev_null, libc::STDIN_FILENO),
            (dev_null, libc::STDOUT_FILENO),
            (self.diagnostics_write.as_raw_fd(), libc::STDERR_FILENO),
            (self.recipe_read.as_raw_fd(), 3),
        ]
        .into_iter()
        .all(|(source, target)| {
            // SAFETY: dup2 takes integers only and is async-signal-safe. Every source is 10 or
            // above and this HelperExec keeps it open, so no dup2 here closes a source a later
            // one still needs, and each one copies, which clears close-on-exec on the target.
            unsafe { libc::dup2(source, target) == target }
        })
    }

    #[allow(unsafe_code)]
    fn exec(&self) -> ! {
        let argv = [
            self.program.as_ptr(),
            RUNTIME_FD_FLAG.as_ptr(),
            std::ptr::null(),
        ];
        let envp: [*const libc::c_char; 1] = [std::ptr::null()];
        // SAFETY: `argv` and `envp` are null-terminated local arrays whose strings this HelperExec
        // and a static own, so execve reads only live, terminated memory. execve and _exit are
        // async-signal-safe.
        unsafe {
            libc::execve(self.program.as_ptr(), argv.as_ptr(), envp.as_ptr());
            libc::_exit(EXEC_FAILED)
        }
    }
}

impl Launch for HelperExec {
    #[allow(unsafe_code)]
    fn launch(self) -> ! {
        let ready = default_signal_actions()
            && self.install_descriptors()
            && self.closer.close_above_three()
            && enter_root()
            && unblock_every_signal();
        if ready {
            self.exec()
        }
        // SAFETY: _exit is async-signal-safe and ends the forked child without running a Rust
        // destructor.
        unsafe { libc::_exit(SETUP_FAILED) }
    }
}

#[allow(unsafe_code)]
fn unblock_every_signal() -> bool {
    // SAFETY: All zeroes is a valid sigset_t, which sigemptyset then empties. The set is a local
    // that outlives the calls. sigemptyset and sigprocmask are async-signal-safe, and the forked
    // child has one thread, so sigprocmask sets its whole mask.
    unsafe {
        let mut empty: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut empty) == 0
            && libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut()) == 0
    }
}

#[allow(unsafe_code)]
fn default_signal_actions() -> bool {
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

#[allow(unsafe_code)]
fn enter_root() -> bool {
    // SAFETY: umask takes an integer and cannot fail. chdir reads a static NUL-terminated string
    // and keeps no pointer to it. Both are async-signal-safe.
    unsafe {
        libc::umask(0o077);
        libc::chdir(c"/".as_ptr()) == 0
    }
}

#[derive(Clone, Copy)]
struct Closer {
    #[cfg(not(target_os = "linux"))]
    limit: libc::c_int,
}

impl Closer {
    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    fn prepare() -> Result<Closer, LaunchRefusal> {
        // SAFETY: close_range takes integers only. The range starts at the highest descriptor
        // number, which no descriptor can have, so the call closes nothing and only shows that
        // the kernel offers close_range.
        let probed = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                libc::c_uint::MAX,
                libc::c_uint::MAX,
                0 as libc::c_uint,
            )
        };
        if probed == 0 {
            Ok(Closer {})
        } else {
            Err(LaunchRefusal::CloseLimit { errno: errno() })
        }
    }

    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    fn close_above_three(self) -> bool {
        // SAFETY: close_range takes integers only and is a raw system call, so it allocates
        // nothing. prepare showed the kernel offers it.
        unsafe {
            libc::syscall(
                libc::SYS_close_range,
                4 as libc::c_uint,
                libc::c_uint::MAX,
                0 as libc::c_uint,
            ) == 0
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[allow(unsafe_code)]
    fn prepare() -> Result<Closer, LaunchRefusal> {
        let mut per_process: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>();
        // SAFETY: The name is a static NUL-terminated string. `per_process` and `size` are locals
        // that sysctlbyname may write, `size` holds the bytes `per_process` has, and no new value
        // is passed.
        let read = unsafe {
            libc::sysctlbyname(
                c"kern.maxfilesperproc".as_ptr(),
                (&mut per_process as *mut libc::c_int).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 {
            return Err(LaunchRefusal::CloseLimit { errno: errno() });
        }
        // SAFETY: getdtablesize takes no arguments.
        let table = unsafe { libc::getdtablesize() };
        Ok(Closer {
            limit: table.max(per_process),
        })
    }

    #[cfg(not(target_os = "linux"))]
    #[allow(unsafe_code)]
    fn close_above_three(self) -> bool {
        for fd in 4..self.limit {
            // SAFETY: close takes an integer and is async-signal-safe. Every descriptor above 3
            // is one the helper must not inherit, whoever made it.
            unsafe { libc::close(fd) };
        }
        true
    }
}

#[allow(unsafe_code)]
fn open_null(path: &CStr) -> Result<OwnedFd, LaunchRefusal> {
    // SAFETY: `path` is a NUL-terminated string the caller keeps alive for the call, and open
    // keeps no pointer to it.
    let raw = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if raw == -1 {
        return Err(LaunchRefusal::DevNull { errno: errno() });
    }
    // SAFETY: open returned a new descriptor, and nothing else owns it.
    let opened = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: libc::stat is a plain C struct, for which all zeroes is a valid value.
    let mut found: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `found` is a local stat that fstat may write, and `opened` keeps its descriptor
    // open.
    if unsafe { libc::fstat(opened.as_raw_fd(), &mut found) } == -1 {
        return Err(LaunchRefusal::DevNull { errno: errno() });
    }
    // SAFETY: libc::stat is a plain C struct, for which all zeroes is a valid value.
    let mut named: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: The path is a static NUL-terminated string, `named` is a local stat that stat may
    // write, and stat keeps no pointer to either.
    if unsafe { libc::stat(c"/dev/null".as_ptr(), &mut named) } == -1 {
        return Err(LaunchRefusal::DevNull { errno: errno() });
    }
    if found.st_mode & libc::S_IFMT == libc::S_IFCHR && found.st_rdev == named.st_rdev {
        Ok(opened)
    } else {
        Err(LaunchRefusal::NotDevNull)
    }
}

fn create_pipes(held: &DescriptorsHeld) -> std::io::Result<[OwnedFd; 4]> {
    let [diagnostics, diagnostics_write] = pipe_above_stdio(held)?;
    let [recipe_read, recipe_write] = pipe_above_stdio(held)?;
    Ok([diagnostics, diagnostics_write, recipe_read, recipe_write])
}

#[allow(unsafe_code)]
fn pipe_above_stdio(_held: &DescriptorsHeld) -> std::io::Result<[OwnedFd; 2]> {
    let mut ends: [RawFd; 2] = [-1; 2];
    #[cfg(target_os = "linux")]
    // SAFETY: `ends` is a local array of two c_int, the size pipe2 writes.
    let made = unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) };
    #[cfg(not(target_os = "linux"))]
    // SAFETY: `ends` is a local array of two c_int, the size pipe writes.
    let made = unsafe { libc::pipe(ends.as_mut_ptr()) };
    if made == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: pipe returned 0, so both entries are new descriptors, and nothing else owns them.
    let [read, write] = ends.map(|end| unsafe { OwnedFd::from_raw_fd(end) });
    let moved = |fd| moved_above_stdio(fd).map_err(std::io::Error::from_raw_os_error);
    Ok([moved(read)?, moved(write)?])
}

#[allow(unsafe_code)]
fn moved_above_stdio(fd: OwnedFd) -> Result<OwnedFd, i32> {
    // SAFETY: fcntl with F_DUPFD_CLOEXEC takes integers only, and `fd` keeps its descriptor open
    // for the call.
    match unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, FIRST_KEPT) } {
        -1 => Err(errno()),
        // SAFETY: fcntl returned a new descriptor, and nothing else owns it.
        moved => Ok(unsafe { OwnedFd::from_raw_fd(moved) }),
    }
}

#[allow(unsafe_code)]
fn set_nonblocking(fd: &OwnedFd) -> Result<(), LaunchRefusal> {
    // SAFETY: fcntl with F_GETFL takes no pointer, and `fd` keeps its descriptor open.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags == -1 {
        return Err(LaunchRefusal::Descriptor {
            op: "F_GETFL",
            errno: errno(),
        });
    }
    // SAFETY: fcntl with F_SETFL takes an integer flag set, not a pointer, and `fd` keeps its
    // descriptor open.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(LaunchRefusal::Descriptor {
            op: "F_SETFL",
            errno: errno(),
        });
    }
    Ok(())
}

#[allow(unsafe_code)]
fn preload(write_end: OwnedFd, line: &[u8]) -> Result<(), LaunchRefusal> {
    let mut accepted = 0;
    while let Some(rest) = line.get(accepted..).filter(|rest| !rest.is_empty()) {
        // SAFETY: `rest` borrows `rest.len()` live bytes of `line`, and write reads at most that many.
        // `write_end` keeps its descriptor open.
        let wrote = unsafe { libc::write(write_end.as_raw_fd(), rest.as_ptr().cast(), rest.len()) };
        match usize::try_from(wrote) {
            Ok(count) if count > 0 => accepted += count,
            Ok(_) => {
                return Err(LaunchRefusal::RecipeExceedsPipe {
                    bytes: line.len(),
                    accepted,
                });
            }
            Err(_) => match errno() {
                libc::EINTR => {}
                libc::EAGAIN => {
                    return Err(LaunchRefusal::RecipeExceedsPipe {
                        bytes: line.len(),
                        accepted,
                    });
                }
                errno => return Err(LaunchRefusal::Descriptor { op: "write", errno }),
            },
        }
    }
    Ok(())
}

fn errno() -> i32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

impl std::fmt::Debug for HelperExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HelperExec")
            .field("program", &self.program)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for LaunchRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchRefusal::DevNull { errno } => {
                write!(f, "/dev/null could not be opened: errno {errno}")
            }
            LaunchRefusal::NotDevNull => f.write_str("/dev/null is not the null character device"),
            LaunchRefusal::Pipe { errno } => {
                write!(f, "the helper's pipes could not be created: errno {errno}")
            }
            LaunchRefusal::PipeInheritable => f.write_str(
                "a pipe end was still inheritable when it was created, so every end was closed",
            ),
            LaunchRefusal::Descriptor { op, errno } => {
                write!(f, "{op} on a helper descriptor failed: errno {errno}")
            }
            LaunchRefusal::RecipeExceedsPipe { bytes, accepted } => write!(
                f,
                "the recipe pipe took {accepted} of the recipe line's {bytes} bytes"
            ),
            LaunchRefusal::CloseLimit { errno } => write!(
                f,
                "the child cannot close every descriptor above 3: errno {errno}"
            ),
        }
    }
}

impl std::error::Error for LaunchRefusal {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_fixtures::{artifact, recipe, recipe_value};
    use crate::vmm::{VmmChild, VmmState};
    use std::collections::BTreeMap;
    use std::io::{Read, Seek, SeekFrom};
    use std::os::fd::AsRawFd;
    use std::process::Stdio;
    use std::sync::LazyLock;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::{Duration, Instant};

    mod fake {
        include!("../../tests/support/runtime_fake.rs");
    }

    const DEADLINE: Duration = Duration::from_secs(10);
    const HOLD: Duration = Duration::from_millis(250);
    const PROBE: &str = "PLASMOSOME_LAUNCH_PROBE";
    const HIGH_TARGET: libc::c_int = 200_000;
    const ROOM_FOR_CONCURRENT_HIGH_DESCRIPTORS: libc::c_int = 256;
    const HELPER_LINE_BOUND: usize = 1_048_576;

    struct Report {
        text: String,
        exit: VmmState,
    }

    impl Report {
        fn value(&self, key: &str) -> Option<&str> {
            self.text
                .lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        }

        fn values(&self, key: &str) -> Vec<&str> {
            self.text
                .lines()
                .filter_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                .collect()
        }

        fn descriptors(&self) -> BTreeMap<i32, &str> {
            self.text
                .lines()
                .filter_map(|line| {
                    let (number, attributes) = line.strip_prefix("fd=")?.split_once(' ')?;
                    Some((number.parse().ok()?, attributes))
                })
                .collect()
        }

        fn assert_exited_cleanly(&self) {
            assert_eq!(
                self.exit,
                VmmState::Exited { code: 0 },
                "the record fake exits 0; its report:\n{}",
                self.text
            );
        }
    }

    struct Recorded {
        recipe: RuntimeRecipe,
        report: Report,
        inherited: [libc::c_int; 2],
    }

    fn record_recipe() -> RuntimeRecipe {
        let helper = fake::runtime_fake("record");
        recipe(&recipe_value(
            helper.to_str().expect("the target directory is UTF-8"),
        ))
    }

    fn launched(prepared: PreparedHelper) -> Report {
        let PreparedHelper { exec, diagnostics } = prepared;
        let mut child = VmmChild::spawn(exec).expect("the fork succeeds");
        let text = read_to_end_of_file(&diagnostics, DEADLINE);
        let exit = child
            .wait_terminal(DEADLINE)
            .expect("the helper child is supervised");
        Report { text, exit }
    }

    fn record() -> Recorded {
        let recipe = record_recipe();
        let inherited = Inheritable::low_and_high();
        let _blocked = SignalBlocked::on_this_thread(libc::SIGUSR2);
        let prepared = HelperExec::prepare(&recipe).expect("the record recipe prepares");
        let report = launched(prepared);
        Recorded {
            recipe,
            report,
            inherited: inherited.0,
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn errno() -> i32 {
        std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EIO)
    }

    #[allow(unsafe_code)]
    fn read_to_end_of_file(fd: &OwnedFd, patience: Duration) -> String {
        let started = Instant::now();
        let mut bytes = Vec::new();
        loop {
            let Some(left) = patience.checked_sub(started.elapsed()) else {
                panic!(
                    "the diagnostics reach end of file within {patience:?}; read so far:\n{}",
                    String::from_utf8_lossy(&bytes)
                );
            };
            let mut watched = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let wait_ms = libc::c_int::try_from(left.as_millis().min(100)).unwrap_or(100);
            // SAFETY: `watched` is one local pollfd, matching the count of 1, and `fd` keeps its
            // descriptor open.
            if unsafe { libc::poll(&mut watched, 1, wait_ms) } <= 0 {
                continue;
            }
            let mut chunk = [0u8; 4096];
            // SAFETY: `chunk` is a local buffer of `chunk.len()` bytes that read may write, and
            // `fd` keeps its descriptor open.
            let got = unsafe { libc::read(fd.as_raw_fd(), chunk.as_mut_ptr().cast(), chunk.len()) };
            match usize::try_from(got) {
                Ok(0) => return String::from_utf8_lossy(&bytes).into_owned(),
                Ok(count) => bytes.extend_from_slice(&chunk[..count]),
                Err(_) => {
                    let errno = errno();
                    assert!(
                        errno == libc::EAGAIN || errno == libc::EINTR,
                        "reading the diagnostics fails only while they are empty, not with errno {errno}"
                    );
                }
            }
        }
    }

    #[allow(unsafe_code)]
    fn high_descriptor_number() -> libc::c_int {
        static HIGH: LazyLock<libc::c_int> = LazyLock::new(|| {
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            assert_eq!(
                // SAFETY: `limit` is a local rlimit that getrlimit may write.
                unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
                0
            );
            let wanted = HIGH_TARGET as libc::rlim_t + 1;
            if limit.rlim_cur < wanted {
                limit.rlim_cur = wanted.min(limit.rlim_max);
                // SAFETY: `limit` is a local rlimit that setrlimit only reads. Raising the soft
                // limit only lets this test process open more descriptors.
                unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
            }
            // SAFETY: getdtablesize takes no arguments.
            let high = HIGH_TARGET
                .min(unsafe { libc::getdtablesize() } - ROOM_FOR_CONCURRENT_HIGH_DESCRIPTORS);
            assert!(
                high > 1024,
                "this host lets the test open a descriptor above 1024, where a launcher that closes only 4 to 1023 leaves it open; the highest it allows is {high}"
            );
            high
        });
        *HIGH
    }

    struct Inheritable([libc::c_int; 2]);

    impl Inheritable {
        fn low_and_high() -> Inheritable {
            Inheritable([
                inheritable_null_from(4),
                inheritable_null_from(high_descriptor_number()),
            ])
        }
    }

    impl Drop for Inheritable {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            for fd in self.0 {
                // SAFETY: `fd` is a descriptor this Inheritable opened and owns, closed once here.
                unsafe { libc::close(fd) };
            }
        }
    }

    #[allow(unsafe_code)]
    fn inheritable_null_from(lowest: libc::c_int) -> libc::c_int {
        // SAFETY: The path is a static NUL-terminated string, and open keeps no pointer to it.
        let opened = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
        assert!(opened >= 0, "/dev/null opens");
        // SAFETY: fcntl with F_DUPFD takes an integer, and `opened` is open for the call.
        let moved = unsafe { libc::fcntl(opened, libc::F_DUPFD, lowest) };
        // SAFETY: `opened` is the descriptor opened above, which nothing else owns.
        unsafe { libc::close(opened) };
        assert!(
            moved >= lowest,
            "/dev/null is copied to {lowest} or above without close-on-exec"
        );
        moved
    }

    struct SignalBlocked(libc::sigset_t);

    impl SignalBlocked {
        #[allow(unsafe_code)]
        fn on_this_thread(signal: libc::c_int) -> SignalBlocked {
            // SAFETY: All zeroes is a valid sigset_t, and sigemptyset then sets both. They are
            // locals that outlive the calls, and pthread_sigmask changes only this thread's mask.
            unsafe {
                let mut previous: libc::sigset_t = std::mem::zeroed();
                let mut blocked: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut blocked);
                libc::sigaddset(&mut blocked, signal);
                assert_eq!(
                    libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut previous),
                    0
                );
                SignalBlocked(previous)
            }
        }
    }

    impl Drop for SignalBlocked {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: `self.0` is the mask saved when the signal was blocked, which
            // pthread_sigmask only reads.
            unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &self.0, std::ptr::null_mut()) };
        }
    }

    #[allow(unsafe_code)]
    fn this_process_ignores(signal: libc::c_int) -> bool {
        // SAFETY: libc::sigaction is a plain C struct, for which all zeroes is a valid value.
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        // SAFETY: A null new action only reads the disposition into `current`, a local.
        unsafe { libc::sigaction(signal, std::ptr::null(), &mut current) };
        current.sa_sigaction == libc::SIG_IGN
    }

    #[allow(unsafe_code)]
    fn is_close_on_exec(fd: &OwnedFd) -> bool {
        // SAFETY: fcntl with F_GETFD takes no pointer, and `fd` keeps its descriptor open.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
        flags != -1 && flags & libc::FD_CLOEXEC != 0
    }

    fn recipe_of_line_length(length: usize) -> RuntimeRecipe {
        let helper = fake::runtime_fake("record");
        let mut value = recipe_value(helper.to_str().expect("the target directory is UTF-8"));
        let mut left = length - recipe(&value).to_ndjson().len();
        let mut libraries = vec![value["libraries"][0].clone()];
        let full = 88 + 1023;
        let mut index = 0;
        while left > 0 {
            let take = if left >= 2 * full {
                full
            } else if left > full {
                left / 2
            } else {
                left
            };
            let prefix = format!("/opt/cell/lib/{index:06}/");
            let path = prefix.clone() + &"x".repeat(take - 88 - prefix.len());
            libraries.push(artifact(&path, '5'));
            left -= take;
            index += 1;
        }
        value["libraries"] = serde_json::Value::Array(libraries);
        let built = recipe(&value);
        assert_eq!(
            built.to_ndjson().len(),
            length,
            "the recipe line is {length} bytes"
        );
        built
    }

    fn open_descriptor_count() -> usize {
        std::fs::read_dir("/dev/fd")
            .expect("this process's descriptors can be listed")
            .count()
    }

    fn run_probe(test: &str) {
        let output = tempfile::tempfile().expect("the probe has a file for its output");
        let mut command = std::process::Command::new(
            std::env::current_exe().expect("the test executable has a path"),
        );
        command
            .args([
                "--exact",
                test,
                "--ignored",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(PROBE, "1")
            .stdin(Stdio::null())
            .stdout(output.try_clone().expect("the output file is shared"))
            .stderr(output.try_clone().expect("the output file is shared"));
        let spawned = {
            let _no_pipe_mid_creation = crate::spawn_lock::hold_fork_side();
            command.spawn()
        };
        let mut child = spawned.expect("the probe process starts");
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().expect("the probe's status is readable") {
                break Some(status);
            }
            if started.elapsed() >= DEADLINE {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut said = String::new();
        let mut output = output;
        let _ = output
            .seek(SeekFrom::Start(0))
            .and_then(|_| output.read_to_string(&mut said));
        match status {
            Some(status) => assert!(status.success(), "{test} fails: {status}\n{said}"),
            None => panic!("{test} did not finish within {DEADLINE:?}\n{said}"),
        }
    }

    fn probing() -> bool {
        std::env::var_os(PROBE).is_some()
    }

    #[test]
    fn the_helper_gets_exactly_its_path_and_the_runtime_fd_argument() {
        let recorded = record();
        recorded.report.assert_exited_cleanly();
        let report = &recorded.report;
        assert_eq!(report.value("argc"), Some("2"), "{}", report.text);
        assert_eq!(
            report.value("argv0"),
            Some(recorded.recipe.helper().path().as_str())
        );
        assert_eq!(report.value("argv1"), Some("--runtime-fd=3"));
    }

    #[test]
    fn the_helper_starts_with_an_empty_environment() {
        assert!(
            std::env::vars_os().next().is_some(),
            "the test process has an environment the helper could inherit"
        );
        let recorded = record();
        recorded.report.assert_exited_cleanly();
        assert_eq!(recorded.report.value("envc"), Some("0"));
        assert_eq!(recorded.report.value("env_name"), None);
    }

    #[test]
    fn descriptors_zero_to_three_are_read_write_null_a_diagnostics_writer_and_a_recipe_reader() {
        let recorded = record();
        recorded.report.assert_exited_cleanly();
        let descriptors = recorded.report.descriptors();
        assert_eq!(
            descriptors.get(&0),
            Some(&"kind=chr null=1 nonblock=0 access=rw")
        );
        assert_eq!(
            descriptors.get(&1),
            Some(&"kind=chr null=1 nonblock=0 access=rw")
        );
        assert_eq!(
            descriptors.get(&2),
            Some(&"kind=fifo null=0 nonblock=0 access=w")
        );
        assert_eq!(
            descriptors.get(&3),
            Some(&"kind=fifo null=0 nonblock=0 access=r")
        );
    }

    #[test]
    fn no_descriptor_above_three_survives_the_exec() {
        let recorded = record();
        let [low, high] = recorded.inherited;
        assert!(
            low > 3 && high > 1024,
            "the parent held inheritable descriptors {low} and {high} across the fork"
        );
        recorded.report.assert_exited_cleanly();
        let open: Vec<i32> = recorded.report.descriptors().into_keys().collect();
        assert_eq!(open, [0, 1, 2, 3], "{}", recorded.report.text);
    }

    #[test]
    fn a_descriptor_above_a_lowered_soft_limit_does_not_survive_the_exec() {
        run_probe("runtime::launch::tests::probe_launch_under_a_lowered_soft_limit");
    }

    #[test]
    #[ignore = "helper process for a_descriptor_above_a_lowered_soft_limit_does_not_survive_the_exec"]
    #[allow(unsafe_code)]
    fn probe_launch_under_a_lowered_soft_limit() {
        if !probing() {
            return;
        }
        let fitting = record_recipe();
        let inherited = Inheritable::low_and_high();
        let high = inherited.0[1];
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            // SAFETY: `limit` is a local rlimit that getrlimit may write.
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
            0
        );
        limit.rlim_cur = 1024;
        assert_eq!(
            // SAFETY: `limit` is a local rlimit that setrlimit only reads. This probe process runs
            // only this test, so a lower limit reaches no other test.
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) },
            0
        );
        // SAFETY: getdtablesize takes no arguments.
        let table = unsafe { libc::getdtablesize() };
        assert!(
            table <= 1024 && high >= table,
            "the probe holds descriptor {high} at or above its descriptor table of {table}"
        );
        let report = launched(HelperExec::prepare(&fitting).expect("the record recipe prepares"));
        report.assert_exited_cleanly();
        let open: Vec<i32> = report.descriptors().into_keys().collect();
        assert_eq!(open, [0, 1, 2, 3], "{}", report.text);
    }

    #[test]
    fn descriptor_three_yields_the_canonical_recipe_line_then_end_of_file() {
        let recorded = record();
        recorded.report.assert_exited_cleanly();
        let expected = hex(&recorded.recipe.to_ndjson());
        assert_eq!(recorded.report.value("recipe_hex"), Some(expected.as_str()));
        assert_eq!(recorded.report.value("recipe_eof"), Some("1"));
    }

    #[test]
    fn the_helper_starts_in_root_with_umask_077_default_signals_and_its_own_session() {
        assert!(
            this_process_ignores(libc::SIGPIPE),
            "the test harness ignores SIGPIPE, which an exec keeps unless the child resets it"
        );
        let recorded = record();
        recorded.report.assert_exited_cleanly();
        let report = &recorded.report;
        assert_eq!(report.value("cwd"), Some("/"));
        assert_eq!(report.value("umask"), Some("077"));
        assert_eq!(
            report.value("sigmask_empty"),
            Some("1"),
            "the forking thread blocked SIGUSR2"
        );
        assert_eq!(
            report.values("ignored"),
            Vec::<&str>::new(),
            "the helper ignores no signal"
        );
        assert_eq!(report.value("sid_is_self"), Some("1"));
    }

    #[test]
    fn every_signal_the_parent_ignored_reaches_the_helper_at_its_default() {
        run_probe("runtime::launch::tests::probe_launch_with_signals_ignored");
    }

    #[test]
    #[ignore = "helper process for every_signal_the_parent_ignored_reaches_the_helper_at_its_default"]
    #[allow(unsafe_code)]
    fn probe_launch_with_signals_ignored() {
        if !probing() {
            return;
        }
        for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM] {
            // SAFETY: signal takes integers and SIG_IGN. This probe process runs only this test,
            // so ignoring these signals reaches no other test.
            let previous = unsafe { libc::signal(signal, libc::SIG_IGN) };
            assert!(
                previous != libc::SIG_ERR && this_process_ignores(signal),
                "the probe ignores signal {signal} before it launches"
            );
        }
        let recorded = record();
        recorded.report.assert_exited_cleanly();
        assert_eq!(
            recorded.report.values("ignored"),
            Vec::<&str>::new(),
            "{}",
            recorded.report.text
        );
    }

    #[test]
    fn a_recipe_line_longer_than_the_helper_reads_refuses_without_blocking() {
        let oversized = recipe_of_line_length(HELPER_LINE_BOUND + 1);
        let (sent, received) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sent.send(HelperExec::prepare(&oversized).err());
        });
        match received.recv_timeout(Duration::from_secs(5)) {
            Ok(Some(LaunchRefusal::RecipeExceedsPipe { bytes, accepted })) => {
                assert_eq!(bytes, HELPER_LINE_BOUND + 1);
                assert!(
                    accepted < bytes,
                    "the pipe took {accepted} of {bytes} bytes"
                );
            }
            Ok(other) => panic!("a line the pipe cannot take refuses, got {other:?}"),
            Err(RecvTimeoutError::Timeout) => {
                panic!("prepare blocked writing a line the pipe cannot take")
            }
            Err(RecvTimeoutError::Disconnected) => panic!("prepare panicked"),
        }
    }

    #[test]
    fn a_refused_or_dropped_preparation_leaves_no_descriptor_open() {
        run_probe("runtime::launch::tests::probe_descriptors_around_a_refusal");
    }

    #[test]
    #[ignore = "helper process for a_refused_or_dropped_preparation_leaves_no_descriptor_open"]
    fn probe_descriptors_around_a_refusal() {
        if !probing() {
            return;
        }
        let oversized = recipe_of_line_length(HELPER_LINE_BOUND + 1);
        let fitting = record_recipe();
        let before = open_descriptor_count();
        assert!(matches!(
            HelperExec::prepare(&oversized),
            Err(LaunchRefusal::RecipeExceedsPipe { .. })
        ));
        assert_eq!(
            open_descriptor_count(),
            before,
            "a refused preparation closed every descriptor it made"
        );
        drop(HelperExec::prepare(&fitting).expect("the record recipe prepares"));
        assert_eq!(
            open_descriptor_count(),
            before,
            "a dropped preparation closed every descriptor it made"
        );
    }

    #[test]
    fn a_helper_that_cannot_be_executed_exits_the_child_with_127() {
        let dir = tempfile::tempdir().expect("the test owns a directory");
        let helper = dir.path().join("not-executable");
        std::fs::write(&helper, "this is not a program\n").expect("the test writes a plain file");
        let unrunnable = recipe(&recipe_value(
            helper.to_str().expect("the temporary path is UTF-8"),
        ));
        let report = launched(HelperExec::prepare(&unrunnable).expect("the recipe prepares"));
        assert_eq!(report.exit, VmmState::Exited { code: EXEC_FAILED });
        assert_eq!(report.text, "");
    }

    #[test]
    fn a_child_whose_setup_fails_exits_with_125_and_never_execs() {
        run_probe("runtime::launch::tests::probe_launch_with_descriptor_three_out_of_range");
    }

    #[test]
    #[ignore = "helper process for a_child_whose_setup_fails_exits_with_125_and_never_execs"]
    #[allow(unsafe_code)]
    fn probe_launch_with_descriptor_three_out_of_range() {
        if !probing() {
            return;
        }
        let prepared = HelperExec::prepare(&record_recipe()).expect("the record recipe prepares");
        let PreparedHelper { exec, diagnostics } = prepared;
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            // SAFETY: `limit` is a local rlimit that getrlimit may write.
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
            0
        );
        let restored = limit;
        limit.rlim_cur = 3;
        assert_eq!(
            // SAFETY: `limit` is a local rlimit that setrlimit only reads. This probe process runs
            // only this test, and the fork below opens no descriptor, so the child inherits a
            // limit under which descriptor 3 cannot be made.
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) },
            0
        );
        let spawned = VmmChild::spawn(exec);
        assert_eq!(
            // SAFETY: `restored` is a local rlimit that setrlimit only reads.
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &restored) },
            0
        );
        let mut child = spawned.expect("the fork succeeds");
        let text = read_to_end_of_file(&diagnostics, DEADLINE);
        let exit = child
            .wait_terminal(DEADLINE)
            .expect("the helper child is supervised");
        assert_eq!(exit, VmmState::Exited { code: SETUP_FAILED }, "{text}");
        assert_eq!(text, "", "the helper never ran");
    }

    #[test]
    fn the_diagnostics_reach_end_of_file_once_the_helper_has_exited() {
        let prepared = HelperExec::prepare(&record_recipe()).expect("the record recipe prepares");
        let PreparedHelper { exec, diagnostics } = prepared;
        let mut child = VmmChild::spawn(exec).expect("the fork succeeds");
        assert_eq!(
            child.wait_terminal(DEADLINE),
            Ok(VmmState::Exited { code: 0 })
        );
        let text = read_to_end_of_file(&diagnostics, Duration::from_secs(1));
        assert!(text.contains("recipe_eof=1\n"), "{text}");
    }

    #[test]
    #[allow(unsafe_code)]
    fn the_diagnostics_read_end_is_nonblocking() {
        let prepared = HelperExec::prepare(&record_recipe()).expect("the record recipe prepares");
        // SAFETY: fcntl with F_GETFL takes no pointer, and `prepared` keeps the descriptor open.
        let flags = unsafe { libc::fcntl(prepared.diagnostics.as_raw_fd(), libc::F_GETFL) };
        assert!(
            flags != -1 && flags & libc::O_NONBLOCK != 0,
            "a caller can poll the diagnostics without a read that blocks"
        );
    }

    #[test]
    fn preparing_waits_while_a_fork_holds_the_spawn_lock() {
        let fitting = record_recipe();
        let held = crate::spawn_lock::hold_fork_side();
        let (sent, received) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sent.send(HelperExec::prepare(&fitting).map(drop));
        });
        let early = received.recv_timeout(HOLD);
        drop(held);
        assert!(
            matches!(early, Err(RecvTimeoutError::Timeout)),
            "prepare created its pipes while a fork held the spawn lock: {early:?}"
        );
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)),
            Ok(Ok(())),
            "prepare finishes once the fork releases the lock"
        );
    }

    #[test]
    fn every_kept_descriptor_is_close_on_exec_and_above_nine_even_with_stdin_closed() {
        run_probe("runtime::launch::tests::probe_prepare_with_stdin_closed");
    }

    #[test]
    #[ignore = "helper process for every_kept_descriptor_is_close_on_exec_and_above_nine_even_with_stdin_closed"]
    #[allow(unsafe_code)]
    fn probe_prepare_with_stdin_closed() {
        if !probing() {
            return;
        }
        let fitting = record_recipe();
        // SAFETY: close takes no pointer. This probe process runs only this test, and nothing in
        // it reads standard input.
        unsafe { libc::close(0) };
        let prepared = HelperExec::prepare(&fitting).expect("the record recipe prepares");
        for (name, fd) in [
            ("/dev/null", &prepared.exec.dev_null),
            ("diagnostics write end", &prepared.exec.diagnostics_write),
            ("recipe read end", &prepared.exec.recipe_read),
            ("diagnostics read end", &prepared.diagnostics),
        ] {
            assert!(
                fd.as_raw_fd() >= 10 && is_close_on_exec(fd),
                "the {name} is descriptor {}, close-on-exec {}",
                fd.as_raw_fd(),
                is_close_on_exec(fd)
            );
        }
        let report = launched(prepared);
        report.assert_exited_cleanly();
        assert_eq!(
            report.descriptors().get(&0),
            Some(&"kind=chr null=1 nonblock=0 access=rw")
        );
    }

    #[test]
    fn a_process_out_of_descriptors_refuses_instead_of_launching() {
        run_probe("runtime::launch::tests::probe_prepare_out_of_descriptors");
    }

    #[test]
    #[ignore = "helper process for a_process_out_of_descriptors_refuses_instead_of_launching"]
    #[allow(unsafe_code)]
    fn probe_prepare_out_of_descriptors() {
        if !probing() {
            return;
        }
        let fitting = record_recipe();
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            // SAFETY: `limit` is a local rlimit that getrlimit may write.
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
            0
        );
        for (soft, refused) in [(10, "F_DUPFD_CLOEXEC"), (11, "pipe")] {
            limit.rlim_cur = soft;
            assert_eq!(
                // SAFETY: `limit` is a local rlimit that setrlimit only reads. This probe process
                // runs only this test, so a lower limit reaches no other test.
                unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) },
                0
            );
            let outcome = HelperExec::prepare(&fitting).err();
            let matched = match outcome {
                Some(LaunchRefusal::Descriptor {
                    op: "F_DUPFD_CLOEXEC",
                    ..
                }) => "F_DUPFD_CLOEXEC",
                Some(LaunchRefusal::Pipe { .. }) => "pipe",
                _ => "neither",
            };
            assert_eq!(
                matched, refused,
                "a soft limit of {soft} descriptors refuses typed, got {outcome:?}"
            );
        }
    }

    #[test]
    fn a_prepared_helper_shows_its_program_and_no_descriptor_numbers() {
        let fitting = record_recipe();
        let prepared = HelperExec::prepare(&fitting).expect("the record recipe prepares");
        let shown = format!("{:?}", prepared.exec);
        assert!(
            shown.starts_with("HelperExec { program: ")
                && shown.contains(fitting.helper().path().as_str())
                && shown.ends_with(", .. }"),
            "{shown}"
        );
    }

    #[test]
    fn only_the_null_character_device_is_accepted_as_dev_null() {
        let dir = tempfile::tempdir().expect("the test owns a directory");
        let plain = dir.path().join("null");
        std::fs::write(&plain, "").expect("the test writes a plain file");
        let plain = CString::new(plain.to_str().expect("UTF-8")).expect("no NUL");
        let missing =
            CString::new(dir.path().join("missing").to_str().expect("UTF-8")).expect("no NUL");
        assert!(open_null(c"/dev/null").is_ok());
        assert_eq!(
            open_null(c"/dev/zero").err(),
            Some(LaunchRefusal::NotDevNull)
        );
        assert_eq!(open_null(&plain).err(), Some(LaunchRefusal::NotDevNull));
        assert_eq!(
            open_null(&missing).err(),
            Some(LaunchRefusal::DevNull {
                errno: libc::ENOENT
            })
        );
    }

    #[test]
    fn each_refusal_says_what_failed() {
        for (refusal, said) in [
            (
                LaunchRefusal::DevNull { errno: 2 },
                "/dev/null could not be opened: errno 2",
            ),
            (
                LaunchRefusal::NotDevNull,
                "/dev/null is not the null character device",
            ),
            (
                LaunchRefusal::Pipe { errno: 24 },
                "the helper's pipes could not be created: errno 24",
            ),
            (
                LaunchRefusal::PipeInheritable,
                "a pipe end was still inheritable when it was created, so every end was closed",
            ),
            (
                LaunchRefusal::Descriptor {
                    op: "F_SETFL",
                    errno: 9,
                },
                "F_SETFL on a helper descriptor failed: errno 9",
            ),
            (
                LaunchRefusal::RecipeExceedsPipe {
                    bytes: 70_000,
                    accepted: 65_536,
                },
                "the recipe pipe took 65536 of the recipe line's 70000 bytes",
            ),
            (
                LaunchRefusal::CloseLimit { errno: 38 },
                "the child cannot close every descriptor above 3: errno 38",
            ),
        ] {
            assert_eq!(refusal.to_string(), said);
        }
    }
}
