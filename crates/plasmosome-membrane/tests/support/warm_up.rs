use std::fs::File;
use std::io::{self, Read, Seek};
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

#[path = "owned_wait.rs"]
mod owned_wait;
use owned_wait::{SETUP_BOUND, exit_within};

const DAEMON: &str = env!("CARGO_BIN_EXE_membraned");
const BOUND: Duration = SETUP_BOUND;
const STDERR_LIMIT: u64 = 4096;

fn spawn_guarded(command: &mut Command) -> io::Result<Child> {
    command.spawn()
}

static WARM_UP: LazyLock<Result<(), String>> =
    LazyLock::new(|| run_once("membraned", Command::new(DAEMON), BOUND));

/// Takes nothing and returns a command for the membraned under test, with no
/// arguments set.
///
/// macOS charges the first exec of a binary file it has not run before up to
/// seconds under load, and cargo writes `target/debug/membraned` again on every
/// run, so every test run pays that cost. The first call in a test process
/// runs membraned once with no arguments, so the cost is spent before any test
/// deadline starts. A run still going after 60 seconds is killed and reaped.
/// Every call panics unless that run exited with the usage status 64, and every
/// call reports a failure with the same message, which names the cause and up
/// to 4096 bytes of the run's stderr.
pub fn membraned() -> Command {
    if let Err(failure) = &*WARM_UP {
        panic!("{failure}");
    }
    Command::new(DAEMON)
}

fn run_once(daemon: &str, mut command: Command, bound: Duration) -> Result<(), String> {
    let no_file =
        |error: io::Error| format!("warm-up of {daemon} has no file for its stderr: {error}");
    let mut stderr = tempfile::tempfile().map_err(no_file)?;
    let writer = stderr.try_clone().map_err(no_file)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(writer)
        .spawn()
        .map_err(|error| format!("warm-up of {daemon} could not start: {error}"))?;
    let outcome = exit_within(&mut child, bound);
    let said = first_bytes(&mut stderr);
    match outcome {
        Ok(Some(status)) if status.code() == Some(64) => Ok(()),
        Ok(Some(status)) => Err(format!(
            "warm-up of {daemon} exits with its usage status 64, not {status}; its stderr: {said:?}"
        )),
        Ok(None) => Err(format!(
            "warm-up of {daemon} did not exit within {bound:?}; its stderr: {said:?}"
        )),
        Err(error) => Err(format!(
            "the state of the warm-up of {daemon} is unreadable: {error}; its stderr: {said:?}"
        )),
    }
}

fn first_bytes(file: &mut File) -> String {
    let mut bytes = Vec::new();
    match file
        .rewind()
        .and_then(|()| file.by_ref().take(STDERR_LIMIT).read_to_end(&mut bytes))
    {
        Ok(_) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) => format!("unreadable: {error}"),
    }
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
    assert_eq!(run_once("membraned", shell("exit 64"), BOUND), Ok(()));
}

#[test]
fn the_old_usage_exit_fails_the_warm_up() {
    assert_eq!(
        run_once("membraned", shell("exit 2"), BOUND),
        Err(
            "warm-up of membraned exits with its usage status 64, not exit status: 2; its stderr: \"\""
                .to_string()
        )
    );
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
            "warm-up of membraned exits with its usage status 64, not exit status: 3; its stderr: \"\""
                .to_string()
        )
    );
}

#[test]
fn a_warm_up_killed_by_a_signal_fails_setup() {
    let failure = run_once("membraned", shell("kill -9 $$"), BOUND)
        .expect_err("a death by signal fails the warm-up");

    assert!(
        failure.starts_with("warm-up of membraned exits with its usage status 64, not signal: 9"),
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
#[allow(unsafe_code)]
fn an_unreadable_child_state_is_reported_without_a_panic() {
    let mut child = Command::new("true").spawn().expect("true starts");
    let pid = child.id() as libc::pid_t;
    let mut raw = 0;
    assert_eq!(
        // SAFETY: `raw` is a local the call may write, and `pid` is the child this test just
        // spawned, which nothing has reaped yet, so the pid still names it.
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
