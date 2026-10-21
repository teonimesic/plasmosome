use plasmosome_backend::CellId;
use plasmosome_core::runtime::path::RecipePath;
use plasmosome_core::runtime::recipe::{Architecture, PlatformLimits, RuntimeRecipe};
use plasmosome_core::runtime::supervisor_config::SupervisorConfig;
use plasmosome_core::supervisor_start::{StartError, StartStep, SupervisorStart, start_supervisor};
use plasmosome_core::{CellDir, FileIdentity, InstanceRoot, LockProbe, SupervisorLock, WriterLock};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CASE: &str = "PLASMOSOME_SUPERVISOR_START_CASE";
const CONTROLLER: &str = "PLASMOSOME_SUPERVISOR_START_CONTROLLER";
const DEADLINE: &str = "PLASMOSOME_SUPERVISOR_START_DEADLINE";
const SCRATCH: &str = "PLASMOSOME_SUPERVISOR_START_SCRATCH";
const PIDS: &str = "pids";
const CASE_DEADLINE: Duration = Duration::from_secs(20);
const DEADLINE_MARGIN: Duration = Duration::from_secs(2);
const STANDIN_PATIENCE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(5);
const STARTED: &str = "the child controller started its supervisor";
const ONE_MIB: usize = 1 << 20;
const LAST_SIGNAL: libc::c_int = 64;

const CASES: [(&str, fn()); 16] = [
    (
        "starts_detached_with_only_0_to_4",
        starts_detached_with_only_0_to_4,
    ),
    ("lock_never_free_until_exit", lock_never_free_until_exit),
    (
        "survives_the_controller_group",
        survives_the_controller_group,
    ),
    (
        "exec_failure_returns_the_lock",
        exec_failure_returns_the_lock,
    ),
    (
        "a_start_returns_long_before_its_deadline",
        a_start_returns_long_before_its_deadline,
    ),
    ("early_exit_is_closed_pipe", early_exit_is_closed_pipe),
    ("unread_record_times_out", unread_record_times_out),
    ("lock_already_at_four", lock_already_at_four),
    (
        "sources_sit_on_each_others_targets",
        sources_sit_on_each_others_targets,
    ),
    (
        "a_cell_holding_a_nul_is_refused_before_any_fork",
        a_cell_holding_a_nul_is_refused_before_any_fork,
    ),
    (
        "no_handler_of_the_controller_runs_in_a_child",
        no_handler_of_the_controller_runs_in_a_child,
    ),
    (
        "closes_from_5_to_the_per_process_top",
        closes_from_5_to_the_per_process_top,
    ),
    (
        "the_callers_signal_mask_is_restored",
        the_callers_signal_mask_is_restored,
    ),
    (
        "an_unconfirmed_exec_leaves_membraned_an_empty_record",
        an_unconfirmed_exec_leaves_membraned_an_empty_record,
    ),
    (
        "test_threads_is_a_usage_error",
        test_threads_is_a_usage_error,
    ),
    (
        "skip_is_a_usage_error_not_a_filter",
        skip_is_a_usage_error_not_a_filter,
    ),
];

const NO_OP_FLAGS: [&str; 5] = [
    "--include-ignored",
    "--nocapture",
    "--show-output",
    "--quiet",
    "-q",
];

fn main() -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().collect();
    if let Some(cell) = standin_cell(&arguments) {
        standin::run(&arguments, cell)
    }
    if given_environment().is_empty() {
        eprintln!(
            "supervisor_start: an empty environment without spec 023's argv is a broken start, not a test run"
        );
        standin::exit(2)
    }
    if let Some(root) = std::env::var_os(CONTROLLER) {
        child_controller(Path::new(&root))
    }
    if let Some(name) = std::env::var_os(CASE) {
        let (_, case) = CASES
            .iter()
            .find(|(case, _)| name == *case)
            .unwrap_or_else(|| panic!("{name:?} names a case"));
        case();
        return ExitCode::SUCCESS;
    }
    run_every_case(&arguments[1..])
}

fn run_every_case(arguments: &[OsString]) -> ExitCode {
    let mut exact = false;
    let mut only_ignored = false;
    let mut filters = Vec::new();
    for argument in arguments {
        match argument.to_string_lossy().as_ref() {
            "--exact" => exact = true,
            "--ignored" => only_ignored = true,
            flag if NO_OP_FLAGS.contains(&flag) => {}
            flag if flag.starts_with('-') => {
                eprintln!(
                    "supervisor_start: {flag} is not a flag this runner takes. It takes case-name filters, --exact, --ignored, {}",
                    NO_OP_FLAGS.join(", ")
                );
                return ExitCode::from(2);
            }
            filter => filters.push(filter.to_owned()),
        }
    }
    let chosen: Vec<&str> = CASES
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| {
            !only_ignored
                && (filters.is_empty()
                    || filters.iter().any(|filter| {
                        if exact {
                            name == filter
                        } else {
                            name.contains(filter.as_str())
                        }
                    }))
        })
        .collect();
    let filtered_out = CASES.len() - chosen.len();
    let all_started = Instant::now();
    println!("\nrunning {} tests", chosen.len());
    let mut passed = 0;
    let mut failed = Vec::new();
    for name in chosen {
        let started = Instant::now();
        match run_case(name) {
            Ok(()) => {
                println!("test {name} ... ok");
                println!("    took {:?}", started.elapsed());
                passed += 1;
            }
            Err(failure) => {
                println!("test {name} ... FAILED");
                println!("    {failure}");
                failed.push(name);
            }
        }
    }
    if !failed.is_empty() {
        println!("\nfailures:\n    {}", failed.join("\n    "));
    }
    println!(
        "\ntest result: {}. {passed} passed; {} failed; 0 ignored; 0 measured; {filtered_out} filtered out; finished in {:.2}s\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        failed.len(),
        all_started.elapsed().as_secs_f64()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run_case(name: &str) -> Result<(), String> {
    let scratch = tempfile::Builder::new()
        .prefix("supervisor-start-")
        .tempdir()
        .map_err(|error| format!("the case's scratch directory is not made: {error}"))?;
    let pids = scratch.path().join(PIDS);
    std::fs::create_dir(&pids)
        .map_err(|error| format!("the case's pid directory is not made: {error}"))?;
    let mut child = Command::new(this_executable())
        .env(CASE, name)
        .env(
            DEADLINE,
            (epoch_millis() + CASE_DEADLINE.as_millis()).to_string(),
        )
        .env(SCRATCH, scratch.path())
        .process_group(0)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|error| format!("the case process does not start: {error}"))?;
    let outcome = wait_for_case(&mut child);
    let left = stop_what_the_case_left(&pids);
    drop(scratch);
    match (outcome, left.is_empty()) {
        (Ok(()), true) => Ok(()),
        (Ok(()), false) => Err(format!("the case passed but left {left:?} running")),
        (Err(failure), true) => Err(failure),
        (Err(failure), false) => Err(format!("{failure}, and it left {left:?} running")),
    }
}

fn wait_for_case(child: &mut Child) -> Result<(), String> {
    let deadline = Instant::now() + CASE_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("the case process ended with {status}")),
            Ok(None) if Instant::now() >= deadline => {
                let group = libc::pid_t::try_from(child.id()).expect("a pid fits pid_t");
                kill_group(group, libc::SIGKILL);
                let _ = child.wait();
                return Err(format!(
                    "the case did not finish within {CASE_DEADLINE:?}, so its process group was killed"
                ));
            }
            Ok(None) => thread::sleep(POLL),
            Err(error) => return Err(format!("the case process cannot be waited for: {error}")),
        }
    }
}

fn stop_what_the_case_left(pids: &Path) -> Vec<String> {
    let mut left = Vec::new();
    let Ok(entries) = std::fs::read_dir(pids) else {
        return left;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((role, pid)) = name
            .split_once('-')
            .and_then(|(role, pid)| Some((role.to_owned(), pid.parse::<libc::pid_t>().ok()?)))
        else {
            continue;
        };
        let ours = match role.as_str() {
            "standin" => is_our_standin(pid),
            "controller" => is_our_controller(pid),
            _ => false,
        };
        if ours {
            kill_and_wait(pid);
            left.push(format!("{role} {pid}"));
        }
    }
    left
}

