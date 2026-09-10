use serde_json::json;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PATIENCE: Duration = Duration::from_secs(5);

fn membraned(directory: &Path, arguments: &[&OsStr]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_membraned"));
    command
        .current_dir(directory)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn output_within(mut command: Command) -> Output {
    let mut child = command.spawn().expect("membraned starts as a process");
    let deadline = Instant::now() + PATIENCE;
    while child
        .try_wait()
        .expect("membraned's state is readable")
        .is_none()
    {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("membraned exits within {PATIENCE:?}");
        }
        thread::sleep(Duration::from_millis(25));
    }
    child
        .wait_with_output()
        .expect("membraned's output is readable")
}

fn write_config(directory: &Path, filename: &OsStr, occupied: &Path) {
    let body = json!({
        "control_socket": occupied,
        "status_deadline_ms": 500,
        "brokers": [],
    });
    std::fs::write(
        directory.join(filename),
        serde_json::to_vec(&body).expect("the config is JSON"),
    )
    .expect("the config is written");
}

#[test]
fn exact_help_operands_succeed_before_config_reading() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");
    for argument in [OsStr::new("-h"), OsStr::new("--help")] {
        std::fs::write(directory.path().join(argument), b"not JSON")
            .expect("the conflicting filename is written");
        let output = output_within(membraned(directory.path(), &[argument]));
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
        assert!(stdout.contains("membraned <config.json>"), "{stdout}");
        assert!(stdout.contains("membrane.status"), "{stdout}");
    }
}

#[test]
fn every_other_sole_operand_remains_a_literal_config_path() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");
    let occupied = directory.path().join("taken.uds");
    std::fs::write(&occupied, b"owned occupied path").expect("the occupied path is written");

    let cases: [(OsString, OsString); 5] = [
        (OsString::from("--help"), OsString::from("./--help")),
        (OsString::from("-h"), OsString::from("./-h")),
        (OsString::from("--version"), OsString::from("--version")),
        (OsString::from("help"), OsString::from("help")),
        (OsString::from("--"), OsString::from("--")),
    ];

    for (filename, argument) in cases {
        write_config(directory.path(), &filename, &occupied);
        let output = output_within(membraned(directory.path(), &[&argument]));
        assert_eq!(
            output.status.code(),
            Some(1),
            "literal operand {argument:?} reaches daemon startup: {output:?}"
        );
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("taken.uds"),
            "{output:?}"
        );
    }

    let non_utf8 = OsString::from_vec(vec![b'c', b'f', b'g', 0xff]);
    let output = output_within(membraned(directory.path(), &[&non_utf8]));
    assert_eq!(
        output.status.code(),
        Some(2),
        "a non-UTF-8 operand reaches config reading: {output:?}"
    );
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot read"),
        "{output:?}"
    );
}

#[test]
fn help_does_not_override_the_existing_arity_check() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");
    let config = directory.path().join("valid.json");
    let socket = directory.path().join("membrane.uds");
    write_config(directory.path(), OsStr::new("valid.json"), &socket);

    for arguments in [
        [OsStr::new("--help"), config.as_os_str()],
        [config.as_os_str(), OsStr::new("--help")],
    ] {
        let output = output_within(membraned(directory.path(), &arguments));
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("usage:"),
            "{output:?}"
        );
    }
    assert!(!socket.exists(), "invalid arity never starts the daemon");
}
