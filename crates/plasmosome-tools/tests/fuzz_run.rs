#![deny(clippy::exit)]

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus};
use std::rc::Rc;

use plasmosome_tools::fuzz_run::{TOOLCHAIN, run};

const SECRET: &str = "PRIVATE-CRASH-CONTENTS";
const LOG: &str = "INFO: seed corpus: files: 19 min: 1b max: 8b total: 32b rss: 1Mb\n#20 INITED\nDone 100 runs in 600 second(s)\n";

struct Sink {
    bytes: Rc<RefCell<Vec<u8>>>,
    flushed: Rc<Cell<bool>>,
    fails: bool,
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.fails {
            return Err(io::Error::other(SECRET));
        }
        self.bytes.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushed.set(true);
        Ok(())
    }
}

fn scenario(
    status: i32,
    artifacts: &[&str],
    log: &str,
    hash: &str,
    fails: bool,
) -> (u8, String, String, usize) {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    let args = [
        root.path().as_os_str().to_owned(),
        OsString::from("guest_control_reply"),
        private.as_os_str().to_owned(),
    ];
    let bytes = Rc::new(RefCell::new(Vec::new()));
    let flushed = Rc::new(Cell::new(false));
    let mut sink = Sink {
        bytes: bytes.clone(),
        flushed: flushed.clone(),
        fails,
    };
    let mut calls = 0;
    let mut execute =
        |command: &mut Command| {
            calls += 1;
            assert_eq!(command.get_current_dir(), Some(root.path()));
            if command.get_program() == "cargo" {
                assert!(flushed.get(), "seed must be flushed before spawn");
                assert_eq!(
                    &*bytes.borrow(),
                    b"target=guest_control_reply seed=20261028\n"
                );
                let argv: Vec<_> = command
                    .get_args()
                    .map(|s| s.to_string_lossy().into_owned())
                    .collect();
                assert_eq!(argv[0], TOOLCHAIN);
                for required in [
                    "-seed=20261028",
                    "-max_total_time=600",
                    "-max_len=4096",
                    "-timeout=10",
                    "--debug-assertions",
                ] {
                    assert!(argv.iter().any(|a| a == required), "{argv:?}");
                }
                assert!(argv.iter().any(|a| {
                    a.ends_with("plasmosome-backend/tests/fuzz/corpus/guest_control_reply")
                }));
                assert!(
                    command.get_envs().any(|(k, v)| k == "CARGO_NET_OFFLINE"
                        && v == Some(std::ffi::OsStr::new("true")))
                );
                fs::write(private.join("fuzz.stdout"), SECRET)?;
                fs::write(private.join("fuzz.stderr"), format!("{log}{SECRET}\n"))?;
                for name in artifacts {
                    fs::write(private.join("crashes").join(name), SECRET)?;
                }
                Ok(ExitStatus::from_raw(status))
            } else {
                assert_eq!(command.get_program(), "sha256sum");
                fs::write(private.join("hash.stdout"), hash)?;
                fs::write(private.join("hash.stderr"), SECRET)?;
                Ok(ExitStatus::from_raw(0))
            }
        };
    let mut err = Vec::new();
    let code = run(&args, &mut execute, &mut sink, &mut err);
    let out = String::from_utf8(bytes.borrow().clone()).unwrap();
    let err = String::from_utf8(err).unwrap();
    assert!(!out.contains(SECRET), "{out}");
    assert!(!err.contains(SECRET), "{err}");
    (code, out, err, calls)
}

fn valid_hash() -> String {
    format!("{}  private-file\n", "a".repeat(64))
}

#[test]
fn a_completed_run_prints_only_numeric_summary() {
    let (code, out, err, calls) = scenario(0, &[], LOG, &valid_hash(), false);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "target=guest_control_reply seed=20261028\ncorpus_files=19 runs=100 seconds=600\n"
    );
    assert!(err.is_empty());
    assert_eq!(calls, 1);
}