fn starts_detached_with_only_0_to_4() {
    assert!(
        ignores(libc::SIGPIPE),
        "the test process ignores SIGPIPE, which an exec keeps unless the start resets it"
    );
    let _blocked = SignalBlocked::on_this_thread(libc::SIGUSR2);
    let root = Root::new();
    let cell = Cell::create(&root.path, "cell-101");
    let lock = cell.lock();
    let log = cell.log();
    let record = cell.record();
    let planted = Planted::low_and_last();
    started(cell.start(&root.membraned(), lock, log, record.clone()));
    drop(planted);
    let report = Report::read(&cell.path);
    assert_eq!(
        report.strings("argv"),
        [
            root.membraned().as_str(),
            "--config-fd=3",
            "--lock-fd=4",
            "--cell=cell-101"
        ],
        "{report}"
    );
    assert_eq!(
        report.strings("environment"),
        Vec::<String>::new(),
        "{report}"
    );
    assert_descriptors(&report, &cell);
    assert_eq!(report.text("record"), String::from_utf8(record).unwrap());
    assert_eq!(report.text("decoded"), "cell-101", "{report}");
    assert_eq!(report.numbers("open_above_four"), [0u64; 0], "{report}");
    assert!(report.flag("mask_empty"), "{report}");
    assert_eq!(report.numbers("ignored"), [0u64; 0], "{report}");
    let (pid, sid, pgid) = (
        report.number("pid"),
        report.number("sid"),
        report.number("pgid"),
    );
    assert_eq!((sid, pgid), (pid, pid), "{report}");
    assert_ne!(sid, u64::from(own_sid()), "{report}");
    assert_ne!(pgid, u64::from(own_pgid()), "{report}");
    assert_reparented(&report);
}

fn closes_from_5_to_the_per_process_top() {
    let low = Planted::at(&[5, 6]);
    let root = Root::new();
    let cell = Cell::create(&root.path, "cell-109");
    let lock = cell.lock();
    let log = cell.log();
    let record = cell.record();
    let top = descriptor_top();
    let limit = SoftLimit::raised_past(top);
    let mut targets = vec![top];
    if top > 200_000 {
        targets.insert(0, 200_000);
    }
    let high = Planted::at(&targets);
    limit.lower_to(256);
    started(cell.start(&root.membraned(), lock, log, record));
    drop(limit);
    drop(high);
    drop(low);
    let report = Report::read(&cell.path);
    assert_eq!(
        report.numbers("open_above_four"),
        [0u64; 0],
        "descriptors 5, 6 and {targets:?} are closed, with the soft limit at 256; {report}"
    );
}

fn the_callers_signal_mask_is_restored() {
    let _blocked = SignalBlocked::on_this_thread(libc::SIGUSR2);
    let before = blocked_signals();
    assert_eq!(before, [libc::SIGUSR2], "the case blocks SIGUSR2 alone");
    let root = Root::new();
    let cell = Cell::create(&root.path, "cell-110");
    let missing =
        RecipePath::parse(&format!("{}/missing", root.path.display())).expect("a canonical path");
    match cell.start(&missing, cell.lock(), cell.log(), cell.record()) {
        Err(StartError::Exec { lock, .. }) => drop(lock),
        other => panic!("a missing membraned is an Exec failure, not {other:?}"),
    }
    assert_eq!(
        blocked_signals(),
        before,
        "the start gives the calling thread back the mask it had"
    );
}

fn an_unconfirmed_exec_leaves_membraned_an_empty_record() {
    let root = Root::new();
    for attempt in 0..20 {
        let cell = Cell::create(&root.path, &format!("cell-{}", 111 + attempt));
        let lock = cell.lock();
        let log = cell.log();
        let record = cell.record();
        match cell.start_until(&root.membraned(), lock, log, record, Instant::now()) {
            Err(StartError::ExecUnconfirmed { lock }) => {
                let report = Report::read(&cell.path);
                assert_eq!(
                    report.descriptor(3).kind,
                    "fifo",
                    "the late stand-in reads the record's pipe on 3; {report}"
                );
                assert_eq!(
                    report.text("record"),
                    "",
                    "an unconfirmed start writes no record; {report}"
                );
                assert_eq!(report.text("decoded"), "refused", "{report}");
                drop(lock);
                return;
            }
            Ok(()) | Err(StartError::RecordNotTaken) => {}
            other => panic!(
                "a start whose deadline has passed is unconfirmed, or confirmed before it polled, not {other:?}"
            ),
        }
    }
    panic!("none of 20 starts was still unconfirmed when its deadline had passed");
}

fn test_threads_is_a_usage_error() {
    assert_refused(
        &["--test-threads", "1", "--exact", "no_case_has_this_name"],
        "--test-threads",
    );
}

fn skip_is_a_usage_error_not_a_filter() {
    assert_refused(
        &[
            "--skip",
            "a_cell_holding_a_nul_is_refused_before_any_fork",
            "--exact",
            "no_case_has_this_name",
        ],
        "--skip",
    );
}

fn assert_refused(arguments: &[&str], flag: &str) {
    let ran = Command::new(this_executable())
        .args(arguments)
        .env_remove(CASE)
        .env_remove(DEADLINE)
        .env_remove(SCRATCH)
        .stdin(Stdio::null())
        .output()
        .expect("the runner starts");
    let stdout = String::from_utf8_lossy(&ran.stdout);
    let stderr = String::from_utf8_lossy(&ran.stderr);
    assert_eq!(
        ran.status.code(),
        Some(2),
        "the runner refuses {arguments:?} with exit 2; it printed {stdout}{stderr}"
    );
    assert!(stderr.contains(flag), "the refusal names {flag}: {stderr}");
    assert!(
        !stdout.contains("running"),
        "the refused runner runs no case: {stdout}"
    );
}

fn lock_never_free_until_exit() {
    let root = Root::new();
    let cell = Cell::create(&root.path, "cell-105");
    let lock = cell.lock();
    let probe = Probe::start(&root.path, "cell-105");
    started(cell.start(&root.membraned(), lock, cell.log(), cell.record()));
    probe.start_returned();
    let report = Report::read(&cell.path);
    match probe.next() {
        Seen::HeldAfterStart => {}
        Seen::Free(_) => panic!("the lock was free before the stand-in was released; {report}"),
        Seen::Unexpected(found) => panic!("the probe finds the lock held or free, not {found}"),
    }
    let released_at = Instant::now();
    let exited_at = release(&cell.path);
    match probe.next() {
        Seen::Free(freed_at) => {
            assert!(
                freed_at > released_at,
                "the lock was never free before the stand-in was released; {report}"
            );
            assert!(
                freed_at < exited_at + Duration::from_secs(1),
                "the lock is free within a second of the stand-in's exit, not {:?} after it",
                freed_at.saturating_duration_since(exited_at)
            );
        }
        Seen::HeldAfterStart => unreachable!("the probe reports Held once"),
        Seen::Unexpected(found) => panic!("the probe finds the lock held or free, not {found}"),
    }
}

fn survives_the_controller_group() {
    let root = Root::new();
    let cell_path = root.path.join("cells").join("cell-102");
    let mut standin = Standin {
        cell: cell_path.clone(),
        expected: false,
    };
    let mut controller = ChildController::spawn(&root.path);
    let said = controller.first_line();
    assert_eq!(
        said.as_deref(),
        Some(STARTED),
        "the child controller wrote to stderr: {}",
        controller.stderr()
    );
    standin.expected = true;
    assert_eq!(
        kill_group(controller.group(), libc::SIGKILL),
        0,
        "the child controller's group is killed"
    );
    let status = controller.wait();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    let report = Report::read(&cell_path);
    let pid = libc::pid_t::try_from(report.number("pid")).expect("a pid fits pid_t");
    thread::sleep(Duration::from_millis(100));
    assert!(
        is_running(pid),
        "the stand-in outlives its controller's group; {report}"
    );
    assert_eq!(report.number("sid"), report.number("pid"), "{report}");
    assert_reparented(&report);
}

