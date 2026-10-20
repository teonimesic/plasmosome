mod common;

use std::io::Write;
#[cfg(target_os = "macos")]
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run_with_stdin(binary: &str, stdin: &str) -> Output {
    run_in(Command::new(binary).env_clear(), stdin)
}

fn run_in(command: &mut Command, stdin: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("the configuration is written");
    child.wait_with_output().expect("the binary exits")
}

#[cfg(target_os = "macos")]
fn vmm_with_bundle(bundle: &Path) -> Output {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let mut lane = common::lane_config(scratch.path(), "run-1");
    lane.bundle = bundle.to_path_buf();
    run_with_stdin(
        env!("CARGO_BIN_EXE_lane-vmm"),
        &lane.to_json().expect("the lane serializes"),
    )
}

#[cfg(target_os = "macos")]
fn assert_refused_naming(output: &Output, code: i32, path: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(code), "stderr: {stderr}");
    assert!(
        stderr.contains(path),
        "stderr should name `{path}`: {stderr}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn lane_vmm_refuses_a_configuration_it_cannot_parse() {
    let output = run_with_stdin(env!("CARGO_BIN_EXE_lane-vmm"), "{\"run_id\": 1}");

    assert_eq!(output.status.code(), Some(64));
}

#[cfg(target_os = "macos")]
#[test]
fn lane_vmm_refuses_a_bundle_that_is_not_the_pinned_one_before_loading() {
    let bundle = common::bundle_fixture();

    let output = vmm_with_bundle(bundle.path());

    assert_refused_naming(&output, 65, "manifest.json");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&bundle.id) && stderr.contains("eed141b574337d99"),
        "the refusal should name both bundles: {stderr}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn lane_vmm_refuses_an_inherited_dyld_variable_before_checking_the_bundle() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let lane = common::lane_config(scratch.path(), "run-1");

    let output = run_in(
        Command::new(env!("CARGO_BIN_EXE_lane-vmm"))
            .env_clear()
            .env("DYLD_LIBRARY_PATH", scratch.path()),
        &lane.to_json().expect("the lane serializes"),
    );

    assert_refused_naming(&output, 69, "DYLD_LIBRARY_PATH");
}

#[cfg(target_os = "macos")]
#[test]
fn lane_vmm_refuses_a_libkrunfw_in_its_working_directory_before_checking_the_bundle() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let cwd = std::fs::canonicalize(scratch.path()).expect("the scratch directory resolves");
    common::write_file(&cwd, "libkrunfw.5.dylib", b"planted");
    let lane = common::lane_config(&cwd, "run-1");

    let output = run_in(
        Command::new(env!("CARGO_BIN_EXE_lane-vmm"))
            .env_clear()
            .current_dir(&cwd),
        &lane.to_json().expect("the lane serializes"),
    );

    assert_refused_naming(
        &output,
        69,
        &cwd.join("libkrunfw.5.dylib").display().to_string(),
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn lane_vmm_refuses_to_run_off_macos() {
    let output = run_with_stdin(env!("CARGO_BIN_EXE_lane-vmm"), "");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "lane-vmm: refused: runs only on macOS"
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
fn lane_init_and_lane_probe_refuse_to_run_off_linux() {
    for binary in [
        env!("CARGO_BIN_EXE_lane-init"),
        env!("CARGO_BIN_EXE_lane-probe"),
    ] {
        let output = run_with_stdin(binary, "");
        assert_eq!(output.status.code(), Some(2), "{binary}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn lane_init_refuses_to_run_as_any_process_but_1() {
    let output = run_with_stdin(env!("CARGO_BIN_EXE_lane-init"), "");

    assert_eq!(output.status.code(), Some(3));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("process 1"),
        "{output:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn lane_probe_prints_one_json_line_for_stat() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = common::write_file(dir.path(), "placed", b"bytes");

    let output = Command::new(env!("CARGO_BIN_EXE_lane-probe"))
        .args(["stat", path.to_str().expect("UTF-8")])
        .output()
        .expect("lane-probe runs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let line: serde_json::Value = serde_json::from_str(stdout.trim()).expect("one JSON line");
    assert_eq!(line["probe"], "stat");
    assert_eq!(line["ok"], true);
}

#[cfg(target_os = "linux")]
#[test]
fn lane_probe_refuses_an_unknown_subcommand_with_a_json_line() {
    let output = Command::new(env!("CARGO_BIN_EXE_lane-probe"))
        .arg("teleport")
        .output()
        .expect("lane-probe runs");

    assert_eq!(output.status.code(), Some(64));
    let line: serde_json::Value = serde_json::from_slice(&output.stdout).expect("one JSON line");
    assert_eq!(line["ok"], false);
}
