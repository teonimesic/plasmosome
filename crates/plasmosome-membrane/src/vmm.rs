use crate::spawn_lock::Forked;
use std::io::Write;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// The body of a forked VMM child. `VmmChild::spawn` forks, starts a new
/// session and runs `launch` in the child.
///
/// # Safety
///
/// `launch` runs in the child of a fork, and the parent may have other
/// threads, as it does in production and under the test harness. Until it ends,
/// an implementation may only make async-signal-safe calls, such as
/// `execve`, `_exit`, `pause` and raw system calls. It must not allocate or free
/// memory, wait for a lock or use Rust's stdio, which all take locks: another thread
/// may have held one at the fork, and then the child waits forever. Lay out
/// everything the child needs in the parent, before the fork.
///
/// It must not drop anything that owns heap memory or run any other destructor,
/// including by unwinding. Dropping a `BoundSocket` or a `WritableRoot` in the
/// child would unlink the daemon's live socket or remove the cell's writable root.
/// It ends with `execve`, `_exit` or a loop of `pause`, never `std::process::exit`
/// or `libc::exit`, which run exit handlers and flush stdio.
///
/// An implementation must also never panic or unwind. A panic in the forked child
/// runs the panic hook, which allocates and takes locks to write its
/// message. A panic that escapes is contained by exiting the child with
/// code 70, but only after the hook has run.
///
/// Each implementation is an `unsafe impl` whose `// SAFETY:` comment says
/// what its `launch` does in the child and why that is async-signal-safe:
///
/// ```
/// struct ExitsAtOnce;
///
/// #[allow(unsafe_code)]
/// // SAFETY: In the child, launch only calls _exit, which is async-signal-safe.
/// unsafe impl plasmosome_membrane::vmm::Launch for ExitsAtOnce {
///     #[allow(unsafe_code)]
///     fn launch(self) -> ! {
///         // SAFETY: _exit takes no pointer and is async-signal-safe.
///         unsafe { libc::_exit(0) }
///     }
/// }
/// ```
///
/// A safe implementation does not compile. The compiling twin above adds `unsafe`,
/// the impl's `#[allow(unsafe_code)]` and its SAFETY comment. Neither extra line
/// changes whether this safe impl compiles: it needs no allow, and comments have
/// no effect on compilation. The register package's doctests inherit no unsafe lint.
///
/// ```compile_fail,E0200
/// struct ExitsAtOnce;
///
/// impl plasmosome_membrane::vmm::Launch for ExitsAtOnce {
///     #[allow(unsafe_code)]
///     fn launch(self) -> ! {
///         // SAFETY: _exit takes no pointer and is async-signal-safe.
///         unsafe { libc::_exit(0) }
///     }
/// }
/// ```
#[allow(unsafe_code)]
pub unsafe trait Launch {
    /// Only `VmmChild::spawn` calls this, in the child it forks.
    /// Callers must never call it themselves.
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
    /// The spawn ran on a thread that is creating descriptors under the spawn
    /// lock. Forking there would wait on that thread forever, so nothing was
    /// forked.
    DescriptorLockHeld,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::ForkFailed(error) => write!(f, "fork failed: {error}"),
            SpawnError::DescriptorLockHeld => write!(
                f,
                "spawn refused: this thread holds the descriptor lock, so nothing was forked"
            ),
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
    ///
    /// The fork waits while another thread in this process is creating
    /// descriptors that are not yet close-on-exec. Called on the thread doing
    /// that creation, it forks nothing and returns
    /// `SpawnError::DescriptorLockHeld`.
    #[allow(unsafe_code)]
    pub fn spawn(launcher: impl Launch) -> Result<VmmChild, SpawnError> {
        let pid = match crate::spawn_lock::fork()? {
            Forked::Parent(pid) => pid,
            Forked::Child => {
                let _guard = ExitOnUnwind;
                // SAFETY: setsid takes no arguments and is async-signal-safe, which is all this
                // forked child may call.
                if unsafe { libc::setsid() } == -1 {
                    // SAFETY: _exit is async-signal-safe and ends the child without running a Rust
                    // destructor.
                    unsafe { libc::_exit(71) }
                }
                launcher.launch()
            }
        };
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

    #[allow(unsafe_code)]
    fn signal_group_and_reap(
        &mut self,
        terminal: VmmState,
        blocking: bool,
    ) -> Result<VmmState, SupervisionError> {
        // SAFETY: kill takes no pointer. Every caller passes a terminal state that a WNOWAIT wait
        // observed without consuming it. The zombie leader then still holds its pid, and no other
        // group can take its number, only while this handle is the only consuming waiter, as
        // `VmmChild`'s doc requires.
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

    #[allow(unsafe_code)]
    fn wait_once(
        &self,
        operation: SupervisionOperation,
        options: libc::c_int,
    ) -> Result<Option<VmmState>, SupervisionError> {
        // SAFETY: siginfo_t is a plain C struct, for which all zeroes is a valid value.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a local siginfo_t that waitid may write, and waitid keeps no pointer.
        if unsafe { libc::waitid(libc::P_PID, self.pid as libc::id_t, &mut info, options) } == -1 {
            return Err(self.error(operation, None));
        }
        // SAFETY: `info` was zeroed and then filled by waitid, so every byte is initialized, and
        // any bit pattern is a valid pid_t.
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
        record_reap(options, reported);
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

    #[allow(unsafe_code)]
    fn signal_leader(&self) -> Result<(), SupervisionError> {
        let signalled = leader_signal_stand_in(self.pid).unwrap_or_else(|| {
            // SAFETY: kill takes no pointer, so the call is memory-safe whatever `self.pid` names.
            // Its callers reach it only after observe found the child running, and no wait of this
            // handle has consumed it since. The pid still names that child because this handle is
            // its only consuming waiter, as `VmmChild`'s doc requires.
            if unsafe { libc::kill(self.pid, libc::SIGKILL) } == -1 {
                Err(last_errno())
            } else {
                Ok(())
            }
        });
        signalled.map_err(|errno| SupervisionError {
            operation: SupervisionOperation::SignalLeader,
            errno,
            observed: None,
        })
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
        note_drop_report();
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
        let line = format!(
            "VmmChild pid {} drop cleanup error: {}; later={later:?}; {disposition}; \
             further signalling forbidden={signalling_forbidden}",
            self.pid, error
        );
        if let Some(line) = unless_a_test_captures(line) {
            let _ = writeln!(std::io::stderr(), "{line}");
        }
    }
}

fn unless_a_test_captures(line: String) -> Option<String> {
    #[cfg(test)]
    let line = unless_this_thread_captures(line)?;
    Some(line)
}

#[cfg(test)]
std::thread_local! {
    static CAPTURED_DROP_REPORTS: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn unless_this_thread_captures(line: String) -> Option<String> {
    let mut line = Some(line);
    let _ = CAPTURED_DROP_REPORTS.try_with(|captured| {
        if let Ok(mut captured) = captured.try_borrow_mut()
            && let Some(lines) = captured.as_mut()
        {
            lines.extend(line.take());
        }
    });
    line
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
    static DROP_REPORTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_drop_report() {
    DROP_REPORTS.set(DROP_REPORTS.get() + 1);
}

#[cfg(not(test))]
fn note_drop_report() {}

#[cfg(test)]
fn drop_reports() -> usize {
    DROP_REPORTS.get()
}

#[cfg(test)]
std::thread_local! {
    static LEADER_SIGNAL_STAND_IN: std::cell::Cell<Option<(libc::pid_t, Result<(), i32>)>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn leader_signal_stand_in(pid: libc::pid_t) -> Option<Result<(), i32>> {
    LEADER_SIGNAL_STAND_IN
        .get()
        .and_then(|(named, outcome)| (named == pid).then_some(outcome))
}

#[cfg(not(test))]
fn leader_signal_stand_in(_pid: libc::pid_t) -> Option<Result<(), i32>> {
    None
}

#[cfg(test)]
struct LeaderSignalStandIn;

#[cfg(test)]
impl LeaderSignalStandIn {
    fn withheld(pid: libc::pid_t) -> LeaderSignalStandIn {
        Self::install(pid, Ok(()))
    }

    fn failing(pid: libc::pid_t, errno: i32) -> LeaderSignalStandIn {
        Self::install(pid, Err(errno))
    }

    fn install(pid: libc::pid_t, outcome: Result<(), i32>) -> LeaderSignalStandIn {
        LEADER_SIGNAL_STAND_IN.set(Some((pid, outcome)));
        LeaderSignalStandIn
    }
}

#[cfg(test)]
impl Drop for LeaderSignalStandIn {
    fn drop(&mut self) {
        LEADER_SIGNAL_STAND_IN.set(None);
    }
}

#[cfg(test)]
std::thread_local! {
    static REAPED_PIDS: std::cell::RefCell<Vec<libc::pid_t>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn record_reap(options: libc::c_int, pid: libc::pid_t) {
    if options & libc::WNOWAIT == 0 {
        REAPED_PIDS.with_borrow_mut(|pids| pids.push(pid));
    }
}

#[cfg(not(test))]
fn record_reap(_options: libc::c_int, _pid: libc::pid_t) {}

#[cfg(test)]
pub(crate) fn reset_reap_record() {
    REAPED_PIDS.with_borrow_mut(Vec::clear);
}

#[cfg(test)]
pub(crate) fn reaped_by_a_handle(pid: libc::pid_t) -> bool {
    REAPED_PIDS.with_borrow(|pids| pids.contains(&pid))
}

#[cfg(test)]
pub(crate) fn take_reap_record() -> Vec<libc::pid_t> {
    REAPED_PIDS.take()
}

#[cfg(test)]
fn assert_reaped_by_handle(pid: libc::pid_t) {
    assert!(
        reaped_by_a_handle(pid),
        "no consuming exact-child wait of a handle on this thread returned pid {pid}"
    );
}

impl Drop for VmmChild {
    fn drop(&mut self) {
        self.drop_teardown();
    }
}

#[allow(unsafe_code)]
fn decode_info(info: &libc::siginfo_t) -> Option<VmmState> {
    match info.si_code {
        libc::CLD_EXITED => Some(VmmState::Exited {
            // SAFETY: `info` is an initialized siginfo_t whose si_code is CLD_EXITED, so waitid
            // filled si_status with the exit status, and any bit pattern is a valid c_int.
            code: unsafe { info.si_status() } & 0xff,
        }),
        libc::CLD_KILLED | libc::CLD_DUMPED => Some(VmmState::Signaled {
            // SAFETY: `info` is an initialized siginfo_t whose si_code is CLD_KILLED or CLD_DUMPED,
            // so waitid filled si_status with the signal, and any bit pattern is a valid c_int.
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
#[allow(unsafe_code)]
unsafe extern "C" {
    /// Reads the process table for the process group `leader` through sysctl. Returns 1 only when
    /// every member is a zombie and `leader` appears exactly once, and 0 otherwise, including on
    /// any error. It takes no pointer and keeps no state. When the group has more than one member
    /// it calls `malloc` and `free`, so it must never run in a forked child, and any call can
    /// change errno.
    fn plasmosome_darwin_group_all_zombies(leader: libc::pid_t) -> libc::c_int;
    /// Runs one of the fixed scenarios 0 to 13 against a table it builds on its own stack, and
    /// returns 1 or 0. It takes no pointer and keeps no state.
    #[cfg(test)]
    fn plasmosome_darwin_group_test_table(
        scenario: libc::c_int,
        leader: libc::pid_t,
    ) -> libc::c_int;
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn darwin_group_all_zombies(leader: libc::pid_t) -> bool {
    // SAFETY: The foreign function takes a pid by value, only reads the process table, and keeps no
    // state.
    unsafe { plasmosome_darwin_group_all_zombies(leader) == 1 }
}

#[cfg(not(target_os = "macos"))]
fn darwin_group_all_zombies(_leader: libc::pid_t) -> bool {
    false
}

struct ExitOnUnwind;

impl Drop for ExitOnUnwind {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: _exit is async-signal-safe and ends the forked child before the unwind goes
        // further.
        unsafe { libc::_exit(70) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::ExecCommand;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    include!("../tests/support/fixture.rs");

    const DEADLINE: Duration = Duration::from_secs(5);
    const SETTLE: Duration = Duration::from_millis(300);

    struct ExitWith(i32);

    #[allow(unsafe_code)]
    // SAFETY: In the child, launch only calls _exit with the code, which is async-signal-safe.
    unsafe impl Launch for ExitWith {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            // SAFETY: _exit takes no pointer and is async-signal-safe, which is all this forked
            // child may call.
            unsafe { libc::_exit(self.0) }
        }
    }

    struct DieBySignal(i32);

    #[allow(unsafe_code)]
    // SAFETY: In the child, launch resets the signal to its default action with signal, raises it
    // and calls _exit if it survives. All three are async-signal-safe.
    unsafe impl Launch for DieBySignal {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            // SAFETY: signal, raise and _exit take no pointer and are async-signal-safe, which is
            // all this forked child may call.
            unsafe {
                libc::signal(self.0, libc::SIG_DFL);
                libc::raise(self.0);
                libc::_exit(99)
            }
        }
    }

    struct SleepForever;

    #[allow(unsafe_code)]
    // SAFETY: In the child, launch only calls pause, forever, which is async-signal-safe.
    unsafe impl Launch for SleepForever {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            loop {
                // SAFETY: pause takes no arguments and is async-signal-safe, which is all this
                // forked child may call.
                unsafe { libc::pause() };
            }
        }
    }

    struct PanicOnLaunch(Box<dyn std::any::Any + Send>);

    #[allow(unsafe_code)]
    // SAFETY: This launcher breaks the no-unwind duty on purpose, as the only witness of exit 70.
    // The parent builds its payload before the fork. resume_unwind skips the panic hook, so it
    // cannot stall on a hook lock another thread held at the fork. The child has its own copy of
    // memory, and ExitOnUnwind exits before any parent fixture's frame unwinds or the payload
    // drops. If the child stalls, its test fails once DEADLINE runs out and the handle's drop
    // kills and reaps it.
    unsafe impl Launch for PanicOnLaunch {
        fn launch(self) -> ! {
            std::panic::resume_unwind(self.0)
        }
    }

    struct PipeEnd(i32);

    impl PipeEnd {
        #[allow(unsafe_code)]
        fn close(mut self) {
            if self.0 >= 0 {
                // SAFETY: `self.0` is a descriptor this PipeEnd owns. It is set to -1 below, so
                // Drop skips it and it is closed once.
                unsafe { libc::close(self.0) };
                self.0 = -1;
            }
        }
    }

    impl Drop for PipeEnd {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            if self.0 >= 0 {
                // SAFETY: `self.0` is a descriptor this PipeEnd owns and has not closed, since
                // close sets it to -1.
                unsafe { libc::close(self.0) };
            }
        }
    }

    impl From<OwnedFd> for PipeEnd {
        fn from(fd: OwnedFd) -> PipeEnd {
            PipeEnd(fd.into_raw_fd())
        }
    }

    fn close_on_exec_ends() -> [OwnedFd; 2] {
        crate::spawn_lock::with_each_descriptor_held(super::signal_pressure::close_on_exec_pipe)
            .expect("a close-on-exec pipe opens")
    }

    fn pipe() -> (PipeEnd, PipeEnd) {
        let [read, write] = close_on_exec_ends();
        (PipeEnd::from(read), PipeEnd::from(write))
    }

    #[test]
    fn fixture_pipe_creation_waits_while_a_fork_holds_the_spawn_lock() {
        let held = crate::spawn_lock::hold_fork_side();
        let (finished, finished_after) = std::sync::mpsc::channel();
        let creating = std::thread::spawn(move || {
            let _ = finished.send(close_on_exec_ends());
        });
        let deadline = Instant::now() + DEADLINE;
        let waited = loop {
            if crate::spawn_lock::waits_for_the_write_side(creating.thread().id()) {
                break true;
            }
            if finished_after.try_recv().is_ok() {
                break false;
            }
            assert!(
                Instant::now() < deadline,
                "pipe creation neither finished nor waited"
            );
            std::thread::yield_now();
        };
        drop(held);
        assert!(
            waited,
            "fixture pipes must wait for the spawn lock's write side"
        );
        finished_after
            .recv_timeout(DEADLINE)
            .expect("pipe creation finishes after the fork side is released");
        creating.join().expect("the pipe creator finishes");
    }

    struct KeepAcrossExec {
        command: ExecCommand,
        kept: [OwnedFd; 2],
    }

    impl Launch for KeepAcrossExec {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            for kept in &self.kept {
                // SAFETY: fcntl with F_SETFD takes integers only and is async-signal-safe, which is
                // all this forked child may call. `self` owns the descriptor and keeps it open.
                if unsafe { libc::fcntl(kept.as_raw_fd(), libc::F_SETFD, 0) } == -1 {
                    // SAFETY: _exit is async-signal-safe and ends the child without running a Rust
                    // destructor.
                    unsafe { libc::_exit(77) }
                }
            }
            self.command.launch()
        }
    }

    const LIVENESS: &str = "worker-liveness";

    struct WorkerFixture {
        child: Option<VmmChild>,
        liveness: PipeEnd,
        release: Option<PipeEnd>,
        dir: tempfile::TempDir,
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

        #[allow(unsafe_code)]
        fn release_live_worker(&mut self) {
            let release = self.release.take().expect("the worker is unreleased");
            let byte = b'x';
            assert_eq!(
                // SAFETY: `byte` is a local and the length 1 matches it. `release` keeps its
                // descriptor open for the call.
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
            if std::thread::panicking() {
                return;
            }
            if holds_open(&self.liveness, Duration::ZERO) {
                self.release_live_worker();
            } else {
                self.release.take();
            }
            self.child.take();
        }
    }

    #[allow(unsafe_code)]
    fn open_liveness(path: &Path) -> PipeEnd {
        let c_path =
            CString::new(path.as_os_str().as_bytes()).expect("the temporary path has no NUL");
        // SAFETY: `c_path` is a CString that lives until this function returns, and open keeps no
        // pointer to it.
        let liveness = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        assert!(liveness >= 0, "the test opens its private FIFO reader");
        PipeEnd(liveness)
    }

    #[allow(unsafe_code)]
    fn spawn_with_worker(leader_exits: bool) -> WorkerFixture {
        let [ready_read, ready_write] = close_on_exec_ends();
        let [control_read, control_write] = close_on_exec_ends();
        let (ready_read, control_write) = (PipeEnd::from(ready_read), PipeEnd::from(control_write));
        let dir = tempfile::tempdir().expect("the fixture owns its directory");
        let liveness_path = dir.path().join(LIVENESS);
        let c_path = CString::new(liveness_path.as_os_str().as_bytes())
            .expect("the temporary path has no NUL");
        assert_eq!(
            // SAFETY: `c_path` is a CString that lives until this function returns, and mkfifo
            // keeps no pointer to it.
            unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) },
            0,
            "the worker liveness FIFO is created"
        );
        let liveness = open_liveness(&liveness_path);
        let mode = if leader_exits { "exit" } else { "wait" };
        let command = ExecCommand::new(vec![
            supervision_fixture().display().to_string(),
            mode.to_string(),
            ready_write.as_raw_fd().to_string(),
            control_read.as_raw_fd().to_string(),
            liveness_path.display().to_string(),
            ready_read.0.to_string(),
            control_write.0.to_string(),
        ])
        .expect("the prebuilt fixture executable resolves before fork");
        let mut child = VmmChild::spawn(KeepAcrossExec {
            command,
            kept: [ready_write, control_read],
        })
        .expect("fork succeeds");
        wait_readable(&ready_read, DEADLINE);
        let mut byte = 0u8;
        // SAFETY: `byte` is a local that read may write, and the length 1 matches it.
        // `ready_read` keeps its descriptor open.
        let read =
            unsafe { libc::read(ready_read.0, &mut byte as *mut u8 as *mut libc::c_void, 1) };
        if read != 1 {
            panic!(
                "the execed worker announces readiness after opening its FIFO; read {read}, leader {:?}",
                child.wait_terminal(DEADLINE)
            );
        }
        assert_eq!(
            byte,
            b'x',
            "the execed leader answers {:?}: it must hold its ready_write and control_read, and \
             never its parent's ready_read ({}) or control_write ({}), which would keep its \
             worker from ever seeing the control pipe close",
            char::from(byte),
            ready_read.0,
            control_write.0
        );
        WorkerFixture {
            child: Some(child),
            liveness,
            release: Some(control_write),
            dir,
        }
    }

    #[allow(unsafe_code)]
    fn wait_readable(pipe: &PipeEnd, patience: Duration) {
        let mut watched = libc::pollfd {
            fd: pipe.0,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        let timeout = libc::c_int::try_from(patience.as_millis()).expect("timeout fits");
        loop {
            // SAFETY: `watched` is one local pollfd, matching the count of 1, and `pipe` keeps its
            // descriptor open.
            let ready = unsafe { libc::poll(&mut watched, 1, timeout) };
            if ready > 0 {
                return;
            }
            assert_ne!(ready, 0, "the descriptor changes within {patience:?}");
            assert_eq!(last_errno(), libc::EINTR);
        }
    }

    #[allow(unsafe_code)]
    fn holds_open(pipe: &PipeEnd, patience: Duration) -> bool {
        let started = Instant::now();
        loop {
            let mut byte = 0u8;
            // SAFETY: `byte` is a local that read may write, and the length 1 matches it. `pipe`
            // keeps its descriptor open.
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

    #[allow(unsafe_code)]
    fn witness_unconsumed_exit(pid: i32, expected: VmmState) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            // SAFETY: siginfo_t is a plain C struct, for which all zeroes is a valid value.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: `info` is a local siginfo_t that waitid may write. Every caller passes a child
            // it spawned before any consuming wait, so `pid` still names it, and WNOWAIT leaves it
            // unreaped.
            let waited = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            assert_eq!(waited, 0, "the exact direct child remains waitable");
            // SAFETY: `info` was zeroed and then filled by waitid, so every byte is initialized,
            // and any bit pattern is a valid pid_t.
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

    #[allow(unsafe_code)]
    fn reap_externally(pid: i32) {
        let mut status = 0;
        loop {
            // SAFETY: `status` is a local the call may write. Every caller first saw `pid` exit
            // through a WNOWAIT wait, which consumes nothing, and nothing has reaped it since, so
            // it is still an unreaped child of this test and its number names it.
            let reaped = unsafe { libc::waitpid(pid, &mut status, 0) };
            if reaped == pid {
                return;
            }
            assert_eq!(last_errno(), libc::EINTR);
        }
    }

    struct DropReports;

    impl DropReports {
        fn capture() -> DropReports {
            CAPTURED_DROP_REPORTS.set(Some(Vec::new()));
            DropReports
        }

        fn take(&self) -> Vec<String> {
            CAPTURED_DROP_REPORTS.with_borrow_mut(|captured| {
                std::mem::take(
                    captured
                        .as_mut()
                        .expect("this thread captures drop reports while the guard lives"),
                )
            })
        }
    }

    impl Drop for DropReports {
        fn drop(&mut self) {
            for line in CAPTURED_DROP_REPORTS.take().unwrap_or_default() {
                let _ = writeln!(std::io::stderr(), "{line}");
            }
        }
    }

    fn assert_one_loss_report(lines: Vec<String>, pid: i32, expected: SupervisionError) {
        let [line] = lines.as_slice() else {
            panic!("expected one drop report for pid {pid}, got {lines:?}");
        };
        assert!(
            line.starts_with(&format!("VmmChild pid {pid} ")),
            "the drop report {line:?} names pid {pid}"
        );
        assert_report_names(line, expected);
    }

    fn assert_report_names(line: &str, expected: SupervisionError) {
        let errno = match expected.observed {
            Some(state) => format!("errno {} after observing {state:?};", expected.errno),
            None => format!("errno {};", expected.errno),
        };
        for fragment in [
            format!("{:?} failed", expected.operation),
            errno,
            "workers may remain live".to_string(),
            "further signalling forbidden=true".to_string(),
        ] {
            assert!(
                line.contains(&fragment),
                "the drop report {line:?} contains {fragment:?}"
            );
        }
        if expected.observed.is_none() {
            assert!(
                !line.contains("after observing"),
                "the drop report {line:?} names no observed exit"
            );
        }
    }

    fn assert_no_report(reports: &DropReports, after: &str) {
        assert_eq!(
            reports.take(),
            Vec::<String>::new(),
            "{after} made a drop report"
        );
    }

    #[cfg(target_os = "macos")]
    struct DarwinOwnedGroup {
        pids: Vec<i32>,
        controls: Vec<Option<PipeEnd>>,
    }

    #[cfg(target_os = "macos")]
    impl DarwinOwnedGroup {
        #[allow(unsafe_code)]
        fn new(count: usize) -> DarwinOwnedGroup {
            let mut group = DarwinOwnedGroup {
                pids: Vec::with_capacity(count),
                controls: Vec::with_capacity(count),
            };
            for index in 0..count {
                let (control_read, control_write) = pipe();
                let (ready_read, ready_write) = pipe();
                let leader = group.pids.first().copied().unwrap_or(0);
                // SAFETY: This test process has other threads. The child runs only close, setpgid,
                // write, read and _exit, which are async-signal-safe, on its own copies of the
                // descriptors and on locals, so it cannot deadlock here. It never execs, so it
                // forks without the spawn lock: that lock only keeps a descriptor from crossing an
                // exec before it is close-on-exec, and this child keeps every descriptor it
                // inherits until it exits, with the lock or without it.
                let pid = unsafe { libc::fork() };
                assert!(pid >= 0, "the certificate fixture fork succeeds");
                if pid == 0 {
                    // SAFETY: This is the forked child. Every call here is async-signal-safe, the
                    // descriptors are its own copies, and `byte` and `release` are locals that
                    // match the length 1.
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
                    // SAFETY: `byte` is a local that read may write, and the length 1 matches it.
                    // `ready_read` keeps its descriptor open.
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

        #[allow(unsafe_code)]
        fn release(&mut self, index: usize) {
            let control = self.controls[index]
                .take()
                .expect("the owned member is unreleased");
            let byte = b'x';
            assert_eq!(
                // SAFETY: `byte` is a local and the length 1 matches it. `control` keeps its
                // descriptor open for the call.
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
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            self.controls.clear();
            for pid in self.pids.drain(..) {
                let mut status = 0;
                loop {
                    // SAFETY: `status` is a local the call may write, and `pid` is a child this
                    // group forked and has not reaped.
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
        reset_reap_record();
        child.kill().expect("signalling a live child succeeds");
        drop(child);
        assert_reaped_by_handle(pid);
    }

    #[test]
    fn drop_reaps_a_running_child_without_orphans() {
        let child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        let pid = child.pid();
        reset_reap_record();
        drop(child);
        assert_reaped_by_handle(pid);
    }

    #[test]
    fn drop_first_cleans_a_naturally_exited_leaders_worker() {
        let mut fixture = spawn_with_worker(true);
        let pid = fixture.child().pid();
        witness_unconsumed_exit(pid, VmmState::Exited { code: 7 });
        reset_reap_record();
        drop(fixture.take_child());
        assert_eof_within(&fixture.liveness, DEADLINE);
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
        assert_reaped_by_handle(pid);
        assert_eq!(fixture.child().state(), Ok(VmmState::Exited { code: 7 }));
        fixture.discard_release();
    }

    #[test]
    fn every_entry_point_fails_closed_after_external_reap() {
        let reports = DropReports::capture();
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
                assert_no_report(&reports, entry);
                drop(fixture.take_child());
                assert!(
                    holds_open(&fixture.liveness, SETTLE),
                    "cached loss signalled during Drop"
                );
            }
            assert_one_loss_report(
                reports.take(),
                pid,
                SupervisionError {
                    operation: SupervisionOperation::Observe,
                    errno: libc::ECHILD,
                    observed: None,
                },
            );
            fixture.release_live_worker();
        }
    }

    #[test]
    fn loss_after_a_peek_preserves_the_exit_and_forbids_future_signals() {
        let reports = DropReports::capture();
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
        assert_no_report(&reports, "state or kill");
        drop(fixture.take_child());
        assert!(holds_open(&fixture.liveness, SETTLE));
        assert_one_loss_report(reports.take(), pid, expected);
        fixture.release_live_worker();
    }

    #[test]
    fn completed_and_lost_handles_do_not_touch_an_owned_sentinel() {
        let reports = DropReports::capture();
        let mut sentinel = spawn_with_worker(false);
        let mut completed = VmmChild::spawn(ExitWith(3)).expect("fork succeeds");
        assert_eq!(
            completed.wait_terminal(DEADLINE),
            Ok(VmmState::Exited { code: 3 })
        );
        assert_eq!(completed.state(), Ok(VmmState::Exited { code: 3 }));
        completed.kill().expect("completed kill is a no-op");
        let mut lost = VmmChild::spawn(ExitWith(4)).expect("fork succeeds");
        let lost_pid = lost.pid();
        witness_unconsumed_exit(lost_pid, VmmState::Exited { code: 4 });
        reap_externally(lost_pid);
        assert_eq!(lost.state(), Ok(VmmState::Lost));
        assert_eq!(lost.state(), Ok(VmmState::Lost));
        assert_no_report(&reports, "a lifecycle call");
        drop(completed);
        assert_no_report(&reports, "dropping a completed handle");
        drop(lost);
        assert_one_loss_report(
            reports.take(),
            lost_pid,
            SupervisionError {
                operation: SupervisionOperation::Observe,
                errno: libc::ECHILD,
                observed: None,
            },
        );
        assert!(
            holds_open(&sentinel.liveness, SETTLE),
            "released identities affected an independently owned group"
        );
        sentinel.child().kill().expect("sentinel cleanup succeeds");
        assert_no_report(&reports, "killing the sentinel");
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
                Box::new(|| {
                    let payload = Box::new("a launcher that breaks its no-unwind contract");
                    VmmChild::spawn(PanicOnLaunch(payload))
                }),
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
        let reports = DropReports::capture();
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
                assert_no_report(&reports, entry);
                drop(fixture.take_child());
                assert!(holds_open(&fixture.liveness, SETTLE));
            }
            assert_one_loss_report(reports.take(), pid, expected);
            fixture.release_live_worker();
        }
    }

    #[test]
    fn leader_signal_esrch_is_not_wait_authority_loss() {
        let reports = DropReports::capture();
        let mut fixture = spawn_with_worker(false);
        let pid = fixture.child().pid();
        reset_reap_record();
        {
            let _stand_in = LeaderSignalStandIn::failing(pid, libc::ESRCH);
            assert_eq!(
                fixture.child().kill(),
                Err(SupervisionError {
                    operation: SupervisionOperation::SignalLeader,
                    errno: libc::ESRCH,
                    observed: None,
                })
            );
            assert_eq!(fixture.child().lifecycle, Lifecycle::Owned);
            assert_eq!(
                take_reap_record(),
                Vec::<libc::pid_t>::new(),
                "a failed leader signal led to a consuming wait"
            );
            assert!(
                holds_open(&fixture.liveness, SETTLE),
                "a failed leader signal still signalled the group"
            );
            assert_eq!(fixture.child().state(), Ok(VmmState::Running));
        }
        fixture
            .child()
            .kill()
            .expect("the real leader signal tears the leader down");
        assert_eof_within(&fixture.liveness, DEADLINE);
        assert_reaped_by_handle(pid);
        assert_eq!(
            fixture.child().state(),
            Ok(VmmState::Signaled {
                signal: libc::SIGKILL
            })
        );
        assert_no_report(&reports, "a failed or a completed kill");
        drop(fixture.take_child());
        assert_no_report(&reports, "dropping a completed handle");
        fixture.discard_release();
    }

    #[test]
    fn a_drop_report_is_kept_only_by_the_thread_that_captures_it() {
        assert_eq!(
            unless_a_test_captures("x".to_string()),
            Some("x".to_string()),
            "with no capture the report goes to stderr"
        );
        {
            let reports = DropReports::capture();
            assert_eq!(
                unless_a_test_captures("x".to_string()),
                None,
                "a capture on this thread keeps the report"
            );
            assert_eq!(reports.take(), vec!["x".to_string()]);
            let elsewhere = std::thread::spawn(|| unless_a_test_captures("y".to_string()))
                .join()
                .expect("the other thread reports without panicking");
            assert_eq!(
                elsewhere,
                Some("y".to_string()),
                "this thread's capture kept another thread's report"
            );
            assert_eq!(reports.take(), Vec::<String>::new());
        }
        assert_eq!(
            unless_a_test_captures("x".to_string()),
            Some("x".to_string()),
            "a dropped guard left its capture installed"
        );
    }

    const DROP_PROBE: &str = "PLASMOSOME_VMM_DROP_PROBE";
    const PROBE_PID: &str = "probe drops lost pid ";
    const PROBE_DEADLINE: Duration = DEADLINE.saturating_mul(2);

    #[test]
    #[ignore = "helper process for an_uncaptured_drop_report_reaches_stderr"]
    fn probe_drop_of_a_lost_handle() {
        if std::env::var_os(DROP_PROBE).is_none() {
            return;
        }
        let mut child = VmmChild::spawn(ExitWith(4)).expect("fork succeeds");
        let pid = child.pid();
        println!("{PROBE_PID}{pid}");
        witness_unconsumed_exit(pid, VmmState::Exited { code: 4 });
        reap_externally(pid);
        assert_eq!(child.state(), Ok(VmmState::Lost));
        drop(child);
    }

    fn read_from_start(mut file: std::fs::File) -> String {
        use std::io::{Read, Seek, SeekFrom};
        let mut text = String::new();
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.read_to_string(&mut text))
            .expect("the probe's output file reads back");
        text
    }

    #[test]
    fn an_uncaptured_drop_report_reaches_stderr() {
        let stdout = tempfile::tempfile().expect("the probe has a file for its stdout");
        let stderr = tempfile::tempfile().expect("the probe has a file for its stderr");
        let mut command = std::process::Command::new(
            std::env::current_exe().expect("the test executable has a path"),
        );
        command
            .args([
                "--exact",
                "vmm::tests::probe_drop_of_a_lost_handle",
                "--ignored",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(DROP_PROBE, "1")
            .stdin(std::process::Stdio::null())
            .stdout(stdout.try_clone().expect("the stdout file is shared"))
            .stderr(stderr.try_clone().expect("the stderr file is shared"));
        let spawned = {
            let _no_pipe_mid_creation = crate::spawn_lock::hold_fork_side();
            command.spawn()
        };
        let mut probe = spawned.expect("the probe process starts");
        let started = Instant::now();
        let status = loop {
            if let Some(status) = probe.try_wait().expect("the probe's status is readable") {
                break status;
            }
            if started.elapsed() >= PROBE_DEADLINE {
                let _ = probe.kill();
                let _ = probe.wait();
                panic!(
                    "the probe did not finish within {PROBE_DEADLINE:?}\n{}\n{}",
                    read_from_start(stdout),
                    read_from_start(stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let said = read_from_start(stdout);
        let reported = read_from_start(stderr);
        assert!(
            status.success(),
            "the probe failed: {status}\n{said}\n{reported}"
        );
        assert!(
            said.contains("test result: ok. 1 passed"),
            "the probe ran its one test\n{said}"
        );
        let pid: i32 = said
            .lines()
            .find_map(|line| line.split_once(PROBE_PID))
            .and_then(|(_, pid)| pid.parse().ok())
            .unwrap_or_else(|| panic!("the probe names its child's pid\n{said}"));
        let lines: Vec<&str> = reported
            .split_inclusive('\n')
            .filter(|line| line.contains("drop cleanup error"))
            .collect();
        let [line] = lines.as_slice() else {
            panic!("expected one drop report on the probe's stderr, got {reported:?}");
        };
        let Some(line) = line.strip_suffix('\n') else {
            panic!("the drop report {line:?} ends with a newline");
        };
        assert!(
            line.starts_with(&format!("VmmChild pid {pid} ")),
            "the drop report {line:?} names pid {pid}"
        );
        assert_report_names(
            line,
            SupervisionError {
                operation: SupervisionOperation::Observe,
                errno: libc::ECHILD,
                observed: None,
            },
        );
    }

    const WORKER_CHILD: &str = "PLASMOSOME_WORKER_FIXTURE_CHILD";
    const WORKER_LINE: &str = "worker fixture leader ";

    #[test]
    #[ignore = "helper process for a_killed_test_process_leaves_no_worker_behind"]
    fn worker_fixture_in_a_child_process() {
        use std::io::Read;
        if std::env::var_os(WORKER_CHILD).is_none() {
            return;
        }
        let mut fixture = std::mem::ManuallyDrop::new(spawn_with_worker(true));
        let leader = fixture.child().pid();
        println!(
            "{WORKER_LINE}{leader} {}",
            fixture.dir.path().join(LIVENESS).display()
        );
        std::io::stdout()
            .flush()
            .expect("the helper's line reaches its parent");
        std::io::stdin()
            .read_to_end(&mut Vec::new())
            .expect("the helper reads its stdin until its parent closes it");
    }

    struct KilledOnDrop(std::process::Child);

    impl Drop for KilledOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    struct GroupKilledWhileLive<'a> {
        group: libc::pid_t,
        liveness: &'a PipeEnd,
    }

    impl Drop for GroupKilledWhileLive<'_> {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            let mut byte = 0u8;
            // SAFETY: `byte` is a local that read may write, and the length 1 matches it.
            // `liveness` keeps its descriptor open.
            let read = unsafe {
                libc::read(
                    self.liveness.0,
                    &mut byte as *mut u8 as *mut libc::c_void,
                    1,
                )
            };
            if read != 0 {
                // SAFETY: kill takes no pointer. The FIFO is not at end of file, so the worker
                // that holds its write end still runs, and the process group named by its
                // leader's pid still has that member, so the number names that group.
                unsafe { libc::kill(-self.group, libc::SIGKILL) };
            }
        }
    }

    fn helper_line(stdout: OwnedFd) -> std::sync::mpsc::Receiver<String> {
        use std::io::BufRead;
        let (sender, named) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(fs::File::from(stdout))
                .lines()
                .map_while(Result::ok)
            {
                if let Some((_, named)) = line.split_once(WORKER_LINE) {
                    let _ = sender.send(named.to_string());
                }
            }
        });
        named
    }

    #[test]
    fn a_killed_test_process_leaves_no_worker_behind() {
        use std::os::unix::process::ExitStatusExt;
        let root = tempfile::tempdir().expect("the test owns a temporary root for its helper");
        let stderr = tempfile::tempfile().expect("the helper has a file for its stderr");
        let [stdin, _stdin_held] = close_on_exec_ends();
        let [said, stdout] = close_on_exec_ends();
        let mut command =
            Command::new(std::env::current_exe().expect("the test executable has a path"));
        command
            .args([
                "--exact",
                "vmm::tests::worker_fixture_in_a_child_process",
                "--ignored",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(WORKER_CHILD, "1")
            .env("TMPDIR", root.path())
            .stdin(stdin)
            .stdout(stdout)
            .stderr(stderr.try_clone().expect("the stderr file is shared"));
        let spawned = {
            let _no_pipe_mid_creation = crate::spawn_lock::hold_fork_side();
            command.spawn()
        };
        drop(command);
        let mut helper = KilledOnDrop(spawned.expect("the helper test process starts"));
        let named = helper_line(said)
            .recv_timeout(PROBE_DEADLINE)
            .unwrap_or_else(|error| {
                panic!(
                    "the helper did not name its worker within {PROBE_DEADLINE:?} ({error})\n{}",
                    read_from_start(stderr)
                )
            });
        let (leader, path) = named
            .split_once(' ')
            .unwrap_or_else(|| panic!("the helper names a pid and a path: {named:?}"));
        let leader: libc::pid_t = leader
            .parse()
            .unwrap_or_else(|_| panic!("the helper names its leader's pid: {named:?}"));
        assert!(
            Path::new(path).starts_with(root.path()),
            "the helper's fixture directory {path} is inside the test's root {}, so it is removed with it",
            root.path().display()
        );
        let liveness = open_liveness(Path::new(path));
        assert!(
            holds_open(&liveness, Duration::ZERO),
            "the helper's worker holds its liveness FIFO before the helper is killed"
        );
        let _orphan = GroupKilledWhileLive {
            group: leader,
            liveness: &liveness,
        };
        helper.0.kill().expect("the helper is killed by its pid");
        let status = helper.0.wait().expect("the killed helper is reaped");
        assert_eq!(
            status.signal(),
            Some(libc::SIGKILL),
            "the helper ended by the test's SIGKILL, not on its own: {status}"
        );
        assert_eof_within(&liveness, DEADLINE);
        root.close()
            .expect("the helper's root, and the fixture directory inside it, are removed");
    }

    #[test]
    fn the_leader_signal_stand_in_answers_only_for_its_pid_on_its_thread_until_dropped() {
        assert_eq!(leader_signal_stand_in(4000), None);
        {
            let _stand_in = LeaderSignalStandIn::failing(4000, libc::ESRCH);
            assert_eq!(leader_signal_stand_in(4000), Some(Err(libc::ESRCH)));
            assert_eq!(leader_signal_stand_in(4001), None);
            let elsewhere = std::thread::spawn(|| leader_signal_stand_in(4000))
                .join()
                .expect("the other thread asks without panicking");
            assert_eq!(elsewhere, None);
        }
        assert_eq!(leader_signal_stand_in(4000), None);
        let _withheld = LeaderSignalStandIn::withheld(4000);
        assert_eq!(leader_signal_stand_in(4000), Some(Ok(())));
    }

    #[test]
    fn the_leader_signal_stand_in_clears_when_its_holder_panics() {
        let unwound = std::panic::catch_unwind(|| {
            let _stand_in = LeaderSignalStandIn::failing(4000, libc::ESRCH);
            panic!("an assertion fails while the stand-in is installed");
        });
        assert!(unwound.is_err(), "the holder panicked");
        assert_eq!(
            leader_signal_stand_in(4000),
            None,
            "the unwind left the stand-in installed"
        );
    }

    #[test]
    fn the_reap_record_keeps_only_waits_that_consume() {
        let mut child = VmmChild::spawn(ExitWith(5)).expect("fork succeeds");
        let pid = child.pid();
        reset_reap_record();
        witness_unconsumed_exit(pid, VmmState::Exited { code: 5 });
        assert_eq!(child.observe(), Ok(VmmState::Exited { code: 5 }));
        assert_eq!(
            child.wait_once(
                SupervisionOperation::Reap,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT
            ),
            Ok(Some(VmmState::Exited { code: 5 })),
            "a WNOWAIT wait still sees the exit"
        );
        assert!(
            !reaped_by_a_handle(pid),
            "a WNOWAIT observation consumed nothing, so it is not a reap"
        );
        assert_eq!(child.state(), Ok(VmmState::Exited { code: 5 }));
        assert_reaped_by_handle(pid);
    }

    #[test]
    #[allow(unsafe_code)]
    fn immediate_teardown_while_child_is_pre_setsid_reaps_exact_child() {
        let (gate_read, gate_write) = pipe();
        // SAFETY: This test process has other threads. The child runs only close, read, setsid,
        // pause and _exit, which are async-signal-safe, on its own copies of the descriptors and on
        // a local, so it cannot deadlock here. It never execs, so it forks without the spawn lock:
        // that lock only keeps a descriptor from crossing an exec before it is close-on-exec, and
        // this child keeps every descriptor it inherits until it dies, with the lock or without
        // it.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "the instrumented fork succeeds");
        if pid == 0 {
            // SAFETY: This is the forked child. Every call here is async-signal-safe, `byte` is a
            // local that matches the length 1, and the descriptors are its own copies.
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
        reset_reap_record();
        let mut child = VmmChild {
            pid,
            lifecycle: Lifecycle::Owned,
        };
        child.kill().expect("pre-setsid teardown completes");
        assert_reaped_by_handle(pid);
        gate_write.close();
    }

    #[test]
    fn a_worker_fixture_dropped_by_a_panic_panics_no_second_time_and_ends_its_worker() {
        let mut release = None;
        let mut liveness = None;
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut fixture = spawn_with_worker(true);
            liveness = Some(open_liveness(&fixture.dir.path().join(LIVENESS)));
            release = fixture.release.take();
            panic!("a test fails while its fixture's worker runs and is not released");
        }));
        assert!(unwound.is_err(), "the test body panicked");
        assert_eof_within(
            liveness.as_ref().expect("the body opened a reader"),
            DEADLINE,
        );
        assert!(
            release.is_some(),
            "the body kept the control pipe's write end, so only the fixture's teardown ended the worker"
        );
    }

    #[test]
    fn immediate_drop_kills_execed_leader_and_worker() {
        let mut fixture = spawn_with_worker(false);
        let pid = fixture.child().pid();
        reset_reap_record();
        drop(fixture.take_child());
        assert_eof_within(&fixture.liveness, DEADLINE);
        assert_reaped_by_handle(pid);
        fixture.discard_release();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[allow(unsafe_code)]
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
                // SAFETY: The foreign function takes integers only and reads a table on its own
                // stack.
                unsafe { plasmosome_darwin_group_test_table(scenario, 4000) },
                1,
                "certificate scenario {scenario} is valid"
            );
        }
        for scenario in [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13] {
            assert_eq!(
                // SAFETY: The foreign function takes integers only and reads a table on its own
                // stack.
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
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;

    const CHILDREN: usize = 64;
    const INTERRUPTIONS_PER_REAP: usize = 2;
    const SIGNALS_PER_REAP: usize = 512;
    const YIELDS_PER_SIGNAL: usize = 1_000_000;

    static DELIVERED: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn note_signal(_sig: libc::c_int) {
        DELIVERED.fetch_add(1, Ordering::Relaxed);
    }

    struct ExitOnRelease {
        read: libc::c_int,
        write: libc::c_int,
    }

    #[allow(unsafe_code)]
    // SAFETY: In the child, launch closes its copy of the release pipe's write end, reads one byte
    // from the read end into a local and calls _exit. All three calls are async-signal-safe.
    unsafe impl Launch for ExitOnRelease {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            let mut byte = 0u8;
            // SAFETY: close, read and _exit are async-signal-safe, which is all this forked child
            // may call. Both descriptors are the child's own copies. Closing its copy of the write
            // end means read sees end of file once the test process is gone. `byte` is a local
            // that read may write, and the length 1 matches it.
            unsafe {
                libc::close(self.write);
                libc::read(self.read, &mut byte as *mut u8 as *mut libc::c_void, 1);
                libc::_exit(0)
            }
        }
    }

    struct Release {
        read: Option<OwnedFd>,
        write: OwnedFd,
    }

    impl Release {
        fn open() -> Release {
            let [read, write] = crate::spawn_lock::with_each_descriptor_held(close_on_exec_pipe)
                .expect("a close-on-exec release pipe opens");
            Release {
                read: Some(read),
                write,
            }
        }

        fn read_end(&self) -> libc::c_int {
            self.read
                .as_ref()
                .expect("the read end is open until the leader is forked")
                .as_raw_fd()
        }

        fn write_end(&self) -> libc::c_int {
            self.write.as_raw_fd()
        }

        fn close_read(&mut self) {
            self.read = None;
        }
    }

    #[allow(unsafe_code)]
    pub(super) fn close_on_exec_pipe(
        _held: &crate::spawn_lock::DescriptorsHeld,
    ) -> std::io::Result<[OwnedFd; 2]> {
        let mut fds = [0; 2];
        // SAFETY: `fds` is a local array of two c_int, the size pipe writes.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: pipe returned 0, so both entries are new descriptors, and nothing else owns them.
        let ends = fds.map(|fd| unsafe { OwnedFd::from_raw_fd(fd) });
        for end in &ends {
            // SAFETY: fcntl with F_SETFD takes integers only, and `end` keeps its descriptor open
            // for the call. with_each_descriptor_held checks the flag afterwards.
            unsafe { libc::fcntl(end.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        Ok(ends)
    }

    struct InterruptingHandler {
        previous: libc::sigaction,
    }

    impl InterruptingHandler {
        #[allow(unsafe_code)]
        fn install() -> InterruptingHandler {
            // SAFETY: libc::sigaction is a plain C struct, for which all zeroes is a valid value.
            let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
            // SAFETY: An all-zero sigaction is a valid empty action. The handler only adds to an
            // atomic, which is async-signal-safe. `action` and `previous` are locals that outlive
            // the calls.
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
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: `self.previous` is the action sigaction saved at install, and sigaction only
            // reads it.
            unsafe { libc::sigaction(libc::SIGUSR1, &self.previous, std::ptr::null_mut()) };
        }
    }

    #[allow(unsafe_code)]
    fn set_sigusr1_blocked(blocked: bool) {
        // SAFETY: All zeroes is a valid sigset_t, and sigemptyset then sets it. `set` is a local
        // that outlives the calls, and pthread_sigmask changes only this thread's mask.
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

    struct Aim {
        release: libc::c_int,
        dropped: Arc<AtomicBool>,
    }

    struct Interrupter {
        aims: Option<mpsc::Sender<Aim>>,
        released: mpsc::Receiver<Option<usize>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Interrupter {
        #[allow(unsafe_code)]
        fn aimed_at_this_thread() -> Interrupter {
            // SAFETY: pthread_self takes no arguments and cannot fail.
            let target = unsafe { libc::pthread_self() } as usize;
            let (aims, aimed) = mpsc::channel::<Aim>();
            let (releases, released) = mpsc::channel();
            let thread = std::thread::spawn(move || {
                for aim in aimed {
                    let release = ReleaseOnDrop(aim.release);
                    let unhandled = interrupt(target, &aim);
                    drop(release);
                    if releases.send(unhandled).is_err() {
                        return;
                    }
                }
            });
            set_sigusr1_blocked(true);
            Interrupter {
                aims: Some(aims),
                released,
                thread: Some(thread),
            }
        }

        fn drop_interrupted(&self, child: VmmChild, release: &Release) -> Option<usize> {
            let dropped = Arc::new(AtomicBool::new(false));
            self.aims
                .as_ref()
                .expect("the interrupter takes aims until it is dropped")
                .send(Aim {
                    release: release.write_end(),
                    dropped: Arc::clone(&dropped),
                })
                .expect("the interrupter thread is running");
            let _withheld = LeaderSignalStandIn::withheld(child.pid());
            set_sigusr1_blocked(false);
            drop(child);
            set_sigusr1_blocked(true);
            dropped.store(true, Ordering::Relaxed);
            self.released
                .recv()
                .expect("the interrupter releases every leader it was aimed at")
        }
    }

    impl Drop for Interrupter {
        fn drop(&mut self) {
            self.aims.take();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            set_sigusr1_blocked(false);
        }
    }

    struct ReleaseOnDrop(libc::c_int);

    impl Drop for ReleaseOnDrop {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            let byte = b'x';
            // SAFETY: `byte` is a local and the length 1 matches it. The descriptor stays open
            // while this guard lives, because the Release it belongs to outlives the test
            // thread's wait for this aim's report, and that report or the end of this thread
            // comes only after the guard is dropped.
            unsafe { libc::write(self.0, &byte as *const u8 as *const libc::c_void, 1) };
        }
    }

    #[allow(unsafe_code)]
    fn interrupt(target: usize, aim: &Aim) -> Option<usize> {
        let interruptions_before = interrupted_reaps();
        let mut signals = 0;
        let mut unhandled = None;
        while unhandled.is_none()
            && interrupted_reaps() - interruptions_before < INTERRUPTIONS_PER_REAP
            && signals < SIGNALS_PER_REAP
            && !aim.dropped.load(Ordering::Relaxed)
        {
            let delivered = DELIVERED.load(Ordering::Relaxed);
            // SAFETY: `target` is the thread that made the interrupter. Its one caller keeps the
            // interrupter a local of that thread, so its Drop runs there and joins this thread
            // first. The target is therefore alive for every signal.
            if unsafe { libc::pthread_kill(target as libc::pthread_t, libc::SIGUSR1) } != 0 {
                break;
            }
            signals += 1;
            let mut yields = 0;
            while DELIVERED.load(Ordering::Relaxed) == delivered
                && !aim.dropped.load(Ordering::Relaxed)
            {
                if yields == YIELDS_PER_SIGNAL {
                    unhandled = Some(signals);
                    break;
                }
                std::thread::yield_now();
                yields += 1;
            }
        }
        unhandled
    }

    fn assert_reaped_by_drop(pid: libc::pid_t, interruptions: usize) {
        assert!(
            reaped_by_a_handle(pid),
            "pid {pid} survived a drop whose blocking wait was interrupted {interruptions} times"
        );
    }

    #[test]
    fn a_reap_interrupted_by_a_signal_still_leaves_no_orphan() {
        let _handler = InterruptingHandler::install();
        let interrupter = Interrupter::aimed_at_this_thread();

        for child_number in 1..=CHILDREN {
            let mut release = Release::open();
            let child = VmmChild::spawn(ExitOnRelease {
                read: release.read_end(),
                write: release.write_end(),
            })
            .expect("fork succeeds");
            release.close_read();
            let pid = child.pid();
            let signals_before = DELIVERED.load(Ordering::Relaxed);
            let interruptions_before = interrupted_reaps();
            reset_reap_record();
            let reports_before = drop_reports();
            let unhandled = interrupter.drop_interrupted(child, &release);
            let reports = drop_reports() - reports_before;
            let signals = DELIVERED.load(Ordering::Relaxed) - signals_before;
            let interruptions = interrupted_reaps() - interruptions_before;
            assert_reaped_by_drop(pid, interruptions);
            assert_eq!(
                reports, 0,
                "child {child_number}: the drop reported a cleanup error"
            );
            if let Some(signal) = unhandled {
                panic!(
                    "child {child_number}: SIGUSR1 number {signal} was not handled by the reaping thread within {YIELDS_PER_SIGNAL} yields, so the interrupter released the leader early; something in the drop kept the signal blocked or ignored"
                );
            }
            assert!(
                interruptions >= INTERRUPTIONS_PER_REAP,
                "child {child_number}: the reaping thread took {signals} SIGUSR1 and {interruptions} of them interrupted its blocking wait, fewer than {INTERRUPTIONS_PER_REAP}, so the retry after EINTR was not exercised"
            );
        }
    }
}