fn child_controller(root: &Path) -> ! {
    let parent = parent_pid();
    let cell = Cell::create(root, "cell-102");
    let membraned = membraned_in(root);
    started(cell.start(&membraned, cell.lock(), cell.log(), cell.record()));
    println!("{STARTED}");
    std::io::stdout().flush().expect("stdout flushes");
    while parent_pid() == parent {
        thread::sleep(Duration::from_millis(50));
    }
    drop(cell);
    std::process::exit(0)
}

struct ChildController(Child, PathBuf);

impl ChildController {
    fn spawn(root: &Path) -> ChildController {
        let stderr = root.join("controller.stderr");
        let child = Command::new(this_executable())
            .env(CONTROLLER, root)
            .env_remove(CASE)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(File::create(&stderr).expect("the child controller's stderr file is made"))
            .spawn()
            .expect("the child controller starts");
        record_pid(root, "controller", child.id());
        ChildController(child, stderr)
    }

    fn first_line(&mut self) -> Option<String> {
        first_line(&mut self.0)
    }

    fn group(&self) -> libc::pid_t {
        libc::pid_t::try_from(self.0.id()).expect("a pid fits pid_t")
    }

    fn wait(&mut self) -> std::process::ExitStatus {
        self.0.wait().expect("the child controller is reaped")
    }

    fn stderr(&self) -> String {
        std::fs::read_to_string(&self.1).unwrap_or_default()
    }
}

impl Drop for ChildController {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            kill_group(self.group(), libc::SIGKILL);
            let _ = self.0.wait();
        }
    }
}

fn exec_failure_returns_the_lock() {
    let root = Root::new();
    let cell = Cell::create(&root.path, "cell-106");
    let missing =
        RecipePath::parse(&format!("{}/missing", root.path.display())).expect("a canonical path");
    match cell.start(&missing, cell.lock(), cell.log(), cell.record()) {
        Err(StartError::Exec { lock, step, errno }) => {
            assert_eq!((step, errno), (StartStep::Exec, libc::ENOENT));
            assert!(
                matches!(cell.dir.probe_supervisor_lock(), Ok(LockProbe::Held)),
                "the returned lock is still held"
            );
            assert_no_child_left();
            drop(lock);
            assert_frees_within_a_second(&cell.dir);
        }
        other => panic!("a missing membraned is an Exec failure, not {other:?}"),
    }
}

fn a_start_returns_long_before_its_deadline() {
    let root = Root::new();
    let missing =
        RecipePath::parse(&format!("{}/missing", root.path.display())).expect("a canonical path");
    {
        let cell = Cell::create(&root.path, "cell-111");
        let began = Instant::now();
        let result = cell.start(&missing, cell.lock(), cell.log(), cell.record());
        assert_returned_within_half(began, "a start whose exec fails");
        assert!(
            matches!(
                result,
                Err(StartError::Exec {
                    step: StartStep::Exec,
                    errno: libc::ENOENT,
                    ..
                })
            ),
            "a missing membraned fails at its exec with ENOENT, not {result:?}"
        );
    }
    let cell = Cell::create(&root.path, "cell-112");
    let began = Instant::now();
    let result = cell.start(&root.membraned(), cell.lock(), cell.log(), cell.record());
    assert_returned_within_half(began, "a start whose exec succeeds");
    started(result);
}

fn assert_returned_within_half(began: Instant, start: &str) {
    let took = began.elapsed();
    let deadline = case_deadline().saturating_duration_since(began);
    assert!(
        took < deadline / 2,
        "{start} returns in under half of its {deadline:?} deadline, not after {took:?}"
    );
}

fn early_exit_is_closed_pipe() {
    let root = Root::new();
    let cell = Cell::create(&root.path, "cell-201");
    let _default = SigpipeDefault::on_macos();
    match cell.start(
        &root.membraned(),
        cell.lock(),
        cell.log(),
        oversized_record(),
    ) {
        Err(StartError::SupervisorClosedPipe) => {}
        other => panic!("a reader that exits early closes the pipe, not {other:?}"),
    }
    assert_no_child_left();
}

fn unread_record_times_out() {
    let root = Root::new();
    let cell = Cell::create(&root.path, "cell-301");
    let lock = cell.lock();
    let log = cell.log();
    let deadline = Instant::now() + Duration::from_secs(1);
    match cell.start_until(&root.membraned(), lock, log, oversized_record(), deadline) {
        Err(StartError::RecordNotTaken) => {}
        other => panic!("an unread record times out, not {other:?}"),
    }
    let returned = Instant::now();
    assert!(returned >= deadline, "the write waited until its deadline");
    assert!(
        returned - deadline < Duration::from_millis(800),
        "the write gave up within a second, {:?} after its deadline",
        returned - deadline
    );
    assert_no_child_left();
}

fn lock_already_at_four() {
    let mut low = LowDescriptors::reserve();
    let root = Root::new();
    let cell = Cell::create(&root.path, "cell-104");
    low.free(4);
    let lock = cell.lock();
    assert_eq!(lock.as_fd().as_raw_fd(), 4, "the controller's lock is at 4");
    started(cell.start(&root.membraned(), lock, cell.log(), cell.record()));
    let report = Report::read(&cell.path);
    assert_descriptors(&report, &cell);
}

fn sources_sit_on_each_others_targets() {
    let mut low = LowDescriptors::reserve();
    let root = Root::new();
    let cell = Cell::create(&root.path, "cell-103");
    low.free(3);
    let lock = cell.lock();
    low.free(4);
    let log = cell.log();
    assert_eq!(
        (lock.as_fd().as_raw_fd(), log.as_raw_fd()),
        (3, 4),
        "the lock is at 3 and the log at 4"
    );
    let record = cell.record();
    let stdin = close_stdin();
    started(cell.start(&root.membraned(), lock, log, record));
    drop(stdin);
    let report = Report::read(&cell.path);
    assert_descriptors(&report, &cell);
}

fn a_cell_holding_a_nul_is_refused_before_any_fork() {
    let root = Root::new();
    let cell = Cell::create(&root.path, "cell-107");
    let start = SupervisorStart {
        membraned: &root.membraned(),
        cell: &CellId::from("cell-1\0"),
        lock: cell.lock(),
        log: cell.log(),
        record: cell.record(),
    };
    match start_supervisor(start, case_deadline()) {
        Err(StartError::Prepare { lock, source }) => {
            assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput);
            assert!(
                matches!(cell.dir.probe_supervisor_lock(), Ok(LockProbe::Held)),
                "the returned lock is still held"
            );
            drop(lock);
        }
        other => panic!("a cell ID holding a NUL is refused, not {other:?}"),
    }
    assert_no_child_left();
}

fn no_handler_of_the_controller_runs_in_a_child() {
    assert_eq!(
        own_pgid(),
        std::process::id(),
        "the case leads its own process group, so its signals reach no other process"
    );
    let root = Root::new();
    let caught = Caught::install(&root.path.join("handler.pids"));
    let cell = Cell::create(&root.path, "cell-108");
    let missing =
        RecipePath::parse(&format!("{}/missing", root.path.display())).expect("a canonical path");
    let storm = Storm::start();
    for start in 0..40 {
        let lock = cell.lock();
        let outcome = cell.start(&missing, lock, cell.log(), cell.record());
        assert_eq!(
            caught.pids_other_than_this(),
            Vec::<libc::pid_t>::new(),
            "by start {start} the controller's SIGUSR1 handler ran only in the controller; the start gave {outcome:?}"
        );
        match outcome {
            Err(StartError::Exec {
                lock,
                step: StartStep::Exec,
                errno: libc::ENOENT,
            }) => drop(lock),
            other => panic!(
                "start {start} of a missing membraned under a group signal storm fails at its exec with ENOENT, not {other:?}"
            ),
        }
        assert_no_child_left();
        std::fs::remove_file(cell.path.join("supervisor.lock")).expect("the lock is removed");
    }
    let sent = storm.stop();
    assert!(sent > 100, "the storm sent {sent} signals");
    assert_eq!(
        caught.pids_other_than_this(),
        Vec::<libc::pid_t>::new(),
        "the controller's SIGUSR1 handler ran only in the controller"
    );
}

