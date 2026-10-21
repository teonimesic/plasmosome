use serde_json::json;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[path = "support/private_root.rs"]
mod private_root;
#[path = "support/warm_up.rs"]
mod warm_up;

const PATIENCE: Duration = Duration::from_secs(5);

fn plasmosomed(directory: &Path, arguments: &[&OsStr]) -> Command {
    let mut command = warm_up::plasmosomed();
    command
        .current_dir(directory)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn output_within(mut command: Command) -> Output {
    let mut child = command.spawn().expect("plasmosomed starts as a process");
    let deadline = Instant::now() + PATIENCE;
    while match child.try_wait() {
        Ok(Some(_)) => false,
        Ok(None) => true,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("plasmosomed's state is readable: {error}");
        }
    } {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("plasmosomed exits within {PATIENCE:?}");
        }
        thread::sleep(Duration::from_millis(25));
    }
    child
        .wait_with_output()
        .expect("plasmosomed's output is readable")
}

fn write_config(directory: &Path, filename: &OsStr, occupied: &Path) {
    let body = json!({
        "control_socket": occupied,
        "name": "cli-test",
        "instance_root": directory.join("instance"),
        "recovery_deadline_ms": 5000,
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
        let output = output_within(plasmosomed(directory.path(), &[argument]));
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
        assert!(stdout.contains("plasmosomed <config.json>"), "{stdout}");
        assert!(stdout.contains("--version"), "{stdout}");
        assert!(stdout.contains("plasmosome.status"), "{stdout}");
    }
}

#[test]
fn help_shows_a_config_the_daemon_accepts_and_says_it_recovers_no_cells() {
    let directory = tempfile::tempdir().expect("the test owns a temporary directory");
    let output = output_within(plasmosomed(directory.path(), &[OsStr::new("--help")]));
    assert!(output.status.success(), "{output:?}");
    let help = String::from_utf8(output.stdout).expect("help is UTF-8");

    let example = help
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with('{'))
        .unwrap_or_else(|| panic!("help shows an example config: {help}"));
    if let Err(error) = plasmosome_core::parse_config(example) {
        panic!("the config help shows is one plasmosomed accepts, got {error}: {example}");
    }
    let prose = help.split_whitespace().collect::<Vec<_>>().join(" ");
    for claim in [
        "instance_root, recovery_deadline_ms and an optional absolute registry_root are checked \
         for shape only",
        "this build does not open the roots, read registry_root or recover cells yet",
    ] {
        assert!(
            prose.contains(claim),
            "help claims no more than this build does, and says {claim:?}: {help}"
        );
    }
}

fn assert_prints_the_version(directory: &Path, state: &str) {
    let output = output_within(plasmosomed(directory, &[OsStr::new("--version")]));
    assert_eq!(output.status.code(), Some(0), "{state}: {output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("plasmosomed {}\n", env!("CARGO_PKG_VERSION")),
        "{state}: {output:?}"
    );
    assert!(output.stderr.is_empty(), "{state}: {output:?}");
}

#[test]
fn version_is_the_sole_argument_and_reads_no_config() {
    let directory = private_root::private_root();
    let socket = directory.path().join("free.uds");

    assert_prints_the_version(directory.path(), "no file named --version");

    std::fs::write(directory.path().join("--version"), b"not JSON")
        .expect("the conflicting filename is written");
    assert_prints_the_version(directory.path(), "--version holds not JSON");

    write_config(directory.path(), OsStr::new("--version"), &socket);
    assert_prints_the_version(directory.path(), "--version holds a valid config");
    assert!(!socket.exists(), "--version binds nothing");
}

#[test]
fn every_other_sole_operand_remains_a_literal_config_path() {
    let directory = private_root::private_root();
    let occupied = directory.path().join("taken.uds");
    std::fs::write(&occupied, b"owned occupied path").expect("the occupied path is written");

    let cases: [(OsString, OsString); 9] = [
        (OsString::from("--help"), OsString::from("./--help")),
        (OsString::from("-h"), OsString::from("./-h")),
        (OsString::from("--version"), OsString::from("./--version")),
        (OsString::from("-V"), OsString::from("-V")),
        (OsString::from("--Version"), OsString::from("--Version")),
        (OsString::from("--version=x"), OsString::from("--version=x")),
        (OsString::from("--help=x"), OsString::from("--help=x")),
        (OsString::from("help"), OsString::from("help")),
        (OsString::from("--"), OsString::from("--")),
    ];

    for (filename, argument) in cases {
        write_config(directory.path(), &filename, &occupied);
        let output = output_within(plasmosomed(directory.path(), &[&argument]));
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

    write_config(directory.path(), OsStr::new("cfg\u{fffd}"), &occupied);
    let non_utf8 = OsString::from_vec(vec![b'c', b'f', b'g', 0xff]);
    let output = output_within(plasmosomed(directory.path(), &[&non_utf8]));
    assert_eq!(
        output.status.code(),
        Some(2),
        "a raw non-UTF-8 operand does not alias its lossy spelling: {output:?}"
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
    let socket = directory.path().join("control.uds");
    write_config(directory.path(), OsStr::new("valid.json"), &socket);

    for arguments in [
        [OsStr::new("--help"), config.as_os_str()],
        [config.as_os_str(), OsStr::new("--help")],
    ] {
        let output = output_within(plasmosomed(directory.path(), &arguments));
        assert_eq!(output.status.code(), Some(64), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("usage:"),
            "{output:?}"
        );
    }
    assert!(!socket.exists(), "invalid arity never starts the daemon");
}

#[test]
fn a_usage_error_exits_64_and_starts_nothing() {
    let directory = private_root::private_root();
    let socket = directory.path().join("control.uds");
    write_config(directory.path(), OsStr::new("valid.json"), &socket);

    let lists: [&[&str]; 8] = [
        &[],
        &["--version", "extra"],
        &["extra", "--version"],
        &["-h", "extra"],
        &["--help", "extra"],
        &["--version", "--help"],
        &["valid.json", "extra"],
        &["--config-fd=3", "--lock-fd=4", "--cell=cell-1"],
    ];
    for list in lists {
        let arguments: Vec<&OsStr> = list.iter().map(OsStr::new).collect();
        let output = output_within(plasmosomed(directory.path(), &arguments));
        assert_eq!(output.status.code(), Some(64), "{list:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{list:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("usage: plasmosomed <config.json>"),
            "{list:?}: {output:?}"
        );
        assert!(!socket.exists(), "{list:?} never starts the daemon");
    }
}
