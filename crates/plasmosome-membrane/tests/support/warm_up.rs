use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::LazyLock;
use std::thread;
use std::time::{Duration, Instant};

const DAEMON: &str = env!("CARGO_BIN_EXE_membraned");
const BOUND: Duration = Duration::from_secs(60);

static WARM_UP: LazyLock<Option<ExitStatus>> = LazyLock::new(|| {
    let mut child = Command::new(DAEMON)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("membraned starts for its warm-up");
    exit_within(&mut child, BOUND)
});

/// Takes nothing and returns a command for the membraned under test, with no
/// arguments set.
///
/// macOS charges the first exec of a freshly linked binary up to seconds
/// under load. The first call in a test process runs membraned once with no
/// arguments, so that cost is spent before any test deadline starts. Every
/// call panics unless that run exited with the usage status 2; a run still
/// going after 60 seconds is killed and fails the same way.
pub fn membraned() -> Command {
    exited_with_usage("membraned", *WARM_UP);
    Command::new(DAEMON)
}

/// Takes a daemon's name and the outcome of its warm-up run, where `None`
/// means the run did not exit within its bound, and panics unless the run
/// exited with status 2.
pub fn exited_with_usage(daemon: &str, outcome: Option<ExitStatus>) {
    let Some(status) = outcome else {
        panic!("warm-up of {daemon} did not exit within {BOUND:?}");
    };
    assert_eq!(
        status.code(),
        Some(2),
        "warm-up of {daemon} exits with its usage status 2, not {status}"
    );
}

/// Takes a running child and a bound, and returns the child's exit status,
/// or `None` after killing and reaping a child still running at the bound.
pub fn exit_within(child: &mut Child, bound: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + bound;
    loop {
        if let Some(status) = child.try_wait().expect("the warm-up's state is readable") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn a_warm_up_still_running_at_its_bound_is_killed_and_reaped() {
    let mut child = Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("sleep starts");

    assert_eq!(exit_within(&mut child, Duration::from_millis(50)), None);
    let reaped = child
        .try_wait()
        .expect("the child's state is readable")
        .expect("the child was reaped");
    assert_eq!(reaped.signal(), Some(libc::SIGKILL), "{reaped}");
}

#[test]
#[should_panic(expected = "warm-up of membraned did not exit within 60s")]
fn a_warm_up_that_did_not_exit_fails_setup() {
    exited_with_usage("membraned", None);
}

#[test]
#[should_panic(expected = "warm-up of membraned exits with its usage status 2, not exit status: 3")]
fn a_warm_up_that_exits_otherwise_fails_setup() {
    exited_with_usage("membraned", Some(ExitStatus::from_raw(3 << 8)));
}

#[test]
#[should_panic(expected = "warm-up of membraned exits with its usage status 2, not signal: 9")]
fn a_warm_up_killed_by_a_signal_fails_setup() {
    exited_with_usage("membraned", Some(ExitStatus::from_raw(libc::SIGKILL)));
}