static HANDLER_LOG: std::sync::atomic::AtomicPtr<libc::c_char> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static HANDLER_OWNER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

extern "C" fn note_foreign_handler(_signal: libc::c_int) {
    let me = own_pid();
    let log = HANDLER_LOG.load(std::sync::atomic::Ordering::Relaxed);
    if me != HANDLER_OWNER.load(std::sync::atomic::Ordering::Relaxed) && !log.is_null() {
        let bytes = me.to_ne_bytes();
        // SAFETY: `log` is the NUL-terminated path Caught leaked, which lives until the process
        // ends. `bytes` is a local of four bytes that write only reads. getpid, open, write and
        // close are async-signal-safe, which is all a signal handler may call. Opening the log
        // anew reaches it from a child whose descriptors the start has already replaced.
        #[allow(unsafe_code)]
        unsafe {
            let fd = libc::open(
                log,
                libc::O_WRONLY | libc::O_APPEND | libc::O_CREAT | libc::O_CLOEXEC,
                0o600 as libc::c_uint,
            );
            if fd >= 0 {
                libc::write(fd, bytes.as_ptr().cast(), bytes.len());
                libc::close(fd);
            }
        }
    }
}

struct Caught(PathBuf);

impl Caught {
    #[allow(unsafe_code)]
    fn install(log: &Path) -> Caught {
        let path = std::ffi::CString::new(log.as_os_str().as_encoded_bytes())
            .expect("the handler's log path holds no NUL");
        HANDLER_LOG.store(path.into_raw(), std::sync::atomic::Ordering::Relaxed);
        HANDLER_OWNER.store(own_pid(), std::sync::atomic::Ordering::Relaxed);
        // SAFETY: An all-zero libc::sigaction is a valid empty action with no flags. The handler
        // only calls getpid, open, write and close, which are async-signal-safe. `action` is a
        // local that outlives the calls, and sigaction copies what it reads.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = note_foreign_handler as *const () as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            assert_eq!(
                libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut()),
                0
            );
        }
        Caught(log.to_path_buf())
    }

    fn pids_other_than_this(&self) -> Vec<libc::pid_t> {
        let bytes = std::fs::read(&self.0).unwrap_or_default();
        let (pids, _) = bytes.as_chunks::<4>();
        pids.iter()
            .map(|pid| libc::pid_t::from_ne_bytes(*pid))
            .collect()
    }
}

struct Storm {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    sender: thread::JoinHandle<usize>,
}

impl Storm {
    #[allow(unsafe_code)]
    fn start() -> Storm {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopping = std::sync::Arc::clone(&stop);
        let group = own_pid();
        let sender = thread::spawn(move || {
            let mut sent = 0;
            while !stopping.load(std::sync::atomic::Ordering::Relaxed) {
                assert_eq!(
                    kill_group(group, libc::SIGUSR1),
                    0,
                    "the case signals its own group"
                );
                sent += 1;
                thread::sleep(Duration::from_micros(20));
            }
            sent
        });
        Storm { stop, sender }
    }

    fn stop(self) -> usize {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.sender.join().expect("the storm thread ends")
    }
}

#[allow(unsafe_code)]
fn own_pid() -> libc::pid_t {
    // SAFETY: getpid takes no arguments and is async-signal-safe.
    unsafe { libc::getpid() }
}

fn assert_descriptors(report: &Report, cell: &Cell) {
    let null = std::fs::metadata("/dev/null").expect("/dev/null exists");
    for fd in [0, 1] {
        let found = report.descriptor(fd);
        assert_eq!(
            (found.kind.as_str(), found.rdev),
            ("char", null.rdev()),
            "descriptor {fd} is /dev/null; {report}"
        );
    }
    let log = std::fs::metadata(cell.path.join("membraned.log")).expect("the log exists");
    assert_eq!(
        report.descriptor(2).identity(),
        Some((log.dev(), log.ino())),
        "descriptor 2 is membraned.log; {report}"
    );
    assert_eq!(
        report.descriptor(3).kind,
        "fifo",
        "descriptor 3 is the config pipe; {report}"
    );
    let FileIdentity { dev, ino } = cell
        .dir
        .supervisor_lock_identity()
        .expect("the lock has an identity");
    assert_eq!(
        report.descriptor(4).identity(),
        Some((dev, ino)),
        "descriptor 4 is supervisor.lock; {report}"
    );
    assert!(
        report.flag("lock_taken"),
        "flock on descriptor 4 succeeds; {report}"
    );
}

fn assert_reparented(report: &Report) {
    let ppid = report.number("ppid");
    assert_ne!(ppid, u64::from(std::process::id()), "{report}");
    if cfg!(target_os = "macos") {
        assert_eq!(ppid, 1, "launchd is the stand-in's parent; {report}");
    }
}

fn assert_frees_within_a_second(dir: &CellDir) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match dir.probe_supervisor_lock() {
            Ok(LockProbe::Free(_)) => return,
            Ok(LockProbe::Held) if Instant::now() < deadline => thread::sleep(POLL),
            other => panic!("the lock frees within a second, not {other:?}"),
        }
    }
}

fn started(result: Result<(), StartError>) {
    if let Err(error) = result {
        panic!("the supervisor starts: {error}");
    }
    assert_no_child_left();
}

fn assert_no_child_left() {
    let (reaped, errno) = wait_any();
    assert_eq!(
        (reaped, errno),
        (-1, libc::ECHILD),
        "no waitable child remains"
    );
}

fn oversized_record() -> Vec<u8> {
    let mut record = vec![b'x'; ONE_MIB];
    record.push(b'\n');
    record
}

fn first_line(child: &mut Child) -> Option<String> {
    let stdout = child.stdout.take().expect("stdout is piped");
    let (sent, received) = mpsc::channel();
    thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = sent.send(line.trim_end().to_owned());
    });
    received.recv_timeout(until_the_case_deadline()).ok()
}

fn case_deadline() -> Instant {
    static AT: OnceLock<Instant> = OnceLock::new();
    *AT.get_or_init(|| {
        let left = std::env::var(DEADLINE)
            .ok()
            .and_then(|at| at.parse::<u128>().ok())
            .map_or(CASE_DEADLINE, |at| {
                Duration::from_millis(
                    u64::try_from(at.saturating_sub(epoch_millis())).unwrap_or(u64::MAX),
                )
            });
        Instant::now() + left.saturating_sub(DEADLINE_MARGIN)
    })
}

fn until_the_case_deadline() -> Duration {
    case_deadline().saturating_duration_since(Instant::now())
}

fn epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
}

fn this_executable() -> PathBuf {
    std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .expect("the test executable has a canonical path")
}

fn membraned_in(root: &Path) -> RecipePath {
    RecipePath::parse(&format!("{}/membraned", root.display()))
        .expect("the stand-in's path is canonical")
}

struct Root {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Root {
    fn new() -> Root {
        let dir = match std::env::var_os(SCRATCH) {
            Some(scratch) => tempfile::tempdir_in(scratch),
            None => tempfile::tempdir(),
        }
        .expect("a temporary directory");
        let path = dir
            .path()
            .canonicalize()
            .expect("the temporary directory has a canonical path");
        std::os::unix::fs::symlink(this_executable(), path.join("membraned"))
            .expect("the stand-in is linked into the root");
        Root { _dir: dir, path }
    }

