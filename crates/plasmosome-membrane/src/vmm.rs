use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// The body of a forked VMM child.
pub trait Launch {
    /// Runs inside the forked child and must never return. The parent is
    /// multi-threaded under the test harness, so an implementation may only
    /// use async-signal-safe calls: `libc::_exit`, `libc::pause`, raw syscalls.
    fn launch(self) -> !;
}

/// What the supervisor last observed about its VMM child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmState {
    Running,
    Exited { code: i32 },
    Signaled { signal: i32 },
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
/// dropped handle never leaves an orphan behind.
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
            launcher.launch()
        }
        Ok(VmmChild {
            pid,
            terminal: None,
        })
    }

    /// The child's process id.
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
        if reaped == 0 {
            return VmmState::Running;
        }
        let terminal = decode(status);
        self.terminal = Some(terminal);
        terminal
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
        if unsafe { libc::waitpid(self.pid, &mut status, 0) } == self.pid {
            self.terminal = Some(decode(status));
        }
    }
}

fn decode(status: libc::c_int) -> VmmState {
    if libc::WIFEXITED(status) {
        return VmmState::Exited {
            code: libc::WEXITSTATUS(status),
        };
    }
    if libc::WIFSIGNALED(status) {
        return VmmState::Signaled {
            signal: libc::WTERMSIG(status),
        };
    }
    VmmState::Running
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
