use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use plasmosome_backend::{BootSession, CellId};
use plasmosome_core::{DiscoveryError, InstanceRoot};

const ROOT_VARIABLE: &str = "PLASMOSOME_INSTANCE_CHILD_ROOT";
const HEADROOM_VARIABLE: &str = "PLASMOSOME_INSTANCE_CHILD_HEADROOM";
const UMASK_VARIABLE: &str = "PLASMOSOME_INSTANCE_CHILD_UMASK";
const REPORT: &str = "plasmosome-test-out-of-descriptors-at ";
const DEADLINE: Duration = Duration::from_secs(5);

struct Finished {
    succeeded: bool,
    stdout: String,
    stderr: String,
}

fn temp_root() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = dir
        .path()
        .canonicalize()
        .expect("a canonical temporary path");
    (dir, root)
}

fn drain(mut pipe: impl Read + Send + 'static) -> JoinHandle<String> {
    thread::spawn(move || {
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        text
    })
}

fn run_child(test: &str, root: &Path, setting: (&str, String)) -> Finished {
    let mut command = Command::new(std::env::current_exe().expect("the test binary's path"));
    command
        .args(["--exact", test, "--ignored", "--nocapture"])
        .env(ROOT_VARIABLE, root)
        .env(setting.0, setting.1)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("the child starts");
    let stdout = drain(child.stdout.take().expect("stdout is piped"));
    let stderr = drain(child.stderr.take().expect("stderr is piped"));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("the child's status is readable") {
            break status;
        }
        if started.elapsed() >= DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{test} did not finish within {DEADLINE:?}");
        }
        thread::sleep(Duration::from_millis(10));
    };
    Finished {
        succeeded: status.success(),
        stdout: stdout.join().expect("the stdout reader finishes"),
        stderr: stderr.join().expect("the stderr reader finishes"),
    }
}

fn child_root() -> Option<PathBuf> {
    std::env::var_os(ROOT_VARIABLE).map(PathBuf::from)
}

#[allow(unsafe_code)]
fn lower_descriptor_limit(headroom: libc::rlim_t) {
    // SAFETY: dup takes no pointer.
    let next = unsafe { libc::dup(libc::STDERR_FILENO) };
    assert!(next >= 0, "dup: {}", io::Error::last_os_error());
    // SAFETY: `next` is the descriptor dup just returned, which nothing else owns, so closing it
    // closes no other owner's descriptor.
    unsafe { libc::close(next) };
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        // SAFETY: `limit` is a local libc::rlimit that getrlimit may write.
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
        0,
        "getrlimit: {}",
        io::Error::last_os_error()
    );
    limit.rlim_cur = next as libc::rlim_t + headroom;
    assert_eq!(
        // SAFETY: `limit` is a local libc::rlimit that setrlimit only reads.
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) },
        0,
        "setrlimit: {}",
        io::Error::last_os_error()
    );
}

#[test]
#[ignore = "helper process for the descriptor-limit test"]
fn discovery_under_a_descriptor_limit_child() {
    let Some(root) = child_root() else {
        return;
    };
    let headroom = std::env::var(HEADROOM_VARIABLE)
        .expect("the headroom is set")
        .parse()
        .expect("the headroom is a number");
    let instance = InstanceRoot::open(&root).expect("the instance root opens");
    let lock = instance.lock().expect("the child takes the writer lock");
    lower_descriptor_limit(headroom);
    match instance.discover(&lock) {
        Err(DiscoveryError::OutOfDescriptors { path, source }) => {
            assert_eq!(source.raw_os_error(), Some(libc::EMFILE), "{source}");
            println!("{REPORT}{}", path.display());
        }
        other => panic!("expected OutOfDescriptors, got {other:?}"),
    }
}