    fn membraned(&self) -> RecipePath {
        membraned_in(&self.path)
    }
}

struct Cell {
    _root: InstanceRoot,
    _writer: WriterLock,
    dir: CellDir,
    id: CellId,
    instance: PathBuf,
    path: PathBuf,
    standin_expected: std::cell::Cell<bool>,
}

impl Cell {
    fn create(root: &Path, id: &str) -> Cell {
        let instance = InstanceRoot::open(root).expect("the instance root opens");
        let writer = instance.lock().expect("the writer lock is free");
        let id = CellId::from(id);
        let dir = instance
            .create_cell_dir(&writer, &id)
            .expect("the cell directory is created");
        let path = root.join("cells").join(id.as_str());
        Cell {
            _root: instance,
            _writer: writer,
            dir,
            id,
            instance: root.to_path_buf(),
            path,
            standin_expected: std::cell::Cell::new(false),
        }
    }

    fn lock(&self) -> SupervisorLock {
        self.dir
            .create_supervisor_lock()
            .expect("the supervisor lock is created")
    }

    fn log(&self) -> File {
        self.dir.open_supervisor_log().expect("membraned.log opens")
    }

    fn record(&self) -> Vec<u8> {
        let limits = PlatformLimits::host().expect("this host has limits");
        SupervisorConfig {
            cell: self.id.clone(),
            instance_root: RecipePath::parse(&self.instance.display().to_string())
                .expect("the root is canonical"),
            runtime_recipe: recipe(&limits),
            boot_deadline_ms: 1_000,
        }
        .encode()
        .expect("the record encodes")
    }

    fn start(
        &self,
        membraned: &RecipePath,
        lock: SupervisorLock,
        log: File,
        record: Vec<u8>,
    ) -> Result<(), StartError> {
        self.start_until(membraned, lock, log, record, case_deadline())
    }

