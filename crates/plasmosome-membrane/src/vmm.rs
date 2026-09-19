use std::io::Write;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// The body of a forked VMM child.
pub trait Launch {
    /// Runs inside the forked child and must never return. The parent is
    /// multi-threaded under the test harness, so an implementation may only
    /// use async-signal-safe calls: `libc::_exit`, `libc::pause`, raw syscalls.
    ///
    /// An implementation must also never panic. Unwinding in the forked child
    /// of a multi-threaded parent runs the panic hook, which allocates and
    /// locks stderr; if another thread held that lock at fork time the child
    /// deadlocks. A panic that escapes is contained by exiting the child with
    /// code 70 instead.
    fn launch(self) -> !;
}

/// What the supervisor last observed about its VMM child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmState {
    Running,
    Exited {
        code: i32,
    },
    Signaled {
        signal: i32,
    },
    /// Another waiter consumed the direct child's status. The managed group is
    /// not signalled after this state because its numerical identity is no
    /// longer owned, so its workers may remain.
    Lost,
}

/// Which operating-system operation failed during supervision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisionOperation {
    /// The exact-child wait used to inspect leader state failed.
    Observe,
    /// Signalling the still-running direct leader failed.
    SignalLeader,
    /// Disposing of the retained leader's managed process group failed.
    SignalGroup,
    /// Consuming the retained direct child's terminal status failed.
    Reap,
}

/// Why observing or tearing down a VMM child did not complete.
///
/// A returned error leaves the handle's ownership unchanged, so a later
/// lifecycle call freshly re-observes the child and retries. The one
/// exception is a wait-based `ECHILD`: that loss is cached and permanently
/// prohibits further signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupervisionError {
    /// The operating-system action that failed.
    pub operation: SupervisionOperation,
    /// The operating-system errno, or `EPROTO` for an impossible successful
    /// wait result.
    pub errno: i32,
    /// A terminal leader state retained when later cleanup failed.
    pub observed: Option<VmmState>,
}

impl std::fmt::Display for SupervisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} failed for VMM child with errno {}",
            self.operation, self.errno
        )?;
        if let Some(observed) = self.observed {
            write!(f, " after observing {observed:?}")?;
        }
        Ok(())
    }
}

impl std::error::Error for SupervisionError {}

/// Why a VMM child could not be forked.
#[derive(Debug)]
pub enum SpawnError {
    ForkFailed(std::io::Error),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::ForkFailed(error) => write!(f, "fork failed: {error}"),
        }
    }
}

impl std::error::Error for SpawnError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Owned,
    ExitObserved(VmmState),
    GroupSignaled(VmmState),
    Finished {
        state: VmmState,
        failure: Option<SupervisionError>,
    },
}

/// An owned VMM child and the process group that child creates.
///
/// The child must call `setsid` successfully before its launcher runs. Trusted
/// host helpers must remain in that initial process group and within the
/// supervisor's signal permissions. No member may join or be forked from the
/// first post-terminal group disposition through inspection and reap. Darwin
/// also requires complete process-table visibility. Cleanup disposes of the
/// group before reaping the direct child, so a leader that exits naturally
/// cannot abandon a managed worker. This is not containment for descendants
/// that leave the group, change credentials, or keep forking during teardown.
///
/// The embedding process must leave `SIGCHLD` waitable and make this handle the
/// only consuming waiter for its exact child. Once another waiter consumes the
/// status, the handle reports `Lost` and never signals either the PID or PGID:
/// numerical identity is no longer authority. Dropping an unfinished handle
/// attempts the same synchronous leader teardown and direct-child reap, but
/// its diagnostics are best effort; callers that need an inspectable failure
/// must call a lifecycle method explicitly.
///
/// `mem::forget`, parent abort or `SIGKILL`, and an uninterruptible child are
/// outside the cleanup guarantee.
pub struct VmmChild {
    pid: libc::pid_t,
    lifecycle: Lifecycle,
}

