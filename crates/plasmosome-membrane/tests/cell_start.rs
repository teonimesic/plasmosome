use std::fs::{self, File};
use std::io::{self, PipeWriter, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use plasmosome_backend::CellId;
use plasmosome_core::runtime::cell_id::format_cell_id;
use plasmosome_core::runtime::supervisor_config::{
    ConfigRecordRefusal, MAX_RECORD_BYTES, SupervisorConfig,
};
use plasmosome_core::{
    CellDirError, InstanceRoot, InstanceRootError, LockProbe, SupervisorLock, WriterLock,
};
use plasmosome_membrane::cell_args::CellArgs;
use plasmosome_membrane::cell_start::{CellStartRefusal, adopt_lock, read_config};
use plasmosome_membrane::runtime::path::RecipePath;
use plasmosome_membrane::runtime::recipe::{Architecture, PlatformLimits, RuntimeRecipe};
use serde_json::json;
use tempfile::TempDir;

const FLOOD_BYTES: usize = 8 << 20;
const READER_BUDGET: Duration = Duration::from_secs(30);

struct Instance {
    root: InstanceRoot,
    writer: WriterLock,
    path: PathBuf,
    _dir: TempDir,
}

impl Instance {
    fn new() -> Instance {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = fs::canonicalize(dir.path()).expect("the temporary directory resolves");
        let root = InstanceRoot::open(&path).expect("the instance root opens");
        let writer = root.lock().expect("the writer lock is free");
        Instance {
            root,
            writer,
            path,
            _dir: dir,
        }
    }

    fn cell(&self, number: u64) -> SupervisorLock {
        let cell = self
            .root
            .create_cell_dir(&self.writer, &format_cell_id(number))
            .expect("the cell directory is created");
        cell.create_supervisor_lock()
            .expect("the supervisor lock is created and held")
    }

    fn cell_path(&self, number: u64) -> PathBuf {
        self.path
            .join("cells")
            .join(format_cell_id(number).as_str())
    }

    fn retire_by_hand(&self, number: u64) -> PathBuf {
        let retired = self.path.join("retired");
        fs::create_dir_all(&retired).expect("retired/ is made");
        let moved = retired.join(format_cell_id(number).as_str());
        fs::rename(self.cell_path(number), &moved).expect("the cell is moved to retired/");
        moved
    }

    fn config(&self, number: u64) -> SupervisorConfig {
        config_for(&self.path, number)
    }

    fn probe(&self, number: u64) -> LockProbe {
        self.root
            .cell_dir(&format_cell_id(number))
            .expect("the cell directory opens")
            .probe_supervisor_lock()
            .expect("the probe runs")
    }

    fn tree(&self) -> Vec<String> {
        let mut entries = Vec::new();
        list(&self.path, &self.path, &mut entries);
        entries.sort();
        entries
    }
}

fn list(root: &Path, directory: &Path, entries: &mut Vec<String>) {
    for entry in fs::read_dir(directory).expect("the directory lists") {
        let path = entry.expect("the entry reads").path();
        let kind = fs::symlink_metadata(&path).expect("lstat").file_type();
        let name = path.strip_prefix(root).expect("under the root").display();
        entries.push(format!("{name} {kind:?}"));
        if kind.is_dir() {
            list(root, &path, entries);
        }
    }
}

fn limits() -> PlatformLimits {
    PlatformLimits::host().expect("this host reports its limits")
}

fn recipe() -> RuntimeRecipe {
    let limits = limits();
    let architecture = limits.architecture();
    let kernel_format = match architecture {
        Architecture::X86_64 => "elf",
        Architecture::Aarch64 => "raw",
    };
    let artifact =
        |path: &str, digit: char| json!({"path": path, "sha256": digit.to_string().repeat(64)});
    let value = json!({
        "version": 1,
        "vcpus": 1,
        "memory_mib": 512,
        "kernel": artifact("/opt/cell/kernel", '1'),
        "kernel_format": kernel_format,
        "initramfs": artifact("/opt/cell/initramfs", '2'),
        "root_image": artifact("/opt/cell/root.raw", '3'),
        "writable_root": "/var/cell/c1/root.raw",
        "control_path": "/var/cell/c1/control.sock",
        "data_path": "/var/cell/c1/data.sock",
        "helper": artifact("/opt/cell/krun-helper", '4'),
        "libraries": [artifact("/opt/cell/lib/libkrun.1.dylib", '5')],
        "host_policy": artifact("/opt/cell/host.sb", '6'),
        "guest_policy": artifact("/opt/cell/guest.json", '7'),
        "architecture": architecture.as_str(),
    });
    RuntimeRecipe::parse(format!("{value}\n").as_bytes(), &limits)
        .unwrap_or_else(|refusal| panic!("the test recipe is valid on this host: {refusal}"))
}

fn config_for(root: &Path, number: u64) -> SupervisorConfig {
    let root = root.to_str().expect("the root is UTF-8");
    SupervisorConfig {
        cell: format_cell_id(number),
        instance_root: RecipePath::parse(root).expect("the root is a canonical path"),
        runtime_recipe: recipe(),
        boot_deadline_ms: 60_000,
    }
}

fn sample_config(number: u64) -> SupervisorConfig {
    config_for(Path::new("/var/plasmosome"), number)
}

fn encoded(config: &SupervisorConfig) -> Vec<u8> {
    config.encode().expect("the test record encodes")
}

fn padded(config: &SupervisorConfig, body_bytes: usize) -> Vec<u8> {
    let mut record = encoded(config);
    record.pop();
    assert!(
        record.len() <= body_bytes,
        "the record fits {body_bytes} bytes"
    );
    record.resize(body_bytes, b' ');
    record.push(b'\n');
    record
}

fn argv(number: u64) -> CellArgs {
    CellArgs {
        cell: format_cell_id(number),
    }
}

fn read_piped(bytes: Vec<u8>, args: &CellArgs) -> Result<SupervisorConfig, CellStartRefusal> {
    let (reader, mut writer) = io::pipe().expect("a pipe");
    let writing = thread::spawn(move || writer.write_all(&bytes));
    let read = read_config(OwnedFd::from(reader), args, &limits());
    writing
        .join()
        .expect("the writer does not panic")
        .expect("the reader takes every byte written");
    read
}

fn refused_record(record: impl FnOnce(&Instance) -> Vec<u8>, args: &CellArgs) -> CellStartRefusal {
    let instance = Instance::new();
    let _first = instance.cell(1);
    let _second = instance.cell(2);
    let before = instance.tree();
    let refusal = read_piped(record(&instance), args).expect_err("the record is refused");
    assert_eq!(instance.tree(), before, "the instance root is unchanged");
    refusal
}

fn flood(mut writer: PipeWriter, total: usize) -> usize {
    let chunk = [b'x'; 1 << 16];
    let mut written = 0;
    while written < total {
        match writer.write(&chunk[..chunk.len().min(total - written)]) {
            Ok(bytes) => written += bytes,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => break,
            Err(error) => panic!("the flood write failed: {error}"),
        }
    }
    written
}

#[allow(unsafe_code)]
fn unread_bytes(fd: BorrowedFd<'_>) -> usize {
    let mut count: libc::c_int = 0;
    // SAFETY: FIONREAD writes one c_int through the pointer, which points at `count` and is live
    // for the call, and the BorrowedFd keeps `fd` open for the call.
    let outcome = unsafe { libc::ioctl(fd.as_raw_fd(), libc::FIONREAD, &raw mut count) };
    assert_eq!(outcome, 0, "FIONREAD: {}", io::Error::last_os_error());
    usize::try_from(count).expect("the unread count is not negative")
}

fn wait_until_read(read_end: BorrowedFd<'_>) {
    let deadline = Instant::now() + READER_BUDGET;
    while unread_bytes(read_end) > 0 {
        assert!(
            Instant::now() < deadline,
            "the reader took what was written"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

#[allow(unsafe_code)]
fn inheritable_copy(fd: BorrowedFd<'_>) -> OwnedFd {
    // SAFETY: fcntl with F_DUPFD takes integers only, and the BorrowedFd keeps `fd` open for the
    // call.
    let copy = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD, 0) };
    assert!(copy >= 0, "F_DUPFD: {}", io::Error::last_os_error());
    // SAFETY: `copy` is a new descriptor that F_DUPFD just returned, and nothing else owns it.
    unsafe { OwnedFd::from_raw_fd(copy) }
}

#[allow(unsafe_code)]
fn close_on_exec(fd: RawFd) -> bool {
    // SAFETY: fcntl with F_GETFD takes no pointer, and the caller keeps `fd` open for the call.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    assert!(flags >= 0, "F_GETFD: {}", io::Error::last_os_error());
    flags & libc::FD_CLOEXEC != 0
}

fn handed_over(lock: &SupervisorLock) -> OwnedFd {
    let copy = inheritable_copy(lock.as_fd());
    assert!(!close_on_exec(copy.as_raw_fd()), "the copy is inheritable");
    copy
}

fn refused_lock(
    instance: &Instance,
    descriptor: OwnedFd,
    config: &SupervisorConfig,
) -> CellStartRefusal {
    let before = instance.tree();
    let refusal = adopt_lock(descriptor, config).expect_err("the descriptor is refused");
    assert_eq!(instance.tree(), before, "the instance root is unchanged");
    refusal
}

#[test]
fn reads_one_record() {
    let config = sample_config(1);
    let read = read_piped(encoded(&config), &argv(1));
    assert_eq!(read.expect("the record is read"), config);
}

#[test]
fn descriptor_3_is_closed_after_a_good_record() {
    let config = sample_config(1);
    let (mut ours, theirs) = UnixStream::pair().expect("a socket pair");
    ours.write_all(&encoded(&config))
        .expect("the record is written");
    ours.shutdown(Shutdown::Write)
        .expect("the write side shuts down");
    let read = read_config(OwnedFd::from(theirs), &argv(1), &limits());
    assert_eq!(read.expect("the record is read"), config);
    ours.set_nonblocking(true)
        .expect("the socket is nonblocking");
    let mut byte = [0u8; 1];
    assert_eq!(
        ours.read(&mut byte).map_err(|error| error.kind()),
        Ok(0),
        "the reader's end is closed"
    );
}

#[test]
fn a_record_of_exactly_the_bound_before_its_newline_is_accepted() {
    let config = sample_config(1);
    let read = read_piped(padded(&config, MAX_RECORD_BYTES), &argv(1));
    assert_eq!(read.expect("the largest record is read"), config);
}

#[test]
fn a_record_one_byte_over_the_bound_is_too_large() {
    let refusal = refused_record(
        |instance| padded(&instance.config(1), MAX_RECORD_BYTES + 1),
        &argv(1),
    );
    assert!(
        matches!(
            refusal,
            CellStartRefusal::Record(ConfigRecordRefusal::TooLarge { bytes: 1_048_577 })
        ),
        "{refusal:?}"
    );
}

#[test]
fn a_record_with_no_newline_before_eof_is_refused() {
    let without_newline = |instance: &Instance| {
        let mut record = encoded(&instance.config(1));
        record.pop();
        record
    };
    let refusal = refused_record(without_newline, &argv(1));
    assert!(
        matches!(
            refusal,
            CellStartRefusal::Record(ConfigRecordRefusal::NotOneLine)
        ),
        "{refusal:?}"
    );
}

#[test]
fn nothing_before_eof_is_refused() {
    let refusal = refused_record(|_| Vec::new(), &argv(1));
    assert!(
        matches!(
            refusal,
            CellStartRefusal::Record(ConfigRecordRefusal::NotOneLine)
        ),
        "{refusal:?}"
    );
}

#[test]
fn a_second_line_after_the_record_is_refused() {
    let refusal = refused_record(|instance| encoded(&instance.config(1)).repeat(2), &argv(1));
    assert!(
        matches!(
            refusal,
            CellStartRefusal::Record(ConfigRecordRefusal::NotOneLine)
        ),
        "{refusal:?}"
    );
}

#[test]
fn bytes_written_after_the_record_has_been_read_are_refused() {
    for later in [b"x\n".as_slice(), b"x".as_slice()] {
        let instance = Instance::new();
        let _held = instance.cell(1);
        let before = instance.tree();
        let (reader, mut writer) = io::pipe().expect("a pipe");
        let watched = reader.try_clone().expect("the read end is duplicated");
        let record = encoded(&instance.config(1));
        let later = later.to_vec();
        let writing = thread::spawn(move || {
            writer.write_all(&record)?;
            wait_until_read(watched.as_fd());
            writer.write_all(&later)
        });
        let read = read_config(OwnedFd::from(reader), &argv(1), &limits());
        writing
            .join()
            .expect("the writer does not panic")
            .expect("the writer writes both parts");
        assert!(
            matches!(
                read,
                Err(CellStartRefusal::Record(ConfigRecordRefusal::NotOneLine))
            ),
            "{read:?}"
        );
        assert_eq!(instance.tree(), before, "the instance root is unchanged");
    }
}

#[test]
fn a_record_for_another_cell_is_refused() {
    let refusal = refused_record(|instance| encoded(&instance.config(2)), &argv(1));
    let CellStartRefusal::CellMismatch { argv, record } = refusal else {
        panic!("{refusal:?}");
    };
    assert_eq!(
        (argv, record),
        (CellId::from("cell-1"), CellId::from("cell-2"))
    );
}

#[test]
fn huge_input_stops_at_the_cap() {
    let instance = Instance::new();
    let _held = instance.cell(1);
    let before = instance.tree();
    let (reader, writer) = io::pipe().expect("a pipe");
    let flooding = thread::spawn(move || flood(writer, FLOOD_BYTES));
    let read = read_config(OwnedFd::from(reader), &argv(1), &limits());
    let written = flooding.join().expect("the flood does not panic");
    assert!(
        matches!(
            read,
            Err(CellStartRefusal::Record(ConfigRecordRefusal::TooLarge { bytes }))
                if bytes == MAX_RECORD_BYTES + 2
        ),
        "{read:?}"
    );
    assert!(
        written < FLOOD_BYTES,
        "the reader closed its end after {written} bytes were written"
    );
    assert_eq!(instance.tree(), before, "the instance root is unchanged");
}

#[test]
fn a_descriptor_that_cannot_be_read_is_a_read_error() {
    let instance = Instance::new();
    let directory = File::open(&instance.path).expect("the root opens as a file");
    let read = read_config(OwnedFd::from(directory), &argv(1), &limits());
    assert!(
        matches!(
            &read,
            Err(CellStartRefusal::Read(error)) if error.raw_os_error() == Some(libc::EISDIR)
        ),
        "{read:?}"
    );
}

#[test]
fn adopts_its_own_lock() {
    let instance = Instance::new();
    let controller = instance.cell(1);
    let descriptor = handed_over(&controller);
    let raw = descriptor.as_raw_fd();
    let before = instance.tree();
    let adopted = adopt_lock(descriptor, &instance.config(1)).expect("the lock is adopted");
    assert_eq!(instance.tree(), before, "the instance root is unchanged");
    assert_eq!(adopted.cell_dir().cell(), &format_cell_id(1));
    assert!(close_on_exec(raw), "descriptor {raw} is close-on-exec");
    drop(controller);
    assert!(
        matches!(instance.probe(1), LockProbe::Held),
        "the adopted descriptor holds the lock"
    );
    drop(adopted);
    assert!(
        matches!(instance.probe(1), LockProbe::Free(_)),
        "dropping the adopted lock releases it"
    );
}

#[test]
fn an_adopted_lock_stays_exclusive() {
    let instance = Instance::new();
    let controller = instance.cell(1);
    let adopted =
        adopt_lock(handed_over(&controller), &instance.config(1)).expect("the lock is adopted");
    drop(controller);
    let fresh =
        File::open(instance.cell_path(1).join("supervisor.lock")).expect("the lock file opens");
    let refusal = refused_lock(&instance, OwnedFd::from(fresh), &instance.config(1));
    assert!(
        matches!(refusal, CellStartRefusal::LockNotHeld),
        "{refusal:?}"
    );
    drop(adopted);
}

#[test]
fn refuses_another_cells_lock() {
    let instance = Instance::new();
    let _first = instance.cell(1);
    let second = instance.cell(2);
    let refusal = refused_lock(&instance, handed_over(&second), &instance.config(1));
    assert!(
        matches!(refusal, CellStartRefusal::NotThisCellsLock),
        "{refusal:?}"
    );
}

#[test]
fn refuses_an_unrelated_file() {
    let instance = Instance::new();
    let _held = instance.cell(1);
    let elsewhere = tempfile::tempdir().expect("a temporary directory");
    let path = elsewhere.path().join("unrelated");
    fs::write(&path, b"").expect("the unrelated file is written");
    let file = File::open(&path).expect("the unrelated file opens");
    let refusal = refused_lock(&instance, OwnedFd::from(file), &instance.config(1));
    assert!(
        matches!(refusal, CellStartRefusal::NotThisCellsLock),
        "{refusal:?}"
    );
}

#[test]
fn refuses_this_lock_opened_without_the_lock() {
    let instance = Instance::new();
    let _held = instance.cell(1);
    let fresh =
        File::open(instance.cell_path(1).join("supervisor.lock")).expect("the lock file opens");
    let refusal = refused_lock(&instance, OwnedFd::from(fresh), &instance.config(1));
    assert!(
        matches!(refusal, CellStartRefusal::LockNotHeld),
        "{refusal:?}"
    );
}

#[test]
fn refuses_a_retired_directory() {
    let instance = Instance::new();
    let controller = instance.cell(1);
    let retired = instance.retire_by_hand(1);
    let refusal = refused_lock(&instance, handed_over(&controller), &instance.config(1));
    let CellStartRefusal::CellDirectory(CellDirError::Missing { path }) = &refusal else {
        panic!("{refusal:?}");
    };
    assert_eq!(path.as_path(), instance.cell_path(1));
    assert!(
        retired.join("supervisor.lock").is_file(),
        "the lock moved with the cell"
    );
}

#[test]
fn refuses_a_cell_directory_reached_through_a_symlink() {
    let instance = Instance::new();
    let controller = instance.cell(1);
    let retired = instance.retire_by_hand(1);
    std::os::unix::fs::symlink(&retired, instance.cell_path(1)).expect("the symlink is made");
    let refusal = refused_lock(&instance, handed_over(&controller), &instance.config(1));
    assert!(
        matches!(
            refusal,
            CellStartRefusal::CellDirectory(CellDirError::Symlink { .. })
        ),
        "{refusal:?}"
    );
}

#[test]
fn refuses_another_roots_cell_reached_through_a_symlink() {
    let instance = Instance::new();
    let _other = instance.cell(2);
    let elsewhere = Instance::new();
    let controller = elsewhere.cell(1);
    std::os::unix::fs::symlink(elsewhere.cell_path(1), instance.cell_path(1))
        .expect("the symlink is made");
    let refusal = refused_lock(&instance, handed_over(&controller), &instance.config(1));
    assert!(
        matches!(
            refusal,
            CellStartRefusal::CellDirectory(CellDirError::Symlink { .. })
        ),
        "{refusal:?}"
    );
}

#[test]
fn refuses_a_lock_file_that_is_a_symlink_to_another_roots_lock() {
    let instance = Instance::new();
    let _own = instance.cell(1);
    let elsewhere = Instance::new();
    let controller = elsewhere.cell(1);
    let lock_path = instance.cell_path(1).join("supervisor.lock");
    fs::remove_file(&lock_path).expect("the lock file is unlinked");
    std::os::unix::fs::symlink(elsewhere.cell_path(1).join("supervisor.lock"), &lock_path)
        .expect("the symlink is made");
    let refusal = refused_lock(&instance, handed_over(&controller), &instance.config(1));
    let CellStartRefusal::CellDirectory(CellDirError::Symlink { path }) = &refusal else {
        panic!("{refusal:?}");
    };
    assert_eq!(path.as_path(), lock_path);
}

#[test]
fn refuses_a_cell_whose_lock_file_is_gone() {
    let instance = Instance::new();
    let controller = instance.cell(1);
    let lock_path = instance.cell_path(1).join("supervisor.lock");
    fs::remove_file(&lock_path).expect("the lock file is unlinked");
    let refusal = refused_lock(&instance, handed_over(&controller), &instance.config(1));
    let CellStartRefusal::CellDirectory(CellDirError::Missing { path }) = &refusal else {
        panic!("{refusal:?}");
    };
    assert_eq!(path.as_path(), lock_path);
}

#[test]
fn refuses_an_instance_root_that_does_not_open() {
    let instance = Instance::new();
    let controller = instance.cell(1);
    let config = config_for(&instance.path.join("absent"), 1);
    let refusal = refused_lock(&instance, handed_over(&controller), &config);
    assert!(
        matches!(
            &refusal,
            CellStartRefusal::InstanceRoot(InstanceRootError::Open { source, .. })
                if source.raw_os_error() == Some(libc::ENOENT)
        ),
        "{refusal:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_descriptor_flock_cannot_lock_is_a_lock_error() {
    let instance = Instance::new();
    let _held = instance.cell(1);
    let (reader, _writer) = io::pipe().expect("a pipe");
    let refusal = refused_lock(&instance, OwnedFd::from(reader), &instance.config(1));
    assert!(
        matches!(
            &refusal,
            CellStartRefusal::Lock(error) if error.raw_os_error() == Some(libc::ENOTSUP)
        ),
        "{refusal:?}"
    );
}

#[test]
fn refusals_describe_themselves_on_one_line() {
    let refusals = [
        CellStartRefusal::Read(io::Error::from_raw_os_error(libc::EISDIR)),
        CellStartRefusal::Record(ConfigRecordRefusal::NotOneLine),
        CellStartRefusal::CellMismatch {
            argv: format_cell_id(1),
            record: format_cell_id(2),
        },
        CellStartRefusal::LockNotHeld,
        CellStartRefusal::Lock(io::Error::from_raw_os_error(libc::ENOTSUP)),
        CellStartRefusal::InstanceRoot(InstanceRootError::NotAbsolute {
            path: PathBuf::from("var/plasmosome"),
        }),
        CellStartRefusal::CellDirectory(CellDirError::Missing {
            path: PathBuf::from("/var/plasmosome/cells/cell-1"),
        }),
        CellStartRefusal::NotThisCellsLock,
    ];
    let mut shown: Vec<String> = refusals.iter().map(ToString::to_string).collect();
    for text in &shown {
        assert!(!text.is_empty() && !text.contains(['\n', '\r']), "{text:?}");
    }
    assert!(
        shown[2].contains("cell-1") && shown[2].contains("cell-2"),
        "{}",
        shown[2]
    );
    for (refusal, text) in refusals.iter().zip(&shown) {
        let inner = match refusal {
            CellStartRefusal::Read(error) | CellStartRefusal::Lock(error) => error.to_string(),
            CellStartRefusal::Record(refusal) => refusal.to_string(),
            CellStartRefusal::InstanceRoot(error) => error.to_string(),
            CellStartRefusal::CellDirectory(error) => error.to_string(),
            CellStartRefusal::CellMismatch { .. }
            | CellStartRefusal::LockNotHeld
            | CellStartRefusal::NotThisCellsLock => String::new(),
        };
        assert!(text.contains(&inner), "{text:?} carries {inner:?}");
    }
    shown.sort();
    shown.dedup();
    assert_eq!(
        shown.len(),
        refusals.len(),
        "each refusal says something different"
    );
}