    fn start_until(
        &self,
        membraned: &RecipePath,
        lock: SupervisorLock,
        log: File,
        record: Vec<u8>,
        deadline: Instant,
    ) -> Result<(), StartError> {
        let start = SupervisorStart {
            membraned,
            cell: &self.id,
            lock,
            log,
            record,
        };
        let result = start_supervisor(start, deadline);
        let executed = matches!(
            result,
            Ok(())
                | Err(StartError::ExecUnconfirmed { .. }
                    | StartError::SupervisorClosedPipe
                    | StartError::RecordNotTaken
                    | StartError::Write(_))
        );
        if executed && membraned.as_path() == self.instance.join("membraned") {
            self.standin_expected.set(true);
        }
        result
    }
}

impl Drop for Cell {
    fn drop(&mut self) {
        stop_standin(&self.path, self.standin_expected.get());
    }
}

struct Standin {
    cell: PathBuf,
    expected: bool,
}

impl Drop for Standin {
    fn drop(&mut self) {
        stop_standin(&self.cell, self.expected);
    }
}

fn stop_standin(cell: &Path, expected: bool) {
    let _ = File::create(cell.join("release"));
    loop {
        if let Some(pid) = standin_pid(cell) {
            if is_our_standin(pid) {
                kill_and_wait(pid);
            }
            return;
        }
        if !expected || Instant::now() >= case_deadline() {
            return;
        }
        thread::sleep(POLL);
    }
}

fn standin_pid(cell: &Path) -> Option<libc::pid_t> {
    std::fs::read_to_string(cell.join("standin.pid"))
        .ok()
        .and_then(|text| text.trim().parse().ok())
}

fn record_pid(root: &Path, role: &str, pid: u32) {
    if let Some(pids) = root.parent().map(|scratch| scratch.join(PIDS))
        && pids.is_dir()
    {
        let _ = File::create(pids.join(format!("{role}-{pid}")));
    }
}

#[allow(unsafe_code)]
fn kill_and_wait(pid: libc::pid_t) {
    // SAFETY: kill takes integers. The caller checked just before that `pid` is a process this
    // test binary started.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    let deadline = Instant::now() + Duration::from_secs(2);
    while is_running(pid) && Instant::now() < deadline {
        thread::sleep(POLL);
    }
}

fn is_our_standin(pid: libc::pid_t) -> bool {
    runs_this_executable(pid) && session_of(pid) == Some(pid)
}

fn is_our_controller(pid: libc::pid_t) -> bool {
    runs_this_executable(pid) && group_of(pid) == Some(pid)
}

fn runs_this_executable(pid: libc::pid_t) -> bool {
    is_running(pid) && executable_of(pid).is_some_and(|path| path == this_executable())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn executable_of(pid: libc::pid_t) -> Option<PathBuf> {
    let mut path = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `path` is a local buffer of the length passed, which proc_pidpath fills.
    let length = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
    let length = usize::try_from(length).ok().filter(|length| *length > 0)?;
    Some(PathBuf::from(
        <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(path.get(..length)?),
    ))
}

#[cfg(target_os = "linux")]
fn executable_of(pid: libc::pid_t) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

#[allow(unsafe_code)]
fn session_of(pid: libc::pid_t) -> Option<libc::pid_t> {
    // SAFETY: getsid takes an integer.
    Some(unsafe { libc::getsid(pid) }).filter(|sid| *sid > 0)
}

#[allow(unsafe_code)]
fn group_of(pid: libc::pid_t) -> Option<libc::pid_t> {
    // SAFETY: getpgid takes an integer.
    Some(unsafe { libc::getpgid(pid) }).filter(|group| *group > 0)
}

#[allow(unsafe_code)]
fn parent_pid() -> libc::pid_t {
    // SAFETY: getppid takes no arguments.
    unsafe { libc::getppid() }
}

fn release(cell: &Path) -> Instant {
    let _ = File::create(cell.join("release"));
    let Some(pid) = std::fs::read_to_string(cell.join("standin.pid"))
        .ok()
        .and_then(|text| text.trim().parse::<libc::pid_t>().ok())
    else {
        return Instant::now();
    };
    while is_running(pid) && Instant::now() < case_deadline() {
        thread::sleep(POLL);
    }
    Instant::now()
}

fn recipe(limits: &PlatformLimits) -> RuntimeRecipe {
    let artifact =
        |path: &str, digit: char| json!({"path": path, "sha256": digit.to_string().repeat(64)});
    let value = json!({
        "version": 1,
        "vcpus": 1,
        "memory_mib": 512,
        "kernel": artifact("/opt/cell/kernel", '1'),
        "kernel_format": match limits.architecture() {
            Architecture::X86_64 => "elf",
            Architecture::Aarch64 => "raw",
        },
        "initramfs": artifact("/opt/cell/initramfs", '2'),
        "root_image": artifact("/opt/cell/root.raw", '3'),
        "writable_root": "/var/cell/c1/root.raw",
        "control_path": "/var/cell/c1/control.sock",
        "data_path": "/var/cell/c1/data.sock",
        "helper": artifact("/opt/cell/krun-helper", '4'),
        "libraries": [artifact("/opt/cell/lib/libkrun.1.dylib", '5')],
        "host_policy": artifact("/opt/cell/host.sb", '6'),
        "guest_policy": artifact("/opt/cell/guest.json", '7'),
        "architecture": limits.architecture().as_str(),
    });
    let mut line = serde_json::to_vec(&value).expect("the recipe serializes");
    line.push(b'\n');
    RuntimeRecipe::parse(&line, limits).expect("the recipe is valid")
}

enum Seen {
    HeldAfterStart,
    Free(Instant),
    Unexpected(String),
}

struct Probe {
    seen: mpsc::Receiver<Seen>,
    start_returned: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Probe {
    fn start(root: &Path, cell: &str) -> Probe {
        let instance = InstanceRoot::open(root).expect("the probe opens the root");
        let dir = instance
            .cell_dir(&CellId::from(cell))
            .expect("the probe opens the cell");
        let start_returned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let returned = std::sync::Arc::clone(&start_returned);
        let (sent, seen) = mpsc::channel();
        thread::spawn(move || {
            let mut held_reported = false;
            while Instant::now() < case_deadline() {
                let after_start = returned.load(std::sync::atomic::Ordering::SeqCst);
                match dir.probe_supervisor_lock() {
                    Ok(LockProbe::Held) if after_start && !held_reported => {
                        held_reported = true;
                        let _ = sent.send(Seen::HeldAfterStart);
                    }
                    Ok(LockProbe::Held) => {}
                    Ok(LockProbe::Free(_)) => {
                        let _ = sent.send(Seen::Free(Instant::now()));
                        return;
                    }
                    other => {
                        let _ = sent.send(Seen::Unexpected(format!("{other:?}")));
                        return;
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        Probe {
            seen,
            start_returned,
        }
    }

    fn start_returned(&self) {
        self.start_returned
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn next(&self) -> Seen {
        self.seen
            .recv_timeout(until_the_case_deadline())
            .expect("the probe sees the lock change by the case's deadline")
    }
}

struct Report(Value);

struct Descriptor {
    kind: String,
    dev: u64,
    ino: u64,
    rdev: u64,
}

impl Descriptor {
    fn identity(&self) -> Option<(u64, u64)> {
        matches!(self.kind.as_str(), "char" | "fifo" | "file" | "other")
            .then_some((self.dev, self.ino))
    }
}

impl Report {
    fn read(cell: &Path) -> Report {
        let path = cell.join("standin.json");
        loop {
            if let Ok(text) = std::fs::read_to_string(&path) {
                return Report(serde_json::from_str(&text).expect("the report is JSON"));
            }
            assert!(
                Instant::now() < case_deadline(),
                "the stand-in reported by the case's deadline at {}",
                path.display()
            );
            thread::sleep(POLL);
        }
    }

    fn field(&self, key: &str) -> &Value {
        self.0
            .get(key)
            .unwrap_or_else(|| panic!("the report has {key}: {self}"))
    }

    fn number(&self, key: &str) -> u64 {
        self.field(key)
            .as_u64()
            .unwrap_or_else(|| panic!("{key} is a number: {self}"))
    }

    fn flag(&self, key: &str) -> bool {
        self.field(key)
            .as_bool()
            .unwrap_or_else(|| panic!("{key} is a flag: {self}"))
    }

    fn text(&self, key: &str) -> String {
        self.field(key)
            .as_str()
            .unwrap_or_else(|| panic!("{key} is a string: {self}"))
            .to_owned()
    }

    fn strings(&self, key: &str) -> Vec<String> {
        self.field(key)
            .as_array()
            .unwrap_or_else(|| panic!("{key} is a list: {self}"))
            .iter()
            .map(|value| value.as_str().unwrap_or_default().to_owned())
            .collect()
    }

    fn numbers(&self, key: &str) -> Vec<u64> {
        self.field(key)
            .as_array()
            .unwrap_or_else(|| panic!("{key} is a list: {self}"))
            .iter()
            .map(|value| value.as_u64().unwrap_or(u64::MAX))
            .collect()
    }

    fn descriptor(&self, fd: usize) -> Descriptor {
        let found = &self.field("descriptors")[fd];
        Descriptor {
            kind: found["kind"].as_str().unwrap_or_default().to_owned(),
            dev: found["dev"].as_u64().unwrap_or_default(),
            ino: found["ino"].as_u64().unwrap_or_default(),
            rdev: found["rdev"].as_u64().unwrap_or_default(),
        }
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the stand-in reported {}", self.0)
    }
}

struct Planted(Vec<RawFd>);

impl Planted {
    #[allow(unsafe_code)]
    fn at(targets: &[RawFd]) -> Planted {
        let planted = targets
            .iter()
            .map(|&target| {
                assert!(
                    !is_open(target),
                    "descriptor {target} is free before it is planted"
                );
                let copied = inheritable_null(|null| {
                    // SAFETY: dup2 takes integers only, and `null` is open for the call. `target`
                    // was free just above, so dup2 replaces nothing.
                    unsafe { libc::dup2(null, target) }
                });
                assert_eq!(copied, target, "descriptor {target} is planted");
                assert!(
                    !is_close_on_exec(target),
                    "the planted descriptor {target} is inheritable"
                );
                target
            })
            .collect();
        Planted(planted)
    }

    #[allow(unsafe_code)]
    fn low_and_last() -> Planted {
        // SAFETY: getdtablesize takes no arguments.
        let last = unsafe { libc::getdtablesize() } - 1;
        let low = inheritable_null(|null| {
            // SAFETY: fcntl with F_DUPFD takes integers only, and `null` is open for the call.
            unsafe { libc::fcntl(null, libc::F_DUPFD, 5) }
        });
        let high = inheritable_null(|null| {
            // SAFETY: dup2 takes integers only, and `null` is open for the call. `last` is the
            // highest descriptor this process may have, which nothing else here holds.
            unsafe { libc::dup2(null, last) }
        });
        assert!(
            low >= 5 && high == last,
            "descriptors {low} and {high} are planted"
        );
        assert!(
            !is_close_on_exec(low) && !is_close_on_exec(high),
            "the planted descriptors are inheritable"
        );
        Planted(vec![low, high])
    }
}

impl Drop for Planted {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        for &fd in &self.0 {
            // SAFETY: `fd` is a descriptor this Planted made and owns, closed once here.
            unsafe { libc::close(fd) };
        }
    }
}

#[cfg(target_os = "macos")]
fn descriptor_top() -> RawFd {
    per_process_limit() - 1
}

#[cfg(target_os = "linux")]
fn descriptor_top() -> RawFd {
    let hard = descriptor_limit().rlim_max.min(1 << 20);
    RawFd::try_from(hard).expect("the hard limit fits a descriptor") - 1
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn per_process_limit() -> RawFd {
    let mut limit: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: The name is a static NUL-terminated string. `limit` and `size` are locals that
    // sysctlbyname may write, `size` holds the bytes `limit` has, and no new value is passed.
    let read = unsafe {
        libc::sysctlbyname(
            c"kern.maxfilesperproc".as_ptr(),
            (&raw mut limit).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(read, 0, "kern.maxfilesperproc is readable");
    limit
}

#[allow(unsafe_code)]
fn descriptor_limit() -> libc::rlimit {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a local that getrlimit writes.
    let read = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    assert_eq!(read, 0, "the descriptor limit is readable");
    limit
}

#[allow(unsafe_code)]
fn set_descriptor_limit(limit: libc::rlimit) -> std::io::Result<()> {
    // SAFETY: `limit` is a local that setrlimit only reads. This case runs in its own process,
    // so the limit reaches no other test.
    match unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

#[allow(unsafe_code)]
fn descriptor_table_size() -> RawFd {
    // SAFETY: getdtablesize takes no arguments.
    unsafe { libc::getdtablesize() }
}

struct SoftLimit(libc::rlimit);

impl SoftLimit {
    fn raised_past(top: RawFd) -> SoftLimit {
        let saved = descriptor_limit();
        let wanted = libc::rlim_t::try_from(top + 1).expect("a positive limit");
        if saved.rlim_cur < wanted {
            set_descriptor_limit(libc::rlimit {
                rlim_cur: wanted,
                rlim_max: saved.rlim_max,
            })
            .unwrap_or_else(|error| {
                panic!(
                    "the soft descriptor limit rises to {wanted} under the hard limit {}: {error}",
                    saved.rlim_max
                )
            });
        }
        assert!(
            descriptor_table_size() > top,
            "the descriptor table reaches {top} once the soft limit is raised"
        );
        SoftLimit(saved)
    }

    fn lower_to(&self, soft: libc::rlim_t) {
        set_descriptor_limit(libc::rlimit {
            rlim_cur: soft,
            rlim_max: self.0.rlim_max,
        })
        .expect("the soft descriptor limit is lowered");
        assert_eq!(
            libc::rlim_t::try_from(descriptor_table_size()).ok(),
            Some(soft),
            "the descriptor table ends at the lowered soft limit"
        );
    }
}

impl Drop for SoftLimit {
    fn drop(&mut self) {
        let _ = set_descriptor_limit(self.0);
    }
}

#[allow(unsafe_code)]
fn is_open(fd: RawFd) -> bool {
    // SAFETY: fcntl with F_GETFD takes no pointer.
    unsafe { libc::fcntl(fd, libc::F_GETFD) != -1 }
}

#[allow(unsafe_code)]
fn inheritable_null(copy: impl FnOnce(RawFd) -> RawFd) -> RawFd {
    // SAFETY: The path is a static NUL-terminated string, and open keeps no pointer to it.
    let opened = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
    assert!(opened >= 0, "/dev/null opens");
    let copied = copy(opened);
    // SAFETY: `opened` is the descriptor opened above, which nothing else owns.
    unsafe { libc::close(opened) };
    copied
}

struct LowDescriptors([Option<OwnedFd>; 2]);

impl LowDescriptors {
    #[allow(unsafe_code)]
    fn reserve() -> LowDescriptors {
        let null = inheritable_null(|null| {
            // SAFETY: fcntl with F_DUPFD_CLOEXEC takes integers only, and `null` is open for the
            // call.
            unsafe { libc::fcntl(null, libc::F_DUPFD_CLOEXEC, 10) }
        });
        assert!(null >= 10, "/dev/null is copied to 10 or above");
        // SAFETY: fcntl made `null` a new descriptor, which nothing else owns.
        let null = unsafe { OwnedFd::from_raw_fd(null) };
        let held = [3, 4].map(|target| {
            // SAFETY: dup2 takes integers only, and `null` keeps its descriptor open. This case
            // runs in its own process, so whatever it replaces at 3 or 4 is no other test's.
            let copied = unsafe { libc::dup2(null.as_raw_fd(), target) };
            assert_eq!(copied, target, "descriptor {target} is reserved");
            // SAFETY: dup2 made `target` a descriptor that nothing else owns.
            Some(unsafe { OwnedFd::from_raw_fd(target) })
        });
        LowDescriptors(held)
    }

    fn free(&mut self, fd: RawFd) {
        let slot = usize::try_from(fd - 3).expect("3 or 4");
        assert!(
            self.0[slot].take().is_some(),
            "descriptor {fd} was reserved"
        );
    }
}

struct ClosedStdin(OwnedFd);

#[allow(unsafe_code)]
fn close_stdin() -> ClosedStdin {
    let saved = std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .expect("stdin is copied");
    // SAFETY: close takes an integer. This case runs in its own process and restores 0 when the
    // returned value drops.
    assert_eq!(unsafe { libc::close(0) }, 0, "descriptor 0 is closed");
    ClosedStdin(saved)
}

impl Drop for ClosedStdin {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: dup2 takes integers only, and the saved copy is open for the call.
        unsafe { libc::dup2(self.0.as_raw_fd(), 0) };
    }
}

struct SignalBlocked(libc::sigset_t);

impl SignalBlocked {
    #[allow(unsafe_code)]
    fn on_this_thread(signal: libc::c_int) -> SignalBlocked {
        // SAFETY: All zeroes is a valid sigset_t, and sigemptyset then sets both. They are locals
        // that outlive the calls, and pthread_sigmask changes only this thread's mask.
        unsafe {
            let mut previous: libc::sigset_t = std::mem::zeroed();
            let mut blocked: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut blocked);
            libc::sigaddset(&mut blocked, signal);
            assert_eq!(
                libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut previous),
                0
            );
            SignalBlocked(previous)
        }
    }
}

impl Drop for SignalBlocked {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: `self.0` is the mask saved when the signal was blocked, which pthread_sigmask
        // only reads.
        unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &self.0, std::ptr::null_mut()) };
    }
}

struct SigpipeDefault(Option<libc::sighandler_t>);

impl SigpipeDefault {
    #[allow(unsafe_code)]
    fn on_macos() -> SigpipeDefault {
        if !cfg!(target_os = "macos") {
            return SigpipeDefault(None);
        }
        // SAFETY: signal takes integers and SIG_DFL. This case runs in its own process, so the
        // default action reaches no other test.
        let previous = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
        assert_ne!(previous, libc::SIG_ERR, "SIGPIPE is set to its default");
        SigpipeDefault(Some(previous))
    }
}

impl Drop for SigpipeDefault {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if let Some(previous) = self.0 {
            // SAFETY: signal takes integers and the disposition saved above.
            unsafe { libc::signal(libc::SIGPIPE, previous) };
        }
    }
}

#[allow(unsafe_code)]
fn blocked_signals() -> Vec<libc::c_int> {
    // SAFETY: All zeroes is a valid sigset_t. A null new set only reads this thread's mask into the
    // local `mask`, which sigismember then only reads.
    unsafe {
        let mut mask: libc::sigset_t = std::mem::zeroed();
        assert_eq!(
            libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut mask),
            0
        );
        (1..=LAST_SIGNAL)
            .filter(|signal| libc::sigismember(&mask, *signal) == 1)
            .collect()
    }
}

#[allow(unsafe_code)]
fn ignores(signal: libc::c_int) -> bool {
    // SAFETY: libc::sigaction is a plain C struct, for which all zeroes is a valid value.
    let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
    // SAFETY: A null new action only reads the disposition into `current`, a local.
    unsafe { libc::sigaction(signal, std::ptr::null(), &mut current) };
    current.sa_sigaction == libc::SIG_IGN
}

#[allow(unsafe_code)]
fn is_close_on_exec(fd: RawFd) -> bool {
    // SAFETY: fcntl with F_GETFD takes no pointer.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    flags != -1 && flags & libc::FD_CLOEXEC != 0
}

#[allow(unsafe_code)]
fn wait_any() -> (libc::pid_t, i32) {
    let mut status = 0;
    // SAFETY: `status` is a local that waitpid may write.
    let reaped = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
    (
        reaped,
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
    )
}

#[allow(unsafe_code)]
fn kill_group(group: libc::pid_t, signal: libc::c_int) -> libc::c_int {
    // SAFETY: killpg takes integers. `group` is the process group of a child this case has not
    // reaped, so its number still names that group.
    unsafe { libc::killpg(group, signal) }
}

#[allow(unsafe_code)]
fn is_running(pid: libc::pid_t) -> bool {
    // SAFETY: kill with signal 0 takes integers and sends nothing.
    let exists = unsafe { libc::kill(pid, 0) } == 0;
    exists && !is_zombie(pid)
}

#[cfg(target_os = "linux")]
fn is_zombie(pid: libc::pid_t) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
    })
}

#[cfg(not(target_os = "linux"))]
fn is_zombie(_pid: libc::pid_t) -> bool {
    false
}

#[allow(unsafe_code)]
fn own_sid() -> u32 {
    // SAFETY: getsid takes an integer; 0 names this process.
    u32::try_from(unsafe { libc::getsid(0) }).expect("a session id")
}

#[allow(unsafe_code)]
fn own_pgid() -> u32 {
    // SAFETY: getpgid takes an integer; 0 names this process.
    u32::try_from(unsafe { libc::getpgid(0) }).expect("a process group id")
}

const SET_BY_A_COVERAGE_RUNTIME_AT_START: &str = "__LLVM_PROFILE_RT_INIT_ONCE";

fn given_environment() -> Vec<String> {
    std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .filter(|name| name != SET_BY_A_COVERAGE_RUNTIME_AT_START)
        .collect()
}

fn standin_cell(arguments: &[OsString]) -> Option<&str> {
    let [_, config, lock, cell] = arguments else {
        return None;
    };
    if config != "--config-fd=3" || lock != "--lock-fd=4" {
        return None;
    }
    cell.to_str()?.strip_prefix("--cell=")
}

#[derive(Clone, Copy)]
struct Inherited {
    open: [bool; 5],
    mask_empty: bool,
    ignored: [bool; LAST_SIGNAL as usize + 1],
}

static INHERITED: OnceLock<Inherited> = OnceLock::new();

/// The loader calls each function in this section once, before `main` and before the standard
/// library's start-up ignores SIGPIPE or reopens a closed 0, 1 or 2 on /dev/null, so
/// `record_inherited` sees what `execve` handed over. It needs nothing that start-up sets: it
/// makes plain libc calls and sets a OnceLock.
///
/// Neither entry collides with anything else the process loads. `__DATA,__mod_init_func` and
/// `.init_array` are the input sections for an object's initializer pointers. The linker adds
/// this entry to the ones other objects put there and writes one list per image. On macOS it
/// places that list in `__DATA_CONST,__mod_init_func`, or in `__TEXT,__init_offsets` when the
/// minimum macOS is 12.0 or later. The loader calls every entry in the list, so this entry adds
/// one call and replaces none. Each other image has its own list. The macOS half is measured.
/// The Linux half is reasoned, not measured: CI's ubuntu `gates` job runs this test on Linux,
/// which is only indirect evidence.
#[used]
#[allow(unsafe_code)]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
static RECORD_INHERITED: extern "C" fn() = record_inherited;

extern "C" fn record_inherited() {
    let _ = INHERITED.set(Inherited::now());
}

impl Inherited {
    #[allow(unsafe_code)]
    fn now() -> Inherited {
        let open = [0, 1, 2, 3, 4].map(|fd| {
            // SAFETY: fcntl with F_GETFD takes no pointer.
            unsafe { libc::fcntl(fd, libc::F_GETFD) != -1 }
        });
        // SAFETY: All zeroes is a valid sigset_t. A null new set only reads this thread's mask
        // into the local `mask`, which sigismember then only reads.
        let mask_empty = unsafe {
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut mask) == 0
                && (1..=LAST_SIGNAL).all(|signal| libc::sigismember(&mask, signal) != 1)
        };
        let mut ignored = [false; LAST_SIGNAL as usize + 1];
        for (signal, ignoring) in (1..=LAST_SIGNAL).zip(ignored.iter_mut().skip(1)) {
            *ignoring = ignores(signal);
        }
        Inherited {
            open,
            mask_empty,
            ignored,
        }
    }
}

