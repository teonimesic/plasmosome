mod common;

use std::io::Write;
#[cfg(target_os = "macos")]
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::{Child, ChildStdin};
use std::process::{Command, Output, Stdio};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

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

#[cfg(target_os = "macos")]
#[test]
fn lane_vmm_refuses_a_root_image_inside_the_bundle_before_checking_the_bundle() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let mut lane = common::lane_config(scratch.path(), "run-1");
    lane.root_image = common::write_file(&lane.bundle, "base/rootfs.raw", b"disk");

    let output = run_with_stdin(
        env!("CARGO_BIN_EXE_lane-vmm"),
        &lane.to_json().expect("the lane serializes"),
    );

    assert_refused_naming(&output, 64, "root_image");
}

#[cfg(target_os = "macos")]
const PARENT_GONE_WITHIN: Duration = Duration::from_secs(30);

#[cfg(target_os = "macos")]
const PARENT_POLL: Duration = Duration::from_millis(50);

#[cfg(target_os = "macos")]
fn ps_field(pid: u32, field: &str) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", field, "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
}

#[cfg(target_os = "macos")]
fn running(pid: u32) -> bool {
    ps_field(pid, "stat=").is_some_and(|stat| !stat.starts_with('Z'))
}

#[cfg(target_os = "macos")]
fn poll_until(what: &str, mut done: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while !done() {
        if started.elapsed() > PARENT_GONE_WITHIN {
            eprintln!("{what}: not within {PARENT_GONE_WITHIN:?}");
            return false;
        }
        std::thread::sleep(PARENT_POLL);
    }
    true
}

#[cfg(target_os = "macos")]
fn warmed_vmm() -> &'static str {
    let vmm = env!("CARGO_BIN_EXE_lane-vmm");
    let output = run_with_stdin(vmm, "{}");
    assert_eq!(
        output.status.code(),
        Some(64),
        "the warm-up run should refuse an empty configuration: {output:?}"
    );
    vmm
}

#[cfg(target_os = "macos")]
struct UnderAParent {
    parent: Child,
    _configuration: Option<ChildStdin>,
    vmm: u32,
    stderr: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

#[cfg(target_os = "macos")]
impl UnderAParent {
    fn start(script: &str) -> UnderAParent {
        let vmm = warmed_vmm();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let pid_file = dir.path().join("vmm.pid");
        let stderr = dir.path().join("vmm.stderr");
        let mut parent = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("exec 3<&0\n{script}"))
            .arg(vmm)
            .arg(&pid_file)
            .arg(&stderr)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the parent shell starts");
        let mut pid = None;
        assert!(
            poll_until("the parent shell's pid file", || {
                pid = std::fs::read_to_string(&pid_file)
                    .ok()
                    .filter(|text| text.ends_with('\n'))
                    .and_then(|text| text.trim().parse().ok());
                pid.is_some()
            }),
            "the parent shell never wrote lane-vmm's pid"
        );
        let configuration = parent.stdin.take();
        UnderAParent {
            parent,
            _configuration: configuration,
            vmm: pid.expect("the pid was read"),
            stderr,
            _dir: dir,
        }
    }

    fn stderr(&self) -> String {
        std::fs::read_to_string(&self.stderr).unwrap_or_default()
    }
}

#[cfg(target_os = "macos")]
impl Drop for UnderAParent {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-9", &self.vmm.to_string()])
            .output();
        let _ = self.parent.kill();
        let _ = self.parent.wait();
    }
}

#[cfg(target_os = "macos")]
#[test]
fn lane_vmm_exits_when_the_process_that_started_it_is_killed() {
    let mut run = UnderAParent::start("\"$0\" <&3 3<&- 2>\"$2\" &\necho $! > \"$1\"\nwait");
    let shell = run.parent.id().to_string();
    let vmm = run.vmm;
    assert!(
        poll_until("lane-vmm under the shell", || {
            running(vmm) && ps_field(vmm, "ppid=").as_deref() == Some(shell.as_str())
        }),
        "lane-vmm {vmm} never ran as the shell's child"
    );

    run.parent.kill().expect("the parent shell is killed");
    run.parent.wait().expect("the parent shell is reaped");

    assert!(
        poll_until("lane-vmm gone", || !running(vmm)),
        "lane-vmm {vmm} still runs, as {:?}, after the process that started it was killed; stderr: {}",
        ps_field(vmm, "comm="),
        run.stderr()
    );
    assert!(
        run.stderr().contains("the process that started it is gone"),
        "lane-vmm {vmm} ended, but not by noticing its parent was gone: {}",
        run.stderr()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn lane_vmm_exits_when_it_starts_after_the_process_that_started_it_died() {
    let mut run = UnderAParent::start(
        "outer=$$\n(\n  while kill -0 \"$outer\" 2>/dev/null; do sleep 0.1; done\n  exec \"$0\" <&3 3<&- 2>\"$2\"\n) &\necho $! > \"$1\"\nexit 0",
    );
    let vmm = run.vmm;
    run.parent
        .wait()
        .expect("the parent shell exits and is reaped");

    assert!(
        poll_until("lane-vmm started and gone", || {
            !running(vmm) && !run.stderr().is_empty()
        }),
        "lane-vmm {vmm} did not start and exit after the process that started it died; it runs as {:?}; stderr: {}",
        ps_field(vmm, "comm="),
        run.stderr()
    );
    assert!(
        run.stderr().contains("the process that started it is gone"),
        "lane-vmm {vmm} ended, but not by noticing its parent was gone: {}",
        run.stderr()
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
