use std::io;
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

/// Setup allows first-exec checks to finish before any protocol deadline starts.
pub const SETUP_BOUND: Duration = Duration::from_secs(60);

/// Waits for the retained child within `bound`. A timeout returns `None` after killing and
/// reaping it. Observation errors also clean up unless another waiter already reaped it.
pub fn exit_within(child: &mut Child, bound: Duration) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + bound;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) => {}
            Err(error) => {
                if error.raw_os_error() != Some(libc::ECHILD) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return Err(error);
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[allow(unsafe_code)]
fn hanging_setup_is_bounded_killed_and_reaped() {
    use std::os::unix::process::ExitStatusExt;
    let mut child = super::spawn_guarded(std::process::Command::new("sleep").arg("1")).unwrap();
    let started = Instant::now();
    let outcome = exit_within(&mut child, Duration::from_millis(50));
    let mut raw = 0;
    // SAFETY: raw is writable local storage. This exact-child wait sends no signal.
    let waited = unsafe { libc::waitpid(child.id() as libc::pid_t, &mut raw, libc::WNOHANG) };
    let error = io::Error::last_os_error();
    assert_eq!(waited, -1, "exit_within already reaped its child");
    assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
    let reaped = child.try_wait().unwrap();
    assert!(
        matches!(outcome, Ok(None)),
        "setup exceeded its bound: {outcome:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "setup wait blocked"
    );
    assert_eq!(
        reaped.expect("setup child was reaped").signal(),
        Some(libc::SIGKILL)
    );
}