mod standin {
    use super::*;

    pub(super) fn run(arguments: &[OsString], cell: &str) -> ! {
        let outcome = std::panic::catch_unwind(|| serve(arguments, cell));
        exit(if outcome.is_ok() { 0 } else { 3 })
    }

    fn serve(arguments: &[OsString], cell: &str) {
        let root = Path::new(&arguments[0])
            .parent()
            .expect("the stand-in sits in the instance root");
        let cell_path = root.join("cells").join(cell);
        record_pid(root, "standin", process_id());
        write_atomically(
            &cell_path,
            "standin.pid",
            process_id().to_string().as_bytes(),
        );
        let open_above_four = open_descriptors_from(5);
        let number: u64 = cell
            .strip_prefix("cell-")
            .and_then(|digits| digits.parse().ok())
            .expect("a cell-<n> ID");
        match number / 100 {
            1 => {
                let report = report(arguments, root, open_above_four);
                write_atomically(&cell_path, "standin.json", report.to_string().as_bytes());
                wait_for_release(&cell_path);
            }
            2 => {}
            3 => wait_for_release(&cell_path),
            _ => panic!("{cell} names no stand-in behavior"),
        }
    }

    fn report(arguments: &[OsString], root: &Path, open_above_four: Vec<RawFd>) -> Value {
        let inherited = INHERITED.get().copied();
        let descriptors: Vec<Value> = inherited.map_or_else(Vec::new, |inherited| {
            (0..5)
                .zip(inherited.open)
                .map(|(fd, open)| described(fd, open))
                .collect()
        });
        let record = if inherited.is_some_and(|inherited| inherited.open[3]) {
            read_descriptor_three()
        } else {
            Vec::new()
        };
        let decoded = PlatformLimits::host()
            .ok()
            .and_then(|limits| SupervisorConfig::decode(&record, &limits).ok())
            .filter(|config| config.instance_root.as_path() == root)
            .map_or_else(|| "refused".to_owned(), |config| config.cell.to_string());
        let ppid = reparented_parent();
        json!({
            "argv": arguments.iter().map(|argument| argument.to_string_lossy()).collect::<Vec<_>>(),
            "environment": given_environment(),
            "pid": process_id(),
            "sid": session_id(),
            "pgid": group_id(),
            "ppid": ppid,
            "descriptors": descriptors,
            "record": String::from_utf8_lossy(&record),
            "decoded": decoded,
            "lock_taken": lock_four(),
            "mask_empty": inherited.is_some_and(|inherited| inherited.mask_empty),
            "ignored": inherited.map_or_else(|| vec![0], |inherited| {
                (1..=LAST_SIGNAL).filter(|signal| inherited.ignored[*signal as usize]).collect()
            }),
            "open_above_four": open_above_four,
        })
    }

