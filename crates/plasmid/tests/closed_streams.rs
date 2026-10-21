use std::io::{ErrorKind, PipeWriter, Write};
use std::path::Path;
use std::process::{Command, Output};

#[allow(
    dead_code,
    reason = "these cases read envelopes with the shared helpers but refuse no usage"
)]
mod support;

use support::{entries, envelope, keys, text};

fn closed_pipe() -> PipeWriter {
    let (reader, mut writer) = std::io::pipe().expect("a pipe");
    drop(reader);
    let probe = writer.write(b"x");
    assert!(
        probe
            .as_ref()
            .is_err_and(|error| error.kind() == ErrorKind::BrokenPipe),
        "the pipe's read end is still open in some process: {probe:?}"
    );
    writer
}

fn plasmid_in(directory: &Path, arguments: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_plasmid"));
    command
        .env_remove("PLASMOSOME_KERNEL")
        .current_dir(directory)
        .args(arguments);
    command
}

fn run(mut command: Command, directory: &Path, arguments: &[&str]) -> Output {
    let output = command.output().expect("the plasmid binary runs");
    assert_eq!(
        entries(directory),
        Vec::<String>::new(),
        "plasmid wrote into its working directory for {arguments:?}"
    );
    output
}

fn closed_stdout(arguments: &[&str]) -> Output {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let mut command = plasmid_in(scratch.path(), arguments);
    command.stdout(closed_pipe());
    run(command, scratch.path(), arguments)
}

fn help_and_version_to_a_closed_stdout_exit_1() {
    for (arguments, written) in [
        (&["--help"][..], "--help"),
        (&["-h"], "--help"),
        (&["add", "--help"], "--help"),
        (&["new", "--help"], "--help"),
        (&["--version"], "--version"),
    ] {
        let output = closed_stdout(arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}: {output:?}");
        let (command, error) = envelope(&output);
        assert_eq!(command, written, "{arguments:?}");
        assert_eq!(error["code"], "local_io", "{arguments:?}");
        assert_eq!(error["errno"], 32, "{arguments:?}");
        assert_eq!(
            keys(&error),
            ["code", "domain", "errno", "message"],
            "{arguments:?}"
        );
    }
    let output = closed_stdout(&["--help", "--human"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = text(&output.stderr);
    let lines: Vec<&str> = stderr.split('\n').collect();
    assert_eq!(lines.len(), 3, "{stderr:?}");
    assert!(lines[0].contains("stdout"), "{stderr:?}");
    assert_eq!(lines[1..], ["(client local_io)", ""], "{stderr:?}");
}

fn an_envelope_to_a_closed_stderr_still_exits_with_its_code() {
    for (arguments, code) in [(&["frobnicate"][..], 64), (&["add", "x"], 2)] {
        let scratch = tempfile::tempdir().expect("a scratch directory");
        let mut command = plasmid_in(scratch.path(), arguments);
        command.stderr(closed_pipe());
        let output = run(command, scratch.path(), arguments);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{arguments:?}: {output:?}"
        );
        assert!(output.stdout.is_empty(), "{arguments:?}: {output:?}");
    }
}

fn missing_sections_to_a_closed_stdout_exit_3() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let mut command = plasmid_in(scratch.path(), &["new", "demo"]);
    command.stdout(closed_pipe());
    let output = command.output().expect("the plasmid binary runs");
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert_eq!(entries(scratch.path()), ["demo.toml"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("demo.toml"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn writes_to_a_closed_stream_one_case_at_a_time() {
    help_and_version_to_a_closed_stdout_exit_1();
    an_envelope_to_a_closed_stderr_still_exits_with_its_code();
    missing_sections_to_a_closed_stdout_exit_3();
}
