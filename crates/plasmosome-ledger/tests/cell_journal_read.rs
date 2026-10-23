mod support;

use std::fs::{self, File};
use std::os::unix::{ffi::OsStringExt, fs::symlink};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use plasmosome_backend::CellId;
use plasmosome_ledger::cell_journal::{
    CellJournalFaultKind as Kind, CellJournalRecord, JournalEvent as Event, encode_record,
    read_cell_journal, read_cell_journal_from,
};
use support::*;

fn bytes(records: &[CellJournalRecord]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|r| encode_record(r).unwrap())
        .collect()
}
fn legal() -> Vec<CellJournalRecord> {
    vec![
        prepare(1, vec![attach("p", vec![exact_uds("c", "p")])]),
        record(1, Event::Commit),
        record(1, Event::Finish),
    ]
}
fn path(root: &Path) -> std::path::PathBuf {
    let dir = root.join("cells/c");
    fs::create_dir_all(&dir).unwrap();
    dir.join("ledger.ndjson")
}

#[test]
fn missing_empty_and_legal_journals() {
    let temp = tempfile::tempdir().unwrap();
    let path = path(temp.path());
    assert!(read_cell_journal(&path).unwrap().is_empty());
    fs::write(&path, b"").unwrap();
    assert!(read_cell_journal(&path).unwrap().is_empty());
    let records = legal();
    for end in 1..=records.len() {
        fs::write(&path, bytes(&records[..end])).unwrap();
        assert_eq!(read_cell_journal(&path).unwrap(), records[..end]);
    }
    assert_eq!(
        read_cell_journal_from(
            File::open(&path).unwrap(),
            Path::new("diagnostic-only"),
            &CellId::from("c")
        )
        .unwrap(),
        records
    );
}

#[test]
fn illegal_history_faults_at_its_line_and_preserves_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = path(temp.path());
    let mut records = legal();
    records.push(record(1, Event::Commit));
    let original = bytes(&records);
    fs::write(&path, &original).unwrap();
    let fault = read_cell_journal(&path).unwrap_err();
    assert_eq!(
        (&fault.path, fault.kind, fault.line, fault.lines_parsed),
        (&path, Kind::EventOrder, Some(4), 3)
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    let original = bytes(&[prepare(1, vec![attach("p", vec![exact_uds("other", "p")])])]);
    fs::write(&path, &original).unwrap();
    let fault = read_cell_journal(&path).unwrap_err();
    assert_eq!(
        (fault.kind, fault.line, fault.lines_parsed),
        (Kind::InvalidEffect, Some(1), 0)
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(
        read_cell_journal_from(
            File::open(&path).unwrap(),
            Path::new("irrelevant"),
            &CellId::from("other")
        )
        .is_ok()
    );
}

#[test]
fn repeated_abort_returns_fault_without_records() {
    let records = vec![
        prepare(1, vec![attach("p", vec![])]),
        record(1, Event::Abort),
        record(1, Event::Abort),
    ];
    let path = Path::new("diagnostic-only");
    let result = read_cell_journal_from(&bytes(&records)[..], path, &CellId::from("c"));
    assert!(result.is_err(), "illegal history returned records");
    let fault = result.unwrap_err();
    assert_eq!(
        (
            fault.path.as_path(),
            fault.kind,
            fault.line,
            fault.lines_parsed
        ),
        (path, Kind::EventOrder, Some(3), 2)
    );
}

#[test]
fn older_commit_returns_fault_without_records() {
    let records = vec![
        prepare(1, vec![attach("p", vec![])]),
        record(1, Event::Commit),
        record(1, Event::Finish),
        prepare(2, vec![attach("p", vec![])]),
        record(1, Event::Commit),
    ];
    let path = Path::new("diagnostic-only");
    let result = read_cell_journal_from(&bytes(&records)[..], path, &CellId::from("c"));
    assert!(result.is_err(), "illegal history returned records");
    let fault = result.unwrap_err();
    assert_eq!(
        (
            fault.path.as_path(),
            fault.kind,
            fault.line,
            fault.lines_parsed
        ),
        (path, Kind::GenerationOrder, Some(5), 4)
    );
}

#[test]
fn single_plugin_log_at_cell_path_faults() {
    let temp = tempfile::tempdir().unwrap();
    let path = path(temp.path());
    let mut ledger = plasmosome_ledger::Ledger::new(plasmosome_backend::PluginId::from("p"));
    ledger.push(plasmosome_ledger::Effect::external("sent"));
    ledger.write_to(&mut File::create(&path).unwrap()).unwrap();
    let original = fs::read(&path).unwrap();
    let fault = read_cell_journal(&path).unwrap_err();
    assert_eq!(
        (fault.kind, fault.line, fault.lines_parsed),
        (Kind::UnsupportedFormat, Some(1), 0)
    );
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn symlinked_journal_is_an_io_fault() {
    let temp = tempfile::tempdir().unwrap();
    let path = path(temp.path());
    let target = temp.path().join("target");
    let original = bytes(&legal());
    fs::write(&target, &original).unwrap();
    symlink(&target, &path).unwrap();
    let fault = read_cell_journal(&path).unwrap_err();
    assert_eq!(
        (fault.kind, fault.line, fault.lines_parsed),
        (Kind::Io, None, 0)
    );
    assert_eq!(fs::read(&target).unwrap(), original);
    fs::remove_file(&path).unwrap();
    symlink(temp.path().join("absent"), &path).unwrap();
    assert_eq!(read_cell_journal(&path).unwrap_err().kind, Kind::Io);
}

#[test]
fn directory_at_journal_path_is_an_io_fault() {
    let temp = tempfile::tempdir().unwrap();
    let path = path(temp.path());
    fs::create_dir(&path).unwrap();
    let fault = read_cell_journal(&path).unwrap_err();
    assert_eq!(
        (fault.kind, fault.line, fault.lines_parsed),
        (Kind::Io, None, 0)
    );
    let parent = temp.path().join("regular-parent");
    fs::write(&parent, b"not a directory").unwrap();
    let fault = read_cell_journal(&parent.join("ledger.ndjson")).unwrap_err();
    assert_eq!(
        (fault.kind, fault.line, fault.lines_parsed),
        (Kind::Io, None, 0)
    );
}

#[test]
fn fifo_at_journal_path_faults_without_blocking() {
    let temp = tempfile::tempdir().unwrap();
    let path = path(temp.path());
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let (send, receive) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        send.send(read_cell_journal(&path)).unwrap();
    });
    let fault = receive
        .recv_timeout(Duration::from_secs(5))
        .expect("FIFO open blocked")
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(
        (fault.kind, fault.line, fault.lines_parsed),
        (Kind::Io, None, 0)
    );
}

#[test]
fn unusable_parent_names_are_open_faults() {
    let temp = tempfile::tempdir().unwrap();
    let non_utf8 = temp
        .path()
        .join(std::ffi::OsString::from_vec(vec![0xff]))
        .join("ledger.ndjson");
    for path in [
        Path::new("ledger.ndjson"),
        Path::new("/ledger.ndjson"),
        &non_utf8,
    ] {
        let fault = read_cell_journal(path).unwrap_err();
        assert_eq!(
            (fault.kind, fault.line, fault.lines_parsed),
            (Kind::Io, None, 0)
        );
    }
}
