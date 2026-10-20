use std::io;
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
    exit_within(&mut child, BOUND).expect("the warm-up's state is readable")
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
pub fn exit_within(child: &mut Child, bound: Duration) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + bound;
    loop {
        if let Some(status) = child.try_wait().expect("the warm-up's state is readable") {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// Takes a daemon's name, the command that runs it with no arguments and a
/// bound, and returns `Ok` only when that run exited with status 2 within the
/// bound; otherwise it returns the one message every caller reports.
pub fn run_once(daemon: &str, command: Command, bound: Duration) -> Result<(), String> {
    let _ = (daemon, command, bound);
    todo!("the warm-up's single run")
}

#[test]
fn a_warm_up_still_running_at_its_bound_is_killed_and_reaped() {
    let mut child = Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("sleep starts");

    assert!(matches!(
        exit_within(&mut child, Duration::from_millis(50)),
        Ok(None)
    ));
    let reaped = child
        .try_wait()
        .expect("the child's state is readable")
        .expect("the child was reaped");
    assert_eq!(reaped.signal(), Some(libc::SIGKILL), "{reaped}");
}

fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    command
}

#[test]
fn the_usage_exit_passes_the_warm_up() {
    assert_eq!(run_once("membraned", shell("exit 2"), BOUND), Ok(()));
}

#[test]
fn a_warm_up_that_never_exits_fails_within_its_bound() {
    let bound = Duration::from_millis(200);
    let mut sleeper = Command::new("sleep");
    sleeper.arg("60");

    let started = Instant::now();
    let outcome = run_once("membraned", sleeper, bound);
    let took = started.elapsed();

    assert_eq!(
        outcome,
        Err("warm-up of membraned did not exit within 200ms; its stderr: \"\"".to_string())
    );
    assert!(
        took < Duration::from_secs(10),
        "the warm-up gave up after {took:?}, not near its {bound:?} bound"
    );
}

#[test]
fn a_warm_up_that_cannot_start_fails_setup_with_the_cause() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");
    let missing = Command::new(directory.path().join("absent"));

    let failure =
        run_once("membraned", missing, BOUND).expect_err("a missing executable fails the warm-up");

    assert!(
        failure.starts_with("warm-up of membraned could not start: "),
        "{failure}"
    );
    assert!(failure.contains("No such file or directory"), "{failure}");
}

#[test]
fn a_warm_up_that_exits_with_another_code_fails_setup() {
    assert_eq!(
        run_once("membraned", shell("exit 3"), BOUND),
        Err(
            "warm-up of membraned exits with its usage status 2, not exit status: 3; its stderr: \"\""
                .to_string()
        )
    );
}

#[test]
fn a_warm_up_killed_by_a_signal_fails_setup() {
    let failure = run_once("membraned", shell("kill -9 $$"), BOUND)
        .expect_err("a death by signal fails the warm-up");

    assert!(
        failure.starts_with("warm-up of membraned exits with its usage status 2, not signal: 9"),
        "{failure}"
    );
}

#[test]
fn a_failed_warm_up_reports_its_stderr_up_to_4096_bytes() {
    let noisy = shell("printf 'loader refused' >&2; printf '%010000d' 0 | tr 0 Q >&2; exit 1");

    let failure = run_once("membraned", noisy, BOUND).expect_err("exit 1 fails the warm-up");

    assert!(
        failure.contains("not exit status: 1; its stderr: \"loader refusedQ"),
        "{failure}"
    );
    assert_eq!(
        failure.matches('Q').count(),
        4096 - "loader refused".len(),
        "the report keeps the first 4096 bytes of stderr and no more"
    );
}

#[test]
fn an_unreadable_child_state_is_reported_without_a_panic() {
    let mut child = Command::new("true").spawn().expect("true starts");
    let pid = child.id() as libc::pid_t;
    let mut raw = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid, &mut raw, 0) },
        pid,
        "the test reaps the child behind its handle's back"
    );

    let unreadable =
        exit_within(&mut child, BOUND).expect_err("a child reaped elsewhere has no readable state");

    assert_eq!(
        unreadable.raw_os_error(),
        Some(libc::ECHILD),
        "{unreadable}"
    );
}