#[test]
fn running_out_of_descriptors_aborts_discovery_instead_of_refusing_cells() {
    let (_dir, root) = temp_root();
    let cells = root.join("cells");
    for index in 0..24 {
        let cell = cells.join(format!("cell-{index:02}"));
        fs::create_dir_all(&cell).expect("the cell directory is made");
        fs::write(cell.join("ledger.ndjson"), b"{}\n").expect("the journal is written");
    }
    for (headroom, exact) in [(0, true), (1, true), (16, false)] {
        let finished = run_child(
            "discovery_under_a_descriptor_limit_child",
            &root,
            (HEADROOM_VARIABLE, headroom.to_string()),
        );
        assert!(
            finished.succeeded,
            "headroom {headroom}: the child failed\nstdout:\n{}\nstderr:\n{}",
            finished.stdout, finished.stderr
        );
        let reported = finished
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix(REPORT))
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                panic!(
                    "headroom {headroom}: no report\nstdout:\n{}",
                    finished.stdout
                )
            });
        if exact {
            assert_eq!(reported, cells, "headroom {headroom}");
        } else {
            assert_eq!(
                reported.parent(),
                Some(cells.as_path()),
                "headroom {headroom}"
            );
        }
    }
}

#[test]
#[ignore = "helper process for the umask test"]
#[allow(unsafe_code)]
fn instance_under_a_restrictive_umask_child() {
    let Some(root) = child_root() else {
        return;
    };
    let umask = std::env::var(UMASK_VARIABLE).expect("the umask is set");
    let umask = libc::mode_t::from_str_radix(&umask, 8).expect("the umask is octal");
    // SAFETY: umask takes no pointer and cannot fail.
    unsafe { libc::umask(umask) };
    let instance = InstanceRoot::open(&root).expect("the instance root opens");
    let lock = instance.lock().expect("the child takes the writer lock");
    let created = instance
        .create_cell_dir(&lock, &CellId::from("cell-1"))
        .expect("the cell directory is created");
    let journal = created
        .open_journal_for_append(&lock)
        .expect("the journal is created");
    assert!(journal.created);
    let boot = BootSession::parse("5d1e7c3a-0b2f-4e6d-9a8b-1c2d3e4f5a6b").expect("a boot session");
    created.create_launched(boot).expect("launched is created");
    created.create_created(&lock).expect("created is written");
    drop(created.create_runtime_dir().expect("runtime is created"));
    let ended = instance
        .create_cell_dir(&lock, &CellId::from("cell-2"))
        .expect("the second cell directory is created");
    ended
        .create_launched(boot)
        .expect("its launched is created");
    ended
        .mark_launched_stopped()
        .expect("its clean stop is recorded");
    instance
        .retire(&lock, ended)
        .expect("the second cell retires");
}

#[test]
fn modes_are_private_whatever_the_umask() {
    let names = [
        "controller.lock",
        "cells",
        "cells/cell-1",
        "cells/cell-1/ledger.ndjson",
        "cells/cell-1/launched",
        "cells/cell-1/created",
        "cells/cell-1/runtime",
        "retired",
        "retired/cell-2",
        "retired/cell-2/launched",
    ];
    for umask in ["377", "477", "777"] {
        let (_dir, root) = temp_root();
        let finished = run_child(
            "instance_under_a_restrictive_umask_child",
            &root,
            (UMASK_VARIABLE, umask.to_owned()),
        );
        let modes = names.map(|name| {
            fs::symlink_metadata(root.join(name))
                .map(|metadata| metadata.mode() & 0o7777)
                .ok()
        });
        for name in [
            "cells",
            "cells/cell-1",
            "cells/cell-1/runtime",
            "retired",
            "retired/cell-2",
        ] {
            let _ = fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o700));
        }
        assert!(
            finished.succeeded,
            "umask {umask}: the child failed\nstdout:\n{}\nstderr:\n{}",
            finished.stdout, finished.stderr
        );
        assert_eq!(
            modes,
            [
                Some(0o600),
                Some(0o700),
                Some(0o700),
                Some(0o600),
                Some(0o600),
                Some(0o600),
                Some(0o700),
                Some(0o700),
                Some(0o700),
                Some(0o600),
            ],
            "umask {umask}: {names:?}"
        );
    }
}