impl VmmChild {
    /// Forks and runs `launcher` in a new session. Fork success does not prove
    /// session setup or launch success. A `setsid` failure exits with code 71.
    pub fn spawn(launcher: impl Launch) -> Result<VmmChild, SpawnError> {
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(SpawnError::ForkFailed(std::io::Error::last_os_error()));
        }
        if pid == 0 {
            let _guard = ExitOnUnwind;
            if unsafe { libc::setsid() } == -1 {
                unsafe { libc::_exit(71) }
            }
            launcher.launch()
        }
        Ok(VmmChild {
            pid,
            lifecycle: Lifecycle::Owned,
        })
    }

    /// The direct child's diagnostic process id.
    ///
    /// This does not transfer wait or signal authority. After completion or
    /// loss the number may be reused by an unrelated process.
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Polls without waiting for a running child.
    ///
    /// A successful terminal result means the managed group received its
    /// cleanup disposition and the direct child was reaped.
    pub fn state(&mut self) -> Result<VmmState, SupervisionError> {
        if let Some(result) = self.cached_state() {
            return result;
        }
        if matches!(self.lifecycle, Lifecycle::GroupSignaled(_)) {
            return self.reap(false);
        }
        match self.observe()? {
            VmmState::Running | VmmState::Lost => self.cached_or_running(),
            terminal => self.signal_group_and_reap(terminal, false),
        }
    }

    /// Polls until cleanup completes or `deadline` elapses.
    ///
    /// A zero budget still performs one observation. On expiry the actual last
    /// result is returned; the budget does not cancel pending ownership.
    pub fn wait_terminal(&mut self, deadline: Duration) -> Result<VmmState, SupervisionError> {
        let started = Instant::now();
        loop {
            let result = self.state();
            match result {
                Ok(VmmState::Running) => {}
                Err(error) if error.errno == libc::EINTR && started.elapsed() < deadline => {}
                _ => return result,
            }
            let Some(remaining) = deadline.checked_sub(started.elapsed()) else {
                return result;
            };
            if remaining.is_zero() {
                return result;
            }
            std::thread::sleep(POLL_INTERVAL.min(remaining));
        }
    }

    /// Synchronously stops the leader, signals its managed process group, and
    /// reaps the direct child.
    ///
    /// Worker death can still be delayed by scheduling or uninterruptible
    /// kernel work. Repeated calls after successful completion are no-ops.
    pub fn kill(&mut self) -> Result<(), SupervisionError> {
        if let Lifecycle::Finished { failure, .. } = self.lifecycle {
            return failure.map_or(Ok(()), Err);
        }
        if matches!(self.lifecycle, Lifecycle::GroupSignaled(_)) {
            return self.reap(true).map(|_| ());
        }
        match self.observe()? {
            VmmState::Lost => Err(self.finished_failure()),
            VmmState::Running => {
                self.signal_leader()?;
                let terminal = self.wait_for_leader()?;
                self.signal_group_and_reap(terminal, true).map(|_| ())
            }
            terminal => self.signal_group_and_reap(terminal, true).map(|_| ()),
        }
    }

    fn cached_state(&self) -> Option<Result<VmmState, SupervisionError>> {
        let Lifecycle::Finished { state, failure } = self.lifecycle else {
            return None;
        };
        Some(match failure {
            Some(error) if error.observed.is_some() => Err(error),
            _ => Ok(state),
        })
    }

    fn cached_or_running(&self) -> Result<VmmState, SupervisionError> {
        self.cached_state().unwrap_or(Ok(VmmState::Running))
    }

    fn finished_failure(&self) -> SupervisionError {
        let Lifecycle::Finished {
            failure: Some(error),
            ..
        } = self.lifecycle
        else {
            return self.error(SupervisionOperation::Observe, None);
        };
        error
    }

    fn observed_exit(&self) -> Option<VmmState> {
        match self.lifecycle {
            Lifecycle::ExitObserved(state) | Lifecycle::GroupSignaled(state) => Some(state),
            Lifecycle::Finished { failure, .. } => failure.and_then(|error| error.observed),
            Lifecycle::Owned => None,
        }
    }

    fn observe(&mut self) -> Result<VmmState, SupervisionError> {
        let known = self.observed_exit();
        match self.wait_once(
            SupervisionOperation::Observe,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        ) {
            Ok(Some(state)) => {
                self.lifecycle = Lifecycle::ExitObserved(state);
                Ok(state)
            }
            Ok(None) if known.is_none() => Ok(VmmState::Running),
            Ok(None) => Err(self.protocol_error(SupervisionOperation::Observe, known)),
            Err(error) if error.errno == libc::ECHILD => {
                let lost = SupervisionError {
                    observed: known,
                    ..error
                };
                self.lifecycle = Lifecycle::Finished {
                    state: VmmState::Lost,
                    failure: Some(lost),
                };
                if known.is_none() {
                    Ok(VmmState::Lost)
                } else {
                    Err(lost)
                }
            }
            Err(error) => Err(SupervisionError {
                observed: known,
                ..error
            }),
        }
    }

    fn wait_for_leader(&mut self) -> Result<VmmState, SupervisionError> {
        loop {
            match self.wait_once(SupervisionOperation::Observe, libc::WEXITED | libc::WNOWAIT) {
                Ok(Some(state)) => {
                    self.lifecycle = Lifecycle::ExitObserved(state);
                    return Ok(state);
                }
                Ok(None) => {
                    return Err(self.protocol_error(SupervisionOperation::Observe, None));
                }
                Err(error) if error.errno == libc::EINTR => {
                    note_interrupted_wait();
                }
                Err(error) if error.errno == libc::ECHILD => {
                    self.lifecycle = Lifecycle::Finished {
                        state: VmmState::Lost,
                        failure: Some(error),
                    };
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn signal_group_and_reap(
        &mut self,
        terminal: VmmState,
        blocking: bool,
    ) -> Result<VmmState, SupervisionError> {
        if unsafe { libc::kill(-self.pid, libc::SIGKILL) } == -1 {
            let errno = last_errno();
            let permitted = errno == libc::ESRCH
                || (errno == libc::EPERM && darwin_group_all_zombies(self.pid));
            if !permitted {
                self.lifecycle = Lifecycle::ExitObserved(terminal);
                return Err(SupervisionError {
                    operation: SupervisionOperation::SignalGroup,
                    errno,
                    observed: Some(terminal),
                });
            }
        }
        self.lifecycle = Lifecycle::GroupSignaled(terminal);
        self.reap(blocking)
    }

    fn reap(&mut self, blocking: bool) -> Result<VmmState, SupervisionError> {
        let observed = self.observed_exit();
        loop {
            let mut options = libc::WEXITED;
            if !blocking {
                options |= libc::WNOHANG;
            }
            match self.wait_once(SupervisionOperation::Reap, options) {
                Ok(Some(state)) => {
                    self.lifecycle = Lifecycle::Finished {
                        state,
                        failure: None,
                    };
                    return Ok(state);
                }
                Ok(None) => {
                    return Err(self.protocol_error(SupervisionOperation::Reap, observed));
                }
                Err(error) if blocking && error.errno == libc::EINTR => {
                    note_interrupted_wait();
                }
                Err(error) if error.errno == libc::ECHILD => {
                    let lost = SupervisionError { observed, ..error };
                    self.lifecycle = Lifecycle::Finished {
                        state: VmmState::Lost,
                        failure: Some(lost),
                    };
                    return Err(lost);
                }
                Err(error) => return Err(SupervisionError { observed, ..error }),
            }
        }
    }

    fn wait_once(
        &self,
        operation: SupervisionOperation,
        options: libc::c_int,
    ) -> Result<Option<VmmState>, SupervisionError> {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe { libc::waitid(libc::P_PID, self.pid as libc::id_t, &mut info, options) } == -1 {
            return Err(self.error(operation, None));
        }
        let reported = unsafe { info.si_pid() };
        if reported == 0 {
            return Ok(None);
        }
        if reported != self.pid {
            return Err(SupervisionError {
                operation,
                errno: libc::EPROTO,
                observed: None,
            });
        }
        let state = decode_info(&info).ok_or(SupervisionError {
            operation,
            errno: libc::EPROTO,
            observed: None,
        })?;
        record_reap(operation, reported);
        Ok(Some(state))
    }

    fn error(
        &self,
        operation: SupervisionOperation,
        observed: Option<VmmState>,
    ) -> SupervisionError {
        SupervisionError {
            operation,
            errno: last_errno(),
            observed,
        }
    }

    fn protocol_error(
        &self,
        operation: SupervisionOperation,
        observed: Option<VmmState>,
    ) -> SupervisionError {
        SupervisionError {
            operation,
            errno: libc::EPROTO,
            observed,
        }
    }

    fn signal_leader(&self) -> Result<(), SupervisionError> {
        if unsafe { libc::kill(self.pid, libc::SIGKILL) } == -1 {
            Err(self.error(SupervisionOperation::SignalLeader, None))
        } else {
            Ok(())
        }
    }

    fn drop_teardown(&mut self) {
        match self.lifecycle {
            Lifecycle::Finished { failure, .. } => {
                if let Some(error) = failure {
                    self.report_drop(error, None);
                }
                return;
            }
            Lifecycle::GroupSignaled(_) => {
                if let Err(error) = self.reap(true) {
                    self.report_drop(error, None);
                }
                return;
            }
            Lifecycle::Owned | Lifecycle::ExitObserved(_) => {}
        }

        let observed = loop {
            match self.observe() {
                Err(error) if error.errno == libc::EINTR => {
                    note_interrupted_wait();
                }
                observed => break observed,
            }
        };
        match observed {
            Ok(VmmState::Lost) => self.report_drop(self.finished_failure(), None),
            Ok(VmmState::Running) => {
                if let Err(original) = self.signal_leader() {
                    self.drop_after_leader_signal_failure(original);
                    return;
                }
                match self.wait_for_leader() {
                    Ok(terminal) => self.drop_terminal(terminal, true, None),
                    Err(error) => self.report_drop(error, None),
                }
            }
            Ok(terminal) => self.drop_terminal(terminal, true, None),
            Err(error) => self.report_drop(error, None),
        }
    }

    fn drop_after_leader_signal_failure(&mut self, original: SupervisionError) {
        match self.observe() {
            Ok(VmmState::Running) => self.report_drop(original, None),
            Ok(VmmState::Lost) => self.report_drop(original, Some(self.finished_failure())),
            Ok(terminal) => self.drop_terminal(terminal, false, Some(original)),
            Err(later) => self.report_drop(original, Some(later)),
        }
    }

    fn drop_terminal(
        &mut self,
        terminal: VmmState,
        blocking_reap: bool,
        original: Option<SupervisionError>,
    ) {
        match self.signal_group_and_reap(terminal, blocking_reap) {
            Ok(_) => {
                if let Some(error) = original {
                    self.report_drop(error, None);
                }
            }
            Err(group_error) if group_error.operation == SupervisionOperation::SignalGroup => {
                let later = self.reap_after_group_failure(terminal, group_error);
                self.report_drop(original.unwrap_or(group_error), later);
            }
            Err(error) => self.report_drop(original.unwrap_or(error), Some(error)),
        }
    }

    fn reap_after_group_failure(
        &mut self,
        terminal: VmmState,
        group_error: SupervisionError,
    ) -> Option<SupervisionError> {
        let reaped = loop {
            match self.wait_once(SupervisionOperation::Reap, libc::WEXITED | libc::WNOHANG) {
                Err(error) if error.errno == libc::EINTR => {
                    note_interrupted_wait();
                }
                reaped => break reaped,
            }
        };
        match reaped {
            Ok(Some(state)) => {
                self.lifecycle = Lifecycle::Finished {
                    state,
                    failure: Some(group_error),
                };
                None
            }
            Ok(None) => Some(self.protocol_error(SupervisionOperation::Reap, Some(terminal))),
            Err(error) if error.errno == libc::ECHILD => {
                let lost = SupervisionError {
                    observed: Some(terminal),
                    ..error
                };
                self.lifecycle = Lifecycle::Finished {
                    state: VmmState::Lost,
                    failure: Some(lost),
                };
                Some(lost)
            }
            Err(error) => Some(SupervisionError {
                observed: Some(terminal),
                ..error
            }),
        }
    }

    fn report_drop(&self, error: SupervisionError, later: Option<SupervisionError>) {
        let signalling_forbidden = matches!(
            self.lifecycle,
            Lifecycle::Finished {
                state: VmmState::Lost,
                ..
            }
        );
        let disposition = match self.lifecycle {
            Lifecycle::Owned => "leader and workers may remain live",
            Lifecycle::ExitObserved(_) => "leader exit retained; group cleanup incomplete",
            Lifecycle::GroupSignaled(_) => "group signal completed; direct reap incomplete",
            Lifecycle::Finished {
                state: VmmState::Lost,
                ..
            } => "authority lost; workers may remain live",
            Lifecycle::Finished {
                failure: Some(_), ..
            } => "direct child reaped; group cleanup failed",
            Lifecycle::Finished { failure: None, .. } => "cleanup completed",
        };
        let _ = writeln!(
            std::io::stderr(),
            "VmmChild pid {} drop cleanup error: {}; later={later:?}; {disposition}; \
             further signalling forbidden={signalling_forbidden}",
            self.pid,
            error
        );
    }
}

#[cfg(test)]
static INTERRUPTED_REAPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn note_interrupted_wait() {
    INTERRUPTED_REAPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(not(test))]
fn note_interrupted_wait() {}

#[cfg(test)]
fn interrupted_reaps() -> usize {
    INTERRUPTED_REAPS.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
std::thread_local! {
    static LAST_REAPED_PID: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn record_reap(operation: SupervisionOperation, pid: i32) {
    if operation == SupervisionOperation::Reap {
        LAST_REAPED_PID.set(pid);
    }
}

#[cfg(not(test))]
fn record_reap(_operation: SupervisionOperation, _pid: i32) {}

#[cfg(test)]
fn reset_reap_record() {
    LAST_REAPED_PID.set(0);
}

#[cfg(test)]
fn assert_reaped_by_handle(pid: i32) {
    assert_eq!(
        LAST_REAPED_PID.get(),
        pid,
        "the consuming exact-child wait returned the owned leader"
    );
}

impl Drop for VmmChild {
    fn drop(&mut self) {
        self.drop_teardown();
    }
}

fn decode_info(info: &libc::siginfo_t) -> Option<VmmState> {
    match info.si_code {
        libc::CLD_EXITED => Some(VmmState::Exited {
            code: unsafe { info.si_status() } & 0xff,
        }),
        libc::CLD_KILLED | libc::CLD_DUMPED => Some(VmmState::Signaled {
            signal: unsafe { info.si_status() },
        }),
        _ => None,
    }
}

fn last_errno() -> i32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn plasmosome_darwin_group_all_zombies(leader: libc::pid_t) -> libc::c_int;
    #[cfg(test)]
    fn plasmosome_darwin_group_test_table(
        scenario: libc::c_int,
        leader: libc::pid_t,
    ) -> libc::c_int;
}

#[cfg(target_os = "macos")]
fn darwin_group_all_zombies(leader: libc::pid_t) -> bool {
    unsafe { plasmosome_darwin_group_all_zombies(leader) == 1 }
}

#[cfg(not(target_os = "macos"))]
fn darwin_group_all_zombies(_leader: libc::pid_t) -> bool {
    false
}

struct ExitOnUnwind;

impl Drop for ExitOnUnwind {
    fn drop(&mut self) {
        unsafe { libc::_exit(70) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::ExecCommand;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    include!("../tests/support/fixture.rs");

    const DEADLINE: Duration = Duration::from_secs(5);
    const SETTLE: Duration = Duration::from_millis(300);

    struct ExitWith(i32);

    impl Launch for ExitWith {
        fn launch(self) -> ! {
            unsafe { libc::_exit(self.0) }
        }
    }

    struct DieBySignal(i32);

    impl Launch for DieBySignal {
        fn launch(self) -> ! {
            unsafe {
                libc::signal(self.0, libc::SIG_DFL);
                libc::raise(self.0);
                libc::_exit(99)
            }
        }
    }

    struct SleepForever;

    impl Launch for SleepForever {
        fn launch(self) -> ! {
            loop {
                unsafe { libc::pause() };
            }
        }
    }

    struct PanicOnLaunch;

    impl Launch for PanicOnLaunch {
        fn launch(self) -> ! {
            panic!("a launcher that breaks its no-panic contract")
        }
    }

    struct PipeEnd(i32);

    impl PipeEnd {
        fn close(mut self) {
            if self.0 >= 0 {
                unsafe { libc::close(self.0) };
                self.0 = -1;
            }
        }
    }

    impl Drop for PipeEnd {
        fn drop(&mut self) {
            if self.0 >= 0 {
                unsafe { libc::close(self.0) };
            }
        }
    }

    fn pipe() -> (PipeEnd, PipeEnd) {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "a pipe opens");
        (PipeEnd(fds[0]), PipeEnd(fds[1]))
    }

    struct WorkerFixture {
        child: Option<VmmChild>,
        liveness: PipeEnd,
        release: Option<PipeEnd>,
        _dir: tempfile::TempDir,
    }

    impl WorkerFixture {
        fn child(&mut self) -> &mut VmmChild {
            self.child
                .as_mut()
                .expect("the fixture still owns its leader")
        }

        fn take_child(&mut self) -> VmmChild {
            self.child
                .take()
                .expect("the fixture still owns its leader")
        }

        fn release_live_worker(&mut self) {
            let release = self.release.take().expect("the worker is unreleased");
            let byte = b'x';
            assert_eq!(
                unsafe { libc::write(release.0, &byte as *const u8 as *const libc::c_void, 1,) },
                1,
                "the private control descriptor releases the live worker"
            );
            release.close();
            assert_eof_within(&self.liveness, DEADLINE);
        }

        fn discard_release(&mut self) {
            self.release.take();
        }
    }

    impl Drop for WorkerFixture {
        fn drop(&mut self) {
            if holds_open(&self.liveness, Duration::ZERO) {
                self.release_live_worker();
            } else {
                self.release.take();
            }
            self.child.take();
        }
    }

    fn spawn_with_worker(leader_exits: bool) -> WorkerFixture {
        let (ready_read, ready_write) = pipe();
        let (control_read, control_write) = pipe();
        let dir = tempfile::tempdir().expect("the fixture owns its directory");
        let liveness_path = dir.path().join("worker-liveness");
        let c_path = CString::new(liveness_path.as_os_str().as_bytes())
            .expect("the temporary path has no NUL");
        assert_eq!(
            unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) },
            0,
            "the worker liveness FIFO is created"
        );
        let liveness = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        assert!(liveness >= 0, "the parent opens its private FIFO reader");
        let mode = if leader_exits { "exit" } else { "wait" };
        let command = ExecCommand::new(vec![
            supervision_fixture().display().to_string(),
            mode.to_string(),
            ready_write.0.to_string(),
            control_read.0.to_string(),
            liveness_path.display().to_string(),
        ])
        .expect("the prebuilt fixture executable resolves before fork");
        let child = VmmChild::spawn(command).expect("fork succeeds");
        ready_write.close();
        control_read.close();
        wait_readable(&ready_read, DEADLINE);
        let mut byte = 0u8;
        assert_eq!(
            unsafe { libc::read(ready_read.0, &mut byte as *mut u8 as *mut libc::c_void, 1,) },
            1,
            "the execed worker announces readiness after opening its FIFO"
        );
        assert_eq!(byte, b'x');
        WorkerFixture {
            child: Some(child),
            liveness: PipeEnd(liveness),
            release: Some(control_write),
            _dir: dir,
        }
    }

    fn wait_readable(pipe: &PipeEnd, patience: Duration) {
        let mut watched = libc::pollfd {
            fd: pipe.0,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        let timeout = libc::c_int::try_from(patience.as_millis()).expect("timeout fits");
        loop {
            let ready = unsafe { libc::poll(&mut watched, 1, timeout) };
            if ready > 0 {
                return;
            }
            assert_ne!(ready, 0, "the descriptor changes within {patience:?}");
            assert_eq!(last_errno(), libc::EINTR);
        }
    }

    fn holds_open(pipe: &PipeEnd, patience: Duration) -> bool {
        let started = Instant::now();
        loop {
            let mut byte = 0u8;
            let read = unsafe { libc::read(pipe.0, &mut byte as *mut u8 as *mut libc::c_void, 1) };
            if read == 0 {
                return false;
            }
            if read == -1 {
                let errno = last_errno();
                if errno == libc::EINTR {
                    continue;
                }
                assert!(
                    errno == libc::EAGAIN || errno == libc::EWOULDBLOCK,
                    "the FIFO read fails only because its live writer has no data"
                );
            } else {
                panic!("the liveness-only FIFO carries no data");
            }
            if started.elapsed() >= patience {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn assert_eof_within(pipe: &PipeEnd, patience: Duration) {
        let started = Instant::now();
        while holds_open(pipe, Duration::ZERO) {
            assert!(
                started.elapsed() < patience,
                "the worker closes its private liveness FIFO within {patience:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn witness_unconsumed_exit(pid: i32, expected: VmmState) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let waited = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            assert_eq!(waited, 0, "the exact direct child remains waitable");
            let reported = unsafe { info.si_pid() };
            if reported == pid {
                assert_eq!(decode_info(&info), Some(expected));
                return;
            }
            assert_eq!(reported, 0, "waitid reports only the selected child");
            assert!(
                Instant::now() < deadline,
                "the leader exits within deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn assert_no_waitable_child(pid: i32) {
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(last_errno(), libc::ECHILD);
    }

    fn reap_externally(pid: i32) {
        let mut status = 0;
        loop {
            let reaped = unsafe { libc::waitpid(pid, &mut status, 0) };
            if reaped == pid {
                return;
            }
            assert_eq!(last_errno(), libc::EINTR);
        }
    }

    #[cfg(target_os = "macos")]
    struct DarwinOwnedGroup {
        pids: Vec<i32>,
        controls: Vec<Option<PipeEnd>>,
    }

    #[cfg(target_os = "macos")]
    impl DarwinOwnedGroup {
        fn new(count: usize) -> DarwinOwnedGroup {
            let mut group = DarwinOwnedGroup {
                pids: Vec::with_capacity(count),
                controls: Vec::with_capacity(count),
            };
            for index in 0..count {
                let (control_read, control_write) = pipe();
                let (ready_read, ready_write) = pipe();
                let leader = group.pids.first().copied().unwrap_or(0);
                let pid = unsafe { libc::fork() };
                assert!(pid >= 0, "the certificate fixture fork succeeds");
                if pid == 0 {
                    unsafe {
                        libc::close(control_write.0);
                        libc::close(ready_read.0);
                        if libc::setpgid(0, leader) == -1 {
                            libc::_exit(71);
                        }
                        let byte = b'x';
                        if libc::write(ready_write.0, &byte as *const u8 as *const libc::c_void, 1)
                            != 1
                        {
                            libc::_exit(72);
                        }
                        libc::close(ready_write.0);
                        let mut release = 0u8;
                        let read = libc::read(
                            control_read.0,
                            &mut release as *mut u8 as *mut libc::c_void,
                            1,
                        );
                        if read >= 0 {
                            libc::_exit(7 + index as i32);
                        }
                        libc::_exit(73);
                    }
                }
                control_read.close();
                ready_write.close();
                wait_readable(&ready_read, DEADLINE);
                let mut byte = 0u8;
                assert_eq!(
                    unsafe {
                        libc::read(ready_read.0, &mut byte as *mut u8 as *mut libc::c_void, 1)
                    },
                    1
                );
                assert_eq!(byte, b'x');
                group.pids.push(pid);
                group.controls.push(Some(control_write));
            }
            group
        }

        fn release(&mut self, index: usize) {
            let control = self.controls[index]
                .take()
                .expect("the owned member is unreleased");
            let byte = b'x';
            assert_eq!(
                unsafe { libc::write(control.0, &byte as *const u8 as *const libc::c_void, 1,) },
                1,
                "the private descriptor releases the owned group member"
            );
            control.close();
            witness_unconsumed_exit(
                self.pids[index],
                VmmState::Exited {
                    code: 7 + index as i32,
                },
            );
        }

        fn reap_all(&mut self) {
            for pid in self.pids.drain(..) {
                reap_externally(pid);
            }
        }
    }

    #[cfg(target_os = "macos")]
    impl Drop for DarwinOwnedGroup {
        fn drop(&mut self) {
            self.controls.clear();
            for pid in self.pids.drain(..) {
                let mut status = 0;
                loop {
                    let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
                    if waited == pid || (waited == -1 && last_errno() == libc::ECHILD) {
                        break;
                    }
                    if waited == -1 && last_errno() == libc::EINTR {
                        continue;
                    }
                    panic!("the certificate fixture reaps owned pid {pid}");
                }
            }
        }
    }

    #[test]
    fn a_child_that_exits_reports_exited_with_its_code() {
        let mut child = VmmChild::spawn(ExitWith(7)).expect("fork succeeds");
        assert_eq!(
            child.wait_terminal(DEADLINE),
            Ok(VmmState::Exited { code: 7 })
        );
    }

    #[test]
    fn a_live_child_reports_running() {
        let mut child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        assert_eq!(child.state(), Ok(VmmState::Running));
        child.kill().expect("signalling a live child succeeds");
    }

    #[test]
    fn kill_moves_a_running_child_to_signaled() {
        let mut child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        child.kill().expect("signalling a live child succeeds");
        assert_eq!(
            child.wait_terminal(DEADLINE),
            Ok(VmmState::Signaled {
                signal: libc::SIGKILL
            })
        );
    }

    #[test]
    fn kill_then_immediate_drop_leaves_no_orphan() {
        let mut child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        let pid = child.pid();
        child.kill().expect("signalling a live child succeeds");
        drop(child);
        assert_no_waitable_child(pid);
    }

    #[test]
    fn drop_reaps_a_running_child_without_orphans() {
        let child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        let pid = child.pid();
        drop(child);
        assert_no_waitable_child(pid);
    }

    #[test]
    fn drop_first_cleans_a_naturally_exited_leaders_worker() {
        let mut fixture = spawn_with_worker(true);
        let pid = fixture.child().pid();
        witness_unconsumed_exit(pid, VmmState::Exited { code: 7 });
        reset_reap_record();
        drop(fixture.take_child());
        assert_eof_within(&fixture.liveness, DEADLINE);
        assert_no_waitable_child(pid);
        assert_reaped_by_handle(pid);
        fixture.discard_release();
    }

    #[test]
    fn state_first_cleans_a_naturally_exited_leaders_worker() {
        let mut fixture = spawn_with_worker(true);
        let pid = fixture.child().pid();
        witness_unconsumed_exit(pid, VmmState::Exited { code: 7 });
        reset_reap_record();
        assert_eq!(fixture.child().state(), Ok(VmmState::Exited { code: 7 }));
        assert_eof_within(&fixture.liveness, DEADLINE);
        assert_no_waitable_child(pid);
        assert_reaped_by_handle(pid);
        assert_eq!(fixture.child().state(), Ok(VmmState::Exited { code: 7 }));
        drop(fixture.take_child());
        fixture.discard_release();
    }

    #[test]
    fn wait_first_cleans_a_naturally_exited_leaders_worker() {
        let mut fixture = spawn_with_worker(true);
        let pid = fixture.child().pid();
        witness_unconsumed_exit(pid, VmmState::Exited { code: 7 });
        reset_reap_record();
        assert_eq!(
            fixture.child().wait_terminal(DEADLINE),
            Ok(VmmState::Exited { code: 7 })
        );
        assert_eof_within(&fixture.liveness, DEADLINE);
        assert_no_waitable_child(pid);
        assert_reaped_by_handle(pid);
        drop(fixture.take_child());
        fixture.discard_release();
    }

    #[test]
    fn kill_first_preserves_a_naturally_exited_leaders_status() {
        let mut fixture = spawn_with_worker(true);
        let pid = fixture.child().pid();
        witness_unconsumed_exit(pid, VmmState::Exited { code: 7 });
        reset_reap_record();
        fixture.child().kill().expect("cleanup succeeds");
        assert_eof_within(&fixture.liveness, DEADLINE);
        assert_no_waitable_child(pid);
        assert_reaped_by_handle(pid);
        assert_eq!(fixture.child().state(), Ok(VmmState::Exited { code: 7 }));
        fixture.discard_release();
    }

    #[test]
    fn every_entry_point_fails_closed_after_external_reap() {
        for entry in ["state", "wait", "kill", "drop"] {
            let mut fixture = spawn_with_worker(true);
            let pid = fixture.child().pid();
            witness_unconsumed_exit(pid, VmmState::Exited { code: 7 });
            reap_externally(pid);
            match entry {
                "state" => {
                    assert_eq!(fixture.child().state(), Ok(VmmState::Lost));
                    assert_eq!(fixture.child().state(), Ok(VmmState::Lost));
                }
                "wait" => {
                    assert_eq!(
                        fixture.child().wait_terminal(Duration::ZERO),
                        Ok(VmmState::Lost)
                    );
                    assert_eq!(fixture.child().wait_terminal(DEADLINE), Ok(VmmState::Lost));
                }
                "kill" => {
                    let expected = SupervisionError {
                        operation: SupervisionOperation::Observe,
                        errno: libc::ECHILD,
                        observed: None,
                    };
                    assert_eq!(fixture.child().kill(), Err(expected));
                    assert_eq!(fixture.child().kill(), Err(expected));
                }
                "drop" => drop(fixture.take_child()),
                _ => unreachable!(),
            }
            assert!(
                holds_open(&fixture.liveness, SETTLE),
                "{entry} signalled after exact-child authority was lost"
            );
            if fixture.child.is_some() {
                drop(fixture.take_child());
                assert!(
                    holds_open(&fixture.liveness, SETTLE),
                    "cached loss signalled during Drop"
                );
            }
            fixture.release_live_worker();
        }
    }

    #[test]
    fn loss_after_a_peek_preserves_the_exit_and_forbids_future_signals() {
        let mut fixture = spawn_with_worker(true);
        let pid = fixture.child().pid();
        witness_unconsumed_exit(pid, VmmState::Exited { code: 7 });
        assert_eq!(fixture.child().observe(), Ok(VmmState::Exited { code: 7 }));
        reap_externally(pid);
        let expected = SupervisionError {
            operation: SupervisionOperation::Observe,
            errno: libc::ECHILD,
            observed: Some(VmmState::Exited { code: 7 }),
        };
        assert_eq!(fixture.child().state(), Err(expected));
        assert_eq!(fixture.child().kill(), Err(expected));
        assert!(holds_open(&fixture.liveness, SETTLE));
        drop(fixture.take_child());
        assert!(holds_open(&fixture.liveness, SETTLE));
        fixture.release_live_worker();
    }

    #[test]
    fn completed_and_lost_handles_do_not_touch_an_owned_sentinel() {
        let mut sentinel = spawn_with_worker(false);
        let mut completed = VmmChild::spawn(ExitWith(3)).expect("fork succeeds");
        assert_eq!(
            completed.wait_terminal(DEADLINE),
            Ok(VmmState::Exited { code: 3 })
        );
        assert_eq!(completed.state(), Ok(VmmState::Exited { code: 3 }));
        completed.kill().expect("completed kill is a no-op");
        let mut lost = VmmChild::spawn(ExitWith(4)).expect("fork succeeds");
        witness_unconsumed_exit(lost.pid(), VmmState::Exited { code: 4 });
        reap_externally(lost.pid());
        assert_eq!(lost.state(), Ok(VmmState::Lost));
        assert_eq!(lost.state(), Ok(VmmState::Lost));
        drop(completed);
        drop(lost);
        assert!(
            holds_open(&sentinel.liveness, SETTLE),
            "released identities affected an independently owned group"
        );
        sentinel.child().kill().expect("sentinel cleanup succeeds");
        assert_eof_within(&sentinel.liveness, DEADLINE);
        sentinel.discard_release();
    }

    #[test]
    fn a_running_child_and_zero_budget_keep_ownership() {
        let mut child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        assert_eq!(child.state(), Ok(VmmState::Running));
        assert_eq!(child.wait_terminal(Duration::ZERO), Ok(VmmState::Running));
        child.kill().expect("synchronous cleanup succeeds");
        assert_eq!(
            child.state(),
            Ok(VmmState::Signaled {
                signal: libc::SIGKILL
            })
        );
    }

    #[test]
    fn normal_signal_core_and_unwind_statuses_are_preserved() {
        for (launcher, expected) in [
            (
                Box::new(|| VmmChild::spawn(ExitWith(255))) as Box<dyn Fn() -> _>,
                VmmState::Exited { code: 255 },
            ),
            (
                Box::new(|| VmmChild::spawn(DieBySignal(libc::SIGABRT))),
                VmmState::Signaled {
                    signal: libc::SIGABRT,
                },
            ),
            (
                Box::new(|| VmmChild::spawn(PanicOnLaunch)),
                VmmState::Exited { code: 70 },
            ),
        ] {
            let mut child = launcher().expect("fork succeeds");
            let pid = child.pid();
            witness_unconsumed_exit(pid, expected);
            assert_eq!(child.wait_terminal(DEADLINE), Ok(expected));
        }
    }

    #[test]
    fn completed_group_disposition_never_resignals_after_reap_loss() {
        for entry in ["state", "wait", "kill", "drop"] {
            let mut fixture = spawn_with_worker(true);
            let pid = fixture.child().pid();
            witness_unconsumed_exit(pid, VmmState::Exited { code: 7 });
            assert_eq!(fixture.child().observe(), Ok(VmmState::Exited { code: 7 }));
            fixture.child().lifecycle = Lifecycle::GroupSignaled(VmmState::Exited { code: 7 });
            reap_externally(pid);
            let expected = SupervisionError {
                operation: SupervisionOperation::Reap,
                errno: libc::ECHILD,
                observed: Some(VmmState::Exited { code: 7 }),
            };
            match entry {
                "state" => assert_eq!(fixture.child().state(), Err(expected)),
                "wait" => assert_eq!(fixture.child().wait_terminal(Duration::ZERO), Err(expected)),
                "kill" => assert_eq!(fixture.child().kill(), Err(expected)),
                "drop" => drop(fixture.take_child()),
                _ => unreachable!(),
            }
            assert!(
                holds_open(&fixture.liveness, SETTLE),
                "{entry} repeated a completed group disposition"
            );
            if fixture.child.is_some() {
                drop(fixture.take_child());
                assert!(holds_open(&fixture.liveness, SETTLE));
            }
            fixture.release_live_worker();
        }
    }

    #[test]
    fn leader_signal_esrch_is_not_wait_authority_loss() {
        let mut child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        assert_eq!(child.observe(), Ok(VmmState::Running));
        assert_eq!(unsafe { libc::kill(child.pid(), libc::SIGKILL) }, 0);
        reap_externally(child.pid());
        let expected = SupervisionError {
            operation: SupervisionOperation::SignalLeader,
            errno: libc::ESRCH,
            observed: None,
        };
        assert_eq!(child.signal_leader(), Err(expected));
        assert_eq!(child.lifecycle, Lifecycle::Owned);
        assert_eq!(child.state(), Ok(VmmState::Lost));
    }

    #[test]
    fn immediate_teardown_while_child_is_pre_setsid_reaps_exact_child() {
        let (gate_read, gate_write) = pipe();
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "the instrumented fork succeeds");
        if pid == 0 {
            unsafe {
                libc::close(gate_write.0);
                let mut byte = 0u8;
                libc::read(gate_read.0, &mut byte as *mut u8 as *mut libc::c_void, 1);
                if libc::setsid() == -1 {
                    libc::_exit(71);
                }
                loop {
                    libc::pause();
                }
            }
        }
        gate_read.close();
        let mut child = VmmChild {
            pid,
            lifecycle: Lifecycle::Owned,
        };
        child.kill().expect("pre-setsid teardown completes");
        assert_no_waitable_child(pid);
        gate_write.close();
    }

    #[test]
    fn immediate_drop_kills_execed_leader_and_worker() {
        let mut fixture = spawn_with_worker(false);
        let pid = fixture.child().pid();
        drop(fixture.take_child());
        assert_eof_within(&fixture.liveness, DEADLINE);
        assert_no_waitable_child(pid);
        fixture.discard_release();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_certificate_requires_complete_all_zombie_membership() {
        let mut singleton = DarwinOwnedGroup::new(1);
        singleton.release(0);
        assert!(darwin_group_all_zombies(singleton.pids[0]));
        singleton.reap_all();

        let mut group = DarwinOwnedGroup::new(3);
        assert!(!darwin_group_all_zombies(group.pids[0]));
        group.release(0);
        assert!(!darwin_group_all_zombies(group.pids[0]));
        group.release(1);
        group.release(2);
        assert!(darwin_group_all_zombies(group.pids[0]));
        group.reap_all();

        for scenario in [0, 1, 12] {
            assert_eq!(
                unsafe { plasmosome_darwin_group_test_table(scenario, 4000) },
                1,
                "certificate scenario {scenario} is valid"
            );
        }
        for scenario in [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13] {
            assert_eq!(
                unsafe { plasmosome_darwin_group_test_table(scenario, 4000) },
                0,
                "certificate scenario {scenario} fails closed"
            );
        }
    }
}

#[cfg(test)]
mod signal_pressure {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const CHILDREN: usize = 64;
    const SIGNALS_PER_BURST: usize = 512;

    static DELIVERED: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn note_signal(_sig: libc::c_int) {
        DELIVERED.fetch_add(1, Ordering::Relaxed);
    }

    struct SleepForever;

    impl Launch for SleepForever {
        fn launch(self) -> ! {
            loop {
                unsafe { libc::pause() };
            }
        }
    }

    struct InterruptingHandler {
        previous: libc::sigaction,
    }

    impl InterruptingHandler {
        fn install() -> InterruptingHandler {
            let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
            let installed = unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                libc::sigemptyset(&mut action.sa_mask);
                action.sa_sigaction = note_signal as *const () as usize;
                action.sa_flags = 0;
                libc::sigaction(libc::SIGUSR1, &action, &mut previous)
            };
            assert_eq!(
                installed, 0,
                "a SIGUSR1 handler that does not restart syscalls must install"
            );
            InterruptingHandler { previous }
        }
    }

    impl Drop for InterruptingHandler {
        fn drop(&mut self) {
            unsafe { libc::sigaction(libc::SIGUSR1, &self.previous, std::ptr::null_mut()) };
        }
    }

    fn set_sigusr1_blocked(blocked: bool) {
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGUSR1);
            let how = if blocked {
                libc::SIG_BLOCK
            } else {
                libc::SIG_UNBLOCK
            };
            libc::pthread_sigmask(how, &set, std::ptr::null_mut());
        }
    }

    struct SignalStorm {
        budget: Arc<AtomicUsize>,
        running: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl SignalStorm {
        fn aimed_at_this_thread() -> SignalStorm {
            let target = unsafe { libc::pthread_self() } as usize;
            let budget = Arc::new(AtomicUsize::new(0));
            let running = Arc::new(AtomicBool::new(true));
            let thread = std::thread::spawn({
                let budget = Arc::clone(&budget);
                let running = Arc::clone(&running);
                move || {
                    while running.load(Ordering::Relaxed) {
                        let claimed = budget
                            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                                left.checked_sub(1)
                            })
                            .is_ok();
                        if claimed {
                            unsafe { libc::pthread_kill(target as libc::pthread_t, libc::SIGUSR1) };
                        } else {
                            std::thread::yield_now();
                        }
                    }
                }
            });
            set_sigusr1_blocked(true);
            SignalStorm {
                budget,
                running,
                thread: Some(thread),
            }
        }

        fn burst(&self) -> Burst<'_> {
            self.budget.store(SIGNALS_PER_BURST, Ordering::Relaxed);
            set_sigusr1_blocked(false);
            Burst(&self.budget)
        }
    }

    impl Drop for SignalStorm {
        fn drop(&mut self) {
            self.budget.store(0, Ordering::Relaxed);
            self.running.store(false, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            set_sigusr1_blocked(false);
        }
    }

    struct Burst<'a>(&'a AtomicUsize);

    impl Drop for Burst<'_> {
        fn drop(&mut self) {
            self.0.store(0, Ordering::Relaxed);
            set_sigusr1_blocked(true);
        }
    }

    #[test]
    fn a_reap_interrupted_by_a_signal_still_leaves_no_orphan() {
        let _handler = InterruptingHandler::install();
        let storm = SignalStorm::aimed_at_this_thread();
        let signals_before = DELIVERED.load(Ordering::Relaxed);
        let interruptions_before = interrupted_reaps();

        for _ in 0..CHILDREN {
            let child = VmmChild::spawn(SleepForever).expect("fork succeeds");
            let pid = child.pid();
            {
                let _burst = storm.burst();
                drop(child);
            }
            let mut status: libc::c_int = 0;
            let observed = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            let errno = std::io::Error::last_os_error().raw_os_error();
            assert!(
                observed == -1 && errno == Some(libc::ECHILD),
                "pid {pid} survived a drop taken under signal pressure"
            );
        }

        drop(storm);
        let signals = DELIVERED.load(Ordering::Relaxed) - signals_before;
        let interruptions = interrupted_reaps() - interruptions_before;
        assert!(signals > 0, "no SIGUSR1 reached the dropping thread");
        assert!(
            interruptions > 0,
            "{signals} signals reached the dropping thread but none landed inside a blocking wait, so the EINTR retry was never exercised and this test proves nothing"
        );
    }
}