    #[allow(unsafe_code)]
    fn described(fd: RawFd, open: bool) -> Value {
        if !open {
            return json!({"kind": "closed"});
        }
        // SAFETY: `fd` was open before main ran and nothing in this process has closed it since;
        // the borrow lasts only for the copy.
        let copy = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) }.try_clone_to_owned();
        let Ok(metadata) = copy.map(File::from).and_then(|file| file.metadata()) else {
            return json!({"kind": "unreadable"});
        };
        let kind = metadata.file_type();
        let kind = if kind.is_char_device() {
            "char"
        } else if kind.is_fifo() {
            "fifo"
        } else if kind.is_file() {
            "file"
        } else {
            "other"
        };
        json!({"kind": kind, "dev": metadata.dev(), "ino": metadata.ino(), "rdev": metadata.rdev()})
    }

    #[allow(unsafe_code)]
    fn read_descriptor_three() -> Vec<u8> {
        // SAFETY: Descriptor 3 is the config pipe the start handed over, which nothing else in
        // this process owns, and the returned File is its only owner.
        let mut pipe = unsafe { File::from_raw_fd(3) };
        let mut record = Vec::new();
        let _ = pipe.read_to_end(&mut record);
        record
    }

    #[allow(unsafe_code)]
    fn lock_four() -> bool {
        // SAFETY: flock takes integers only.
        unsafe { libc::flock(4, libc::LOCK_EX | libc::LOCK_NB) == 0 }
    }

    #[allow(unsafe_code)]
    fn reparented_parent() -> u32 {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            // SAFETY: getppid takes no arguments.
            let parent = unsafe { libc::getppid() };
            if parent == 1 || !cfg!(target_os = "macos") || Instant::now() >= deadline {
                return u32::try_from(parent).unwrap_or_default();
            }
            thread::sleep(POLL);
        }
    }

    fn process_id() -> u32 {
        std::process::id()
    }

    fn session_id() -> u32 {
        own_sid()
    }

    fn group_id() -> u32 {
        own_pgid()
    }

    #[cfg(target_os = "linux")]
    fn open_descriptors_from(lowest: RawFd) -> Vec<RawFd> {
        let listed: Vec<RawFd> = std::fs::read_dir("/proc/self/fd")
            .expect("the descriptor list is readable")
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .collect();
        listed
            .into_iter()
            .filter(|fd| *fd >= lowest && is_open(*fd))
            .collect()
    }

    #[cfg(target_os = "macos")]
    fn open_descriptors_from(lowest: RawFd) -> Vec<RawFd> {
        let last = descriptor_table_size().max(per_process_limit());
        (lowest..last).filter(|fd| is_open(*fd)).collect()
    }

    fn write_atomically(directory: &Path, name: &str, contents: &[u8]) {
        let temporary = directory.join(format!("{name}.tmp"));
        std::fs::write(&temporary, contents).expect("the stand-in writes its file");
        std::fs::rename(&temporary, directory.join(name)).expect("the stand-in names its file");
    }

    fn wait_for_release(cell: &Path) {
        let deadline = Instant::now() + STANDIN_PATIENCE;
        while !cell.join("release").exists() {
            assert!(
                Instant::now() < deadline,
                "the stand-in was released within {STANDIN_PATIENCE:?}"
            );
            thread::sleep(POLL);
        }
    }

    #[allow(unsafe_code)]
    pub(super) fn exit(code: i32) -> ! {
        // SAFETY: _exit takes an integer. It skips the exit handlers, so a stand-in started with an
        // empty environment writes no coverage profile into the working directory.
        unsafe { libc::_exit(code) }
    }
}
