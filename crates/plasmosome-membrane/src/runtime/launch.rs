use crate::runtime::recipe::RuntimeRecipe;
use crate::vmm::Launch;
use std::ffi::{CStr, CString};
use std::os::fd::OwnedFd;

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
        let _ = recipe;
        todo!()
    }
}

fn open_null(path: &CStr) -> Result<OwnedFd, LaunchRefusal> {
    let _ = path;
    todo!()
}

impl Launch for HelperExec {
    fn launch(self) -> ! {
        let _ = (
            &self.program,
            &self.dev_null,
            &self.diagnostics_write,
            &self.recipe_read,
        );
        todo!()
    }
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
        let _ = f;
        todo!()
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
            let slice = libc::c_int::try_from(left.as_millis().min(100)).unwrap_or(100);
            // SAFETY: `watched` is one local pollfd, matching the count of 1, and `fd` keeps its
            // descriptor open.
            if unsafe { libc::poll(&mut watched, 1, slice) } <= 0 {
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
            // SAFETY: `limit` is a local rlimit that getrlimit may write.
            assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) }, 0);
            let wanted = HIGH_TARGET as libc::rlim_t + 1;
            if limit.rlim_cur < wanted {
                limit.rlim_cur = wanted.min(limit.rlim_max);
                // SAFETY: `limit` is a local rlimit that setrlimit only reads. Raising the soft
                // limit only lets this test process open more descriptors.
                unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
            }
            // SAFETY: getdtablesize takes no arguments.
            let high = HIGH_TARGET.min(unsafe { libc::getdtablesize() } - 1);
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
    fn this_process_ignores_sigpipe() -> bool {
        // SAFETY: libc::sigaction is a plain C struct, for which all zeroes is a valid value.
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        // SAFETY: A null new action only reads the disposition into `current`, a local.
        unsafe { libc::sigaction(libc::SIGPIPE, std::ptr::null(), &mut current) };
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
        assert_eq!(built.to_ndjson().len(), length, "the recipe line is {length} bytes");
        built
    }

    fn open_descriptor_count() -> usize {
        std::fs::read_dir("/dev/fd")
            .expect("this process's descriptors can be listed")
            .count()
    }

    fn run_probe(test: &str) {
        let output = tempfile::tempfile().expect("the probe has a file for its output");
        let mut child = std::process::Command::new(
            std::env::current_exe().expect("the test executable has a path"),
        )
        .args(["--exact", test, "--ignored", "--test-threads=1", "--nocapture"])
        .env(PROBE, "1")
        .stdin(Stdio::null())
        .stdout(output.try_clone().expect("the output file is shared"))
        .stderr(output.try_clone().expect("the output file is shared"))
        .spawn()
        .expect("the probe process starts");
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
    fn descriptors_zero_to_three_are_null_null_diagnostics_and_a_blocking_recipe_pipe() {
        let recorded = record();
        recorded.report.assert_exited_cleanly();
        let descriptors = recorded.report.descriptors();
        assert_eq!(descriptors.get(&0), Some(&"kind=chr null=1 nonblock=0"));
        assert_eq!(descriptors.get(&1), Some(&"kind=chr null=1 nonblock=0"));
        assert_eq!(descriptors.get(&2), Some(&"kind=fifo null=0 nonblock=0"));
        assert_eq!(descriptors.get(&3), Some(&"kind=fifo null=0 nonblock=0"));
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
            this_process_ignores_sigpipe(),
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
        assert_eq!(report.value("sigpipe"), Some("default"));
        assert_eq!(report.value("sid_is_self"), Some("1"));
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
                assert!(accepted < bytes, "the pipe took {accepted} of {bytes} bytes");
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
            Some(&"kind=chr null=1 nonblock=0")
        );
    }

    #[test]
    fn only_the_null_character_device_is_accepted_as_dev_null() {
        let dir = tempfile::tempdir().expect("the test owns a directory");
        let plain = dir.path().join("null");
        std::fs::write(&plain, "").expect("the test writes a plain file");
        let plain = CString::new(plain.to_str().expect("UTF-8")).expect("no NUL");
        let missing = CString::new(dir.path().join("missing").to_str().expect("UTF-8"))
            .expect("no NUL");
        assert!(open_null(c"/dev/null").is_ok());
        assert_eq!(open_null(c"/dev/zero").err(), Some(LaunchRefusal::NotDevNull));
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