#[test]
fn crashes_signals_timeouts_and_multiple_inputs_keep_both_streams_private() {
    for (status, inputs) in [
        (256, vec!["crash-one"]),
        (9, vec!["timeout-one", "oom-two", "crash-three"]),
    ] {
        let (code, out, err, calls) = scenario(status, &inputs, LOG, &valid_hash(), false);
        assert_eq!(code, 1);
        assert!(err.is_empty());
        assert_eq!(out.lines().count(), inputs.len() + 1);
        assert_eq!(calls, inputs.len() + 2);
        for line in out.lines().skip(1) {
            assert_eq!(
                line,
                format!(
                    "target=guest_control_reply seed=20261028 input_sha256={} length={} binary_sha256={}",
                    "a".repeat(64),
                    SECRET.len(),
                    "a".repeat(64)
                )
            );
        }
    }
}

#[test]
fn missing_or_invalid_artifacts_are_fixed_failures() {
    for (status, inputs, reason) in [
        (256, vec![], "failed child left no crash input"),
        (256, vec!["unexpected"], "invalid crash entry"),
        (0, vec!["crash-one"], "successful child left crash inputs"),
    ] {
        let (code, _, err, _) = scenario(status, &inputs, LOG, &valid_hash(), false);
        assert_eq!(code, 1);
        assert_eq!(err, format!("fuzz-run: {reason}\n"));
    }
}

#[test]
fn malformed_hashes_are_never_public() {
    for hash in [
        SECRET.to_owned(),
        format!("{}  file\n", "A".repeat(64)),
        format!("{}  file\n", "a".repeat(63)),
        format!("{}  \n", "a".repeat(64)),
    ] {
        let (code, out, err, _) = scenario(256, &["crash-one"], LOG, &hash, false);
        assert_eq!(code, 1);
        assert!(!out.contains("input_sha256="));
        assert_eq!(err, "fuzz-run: invalid hash\n");
    }
}

#[test]
fn incomplete_or_invalid_success_summaries_fail() {
    for log in [
        LOG.replace("600", "599"),
        LOG.replace("INITED", "missing"),
        LOG.replace("19 min", "0 min"),
        LOG.replace("100 runs", "-1 runs"),
        String::from("not a summary\n"),
    ] {
        let (code, _, err, _) = scenario(0, &[], &log, &valid_hash(), false);
        assert_eq!(code, 1);
        assert_eq!(err, "fuzz-run: missing completed fuzz summary\n");
    }
}

#[test]
fn an_output_failure_prevents_the_spawn() {
    let (code, _, err, calls) = scenario(0, &[], LOG, &valid_hash(), true);
    assert_eq!(code, 1);
    assert_eq!(calls, 0);
    assert_eq!(err, "fuzz-run: cannot write seed\n");
}

#[test]
fn invalid_arguments_and_existing_private_directory_never_spawn() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec![],
        vec![root.path().into(), "unknown".into(), root.path().into()],
        vec!["relative".into(), "manifest".into(), root.path().into()],
        vec![root.path().into(), "manifest".into(), root.path().into()],
    ] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args, &mut |_| panic!("must not spawn"), &mut out, &mut err);
        assert_ne!(code, 0);
        assert!(out.is_empty());
    }
}

#[test]
fn real_child_streams_are_captured_in_private_files() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    let fake = root.path().join("cargo");
    fs::write(&fake, format!("#!/bin/sh\n[ \"$1\" = warm ] && exit 0\nprintf '{SECRET}'\nprintf '{SECRET}' >&2\nexit 3\n")).unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(Command::new(&fake).arg("warm").status().unwrap().success());
    let mut execute = |cmd: &mut Command| {
        cmd.env("PATH", root.path());
        cmd.status()
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run(
        &[
            root.path().into(),
            "manifest".into(),
            private.clone().into(),
        ],
        &mut execute,
        &mut out,
        &mut err,
    );
    assert_eq!(code, 1);
    assert_eq!(
        fs::read_to_string(private.join("fuzz.stdout")).unwrap(),
        SECRET
    );
    assert_eq!(
        fs::read_to_string(private.join("fuzz.stderr")).unwrap(),
        SECRET
    );
    assert!(!String::from_utf8(out).unwrap().contains(SECRET));
    assert!(!String::from_utf8(err).unwrap().contains(SECRET));
    assert_eq!(fs::metadata(&private).unwrap().mode() & 0o777, 0o700);
    assert_eq!(
        fs::metadata(private.join("fuzz.stdout")).unwrap().mode() & 0o777,
        0o600
    );
}
