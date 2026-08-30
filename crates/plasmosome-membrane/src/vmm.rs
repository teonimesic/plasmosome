use std::time::Duration;

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
    /// The child was reaped by something outside this handle; its exit status
    /// is unknowable.
    Lost,
}

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

/// An owned VMM child process. Dropping it kills and reaps the child, so a
/// dropped handle never leaves an orphan behind. The guarantee holds only if
/// the handle is dropped: `mem::forget` leaks a running child.
pub struct VmmChild {
    pid: libc::pid_t,
    terminal: Option<VmmState>,
}

impl VmmChild {
    /// Forks and runs `launcher` in the child, which never returns. The
    /// returned handle owns the child until it is dropped.
    pub fn spawn(launcher: impl Launch) -> Result<VmmChild, SpawnError> {
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(SpawnError::ForkFailed(std::io::Error::last_os_error()));
        }
        if pid == 0 {
            let _guard = ExitOnUnwind;
            launcher.launch()
        }
        Ok(VmmChild {
            pid,
            terminal: None,
        })
    }

    /// The child's process id. Once a terminal state has been observed the pid
    /// is invalid and may already have been reused by an unrelated process, so
    /// a caller must not signal or wait on it.
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Polls the child without blocking. Once a terminal state is observed it
    /// is cached and returned forever after, so the pid is never waited on
    /// twice.
    pub fn state(&mut self) -> VmmState {
        if let Some(terminal) = self.terminal {
            return terminal;
        }
        let mut status: libc::c_int = 0;
        let reaped = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
        if reaped == self.pid {
            let Some(terminal) = decode(status) else {
                return VmmState::Running;
            };
            self.terminal = Some(terminal);
            return terminal;
        }
        if reaped == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
            self.terminal = Some(VmmState::Lost);
            return VmmState::Lost;
        }
        VmmState::Running
    }

    /// Polls until the child is terminal or `deadline` elapses, returning the
    /// last state observed.
    pub fn wait_terminal(&mut self, deadline: Duration) -> VmmState {
        let started = std::time::Instant::now();
        loop {
            let state = self.state();
            if state != VmmState::Running || started.elapsed() >= deadline {
                return state;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Sends `SIGKILL` to the child. A no-op once the child is terminal — a
    /// reaped pid may already belong to someone else.
    pub fn kill(&mut self) -> std::io::Result<()> {
        if self.terminal.is_some() {
            return Ok(());
        }
        if unsafe { libc::kill(self.pid, libc::SIGKILL) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for VmmChild {
    fn drop(&mut self) {
        if self.terminal.is_some() {
            return;
        }
        unsafe { libc::kill(self.pid, libc::SIGKILL) };
        let mut status: libc::c_int = 0;
        loop {
            let reaped = unsafe { libc::waitpid(self.pid, &mut status, 0) };
            if reaped == self.pid {
                self.terminal = decode(status);
                return;
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                return;
            }
        }
    }
}

fn decode(status: libc::c_int) -> Option<VmmState> {
    if libc::WIFEXITED(status) {
        return Some(VmmState::Exited {
            code: libc::WEXITSTATUS(status),
        });
    }
    if libc::WIFSIGNALED(status) {
        return Some(VmmState::Signaled {
            signal: libc::WTERMSIG(status),
        });
    }
    None
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

    const DEADLINE: Duration = Duration::from_secs(5);

    struct ExitWith(i32);

    impl Launch for ExitWith {
        fn launch(self) -> ! {
            unsafe { libc::_exit(self.0) }
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

    #[test]
    fn a_child_that_exits_reports_exited_with_its_code() {
        let mut child = VmmChild::spawn(ExitWith(7)).expect("fork succeeds");
        assert_eq!(child.wait_terminal(DEADLINE), VmmState::Exited { code: 7 });
    }

    #[test]
    fn a_live_child_reports_running() {
        let mut child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        assert_eq!(child.state(), VmmState::Running);
        child.kill().expect("signalling a live child succeeds");
    }

    #[test]
    fn kill_moves_a_running_child_to_signaled() {
        let mut child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        child.kill().expect("signalling a live child succeeds");
        assert_eq!(
            child.wait_terminal(DEADLINE),
            VmmState::Signaled {
                signal: libc::SIGKILL
            }
        );
    }

    #[test]
    fn state_is_stable_after_reap() {
        let mut child = VmmChild::spawn(ExitWith(3)).expect("fork succeeds");
        let terminal = child.wait_terminal(DEADLINE);
        assert_eq!(terminal, VmmState::Exited { code: 3 });
        assert_eq!(child.state(), terminal);
        assert_eq!(child.state(), terminal);
    }

    #[test]
    fn kill_after_exit_is_a_no_op() {
        let mut child = VmmChild::spawn(ExitWith(0)).expect("fork succeeds");
        assert_eq!(child.wait_terminal(DEADLINE), VmmState::Exited { code: 0 });
        child.kill().expect("killing a reaped child is a no-op");
        assert_eq!(child.state(), VmmState::Exited { code: 0 });
    }

    #[test]
    fn a_panicking_launcher_does_not_escape_the_child() {
        let mut child = VmmChild::spawn(PanicOnLaunch).expect("fork succeeds");
        assert_eq!(child.wait_terminal(DEADLINE), VmmState::Exited { code: 70 });
    }

    #[test]
    fn kill_then_immediate_drop_leaves_no_orphan() {
        let mut child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        let pid = child.pid();
        child.kill().expect("signalling a live child succeeds");
        drop(child);
        let mut status: libc::c_int = 0;
        let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        assert_eq!(
            reaped, -1,
            "drop after kill must leave no reapable child behind"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }

    #[test]
    fn drop_of_an_exited_but_unreaped_child_is_clean() {
        let child = VmmChild::spawn(ExitWith(3)).expect("fork succeeds");
        let pid = child.pid();
        std::thread::sleep(Duration::from_millis(200));
        drop(child);
        let mut status: libc::c_int = 0;
        let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        assert_eq!(
            reaped, -1,
            "dropping an exited but unreaped child must reap it"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }

    #[test]
    fn drop_reaps_a_running_child_without_orphans() {
        let child = VmmChild::spawn(SleepForever).expect("fork succeeds");
        let pid = child.pid();
        drop(child);
        let mut status: libc::c_int = 0;
        let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        assert_eq!(reaped, -1, "drop must leave no reapable child behind");
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}
