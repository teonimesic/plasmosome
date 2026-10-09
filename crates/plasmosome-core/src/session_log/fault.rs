use super::{LogFile, LogStore, OsLogStore};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    CreateDir,
    SyncDir,
    Open,
    Write,
    Flush,
    SyncFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    CreateDir(PathBuf),
    SyncDir(PathBuf),
    Open(PathBuf),
    Write(usize),
    Flush,
    SyncFile,
}

impl Call {
    fn step(&self) -> Step {
        match self {
            Call::CreateDir(_) => Step::CreateDir,
            Call::SyncDir(_) => Step::SyncDir,
            Call::Open(_) => Step::Open,
            Call::Write(_) => Step::Write,
            Call::Flush => Step::Flush,
            Call::SyncFile => Step::SyncFile,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Fault {
    Fail,
    Tear(usize),
    Panic,
}

#[derive(Default)]
struct Record {
    calls: Vec<Call>,
    planned: Vec<(Step, usize, Fault)>,
}

impl Record {
    fn count(&self, step: Step) -> usize {
        self.calls.iter().filter(|call| call.step() == step).count()
    }

    fn enter(&mut self, call: Call) -> Option<Fault> {
        let step = call.step();
        self.calls.push(call);
        let nth = self.count(step);
        self.planned
            .iter()
            .find(|(planned, at, _)| *planned == step && *at == nth)
            .map(|(_, _, fault)| *fault)
    }
}

fn enter(record: &Mutex<Record>, call: Call) -> Option<Fault> {
    record.lock().expect("fault record lock").enter(call)
}

fn injected() -> std::io::Error {
    std::io::Error::other("injected session log fault")
}

fn strike(fault: Option<Fault>) -> std::io::Result<()> {
    match fault {
        None => Ok(()),
        Some(Fault::Panic) => panic!("injected panic inside a session log call"),
        Some(Fault::Fail | Fault::Tear(_)) => Err(injected()),
    }
}

pub(crate) struct FaultLogStore {
    dir: tempfile::TempDir,
    record: Arc<Mutex<Record>>,
}

impl FaultLogStore {
    pub(crate) fn new() -> FaultLogStore {
        FaultLogStore {
            dir: tempfile::tempdir().expect("fault store temp dir"),
            record: Arc::default(),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        self.dir.path()
    }

    pub(crate) fn fail(self, step: Step, nth: usize) -> FaultLogStore {
        self.plan(step, nth, Fault::Fail)
    }

    pub(crate) fn tear(self, nth: usize, kept: usize) -> FaultLogStore {
        self.plan(Step::Write, nth, Fault::Tear(kept))
    }

    pub(crate) fn panic_on_write(self, nth: usize) -> FaultLogStore {
        self.plan(Step::Write, nth, Fault::Panic)
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.record.lock().expect("fault record lock").calls.clone()
    }

    pub(crate) fn count(&self, step: Step) -> usize {
        self.record.lock().expect("fault record lock").count(step)
    }

    fn plan(self, step: Step, nth: usize, fault: Fault) -> FaultLogStore {
        self.record
            .lock()
            .expect("fault record lock")
            .planned
            .push((step, nth, fault));
        self
    }
}

impl LogStore for FaultLogStore {
    fn create_dir(&self, path: &Path) -> std::io::Result<()> {
        strike(enter(&self.record, Call::CreateDir(path.to_path_buf())))?;
        OsLogStore.create_dir(path)
    }

    fn sync_dir(&self, path: &Path) -> std::io::Result<()> {
        strike(enter(&self.record, Call::SyncDir(path.to_path_buf())))?;
        OsLogStore.sync_dir(path)
    }

    fn open_log(&self, path: &Path) -> std::io::Result<Box<dyn LogFile>> {
        strike(enter(&self.record, Call::Open(path.to_path_buf())))?;
        Ok(Box::new(FaultLogFile {
            inner: OsLogStore.open_log(path)?,
            record: Arc::clone(&self.record),
        }))
    }
}

struct FaultLogFile {
    inner: Box<dyn LogFile>,
    record: Arc<Mutex<Record>>,
}

impl std::io::Read for FaultLogFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl LogFile for FaultLogFile {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match enter(&self.record, Call::Write(bytes.len())) {
            None => self.inner.write_all(bytes),
            Some(Fault::Tear(kept)) => {
                self.inner.write_all(&bytes[..kept.min(bytes.len())])?;
                Err(injected())
            }
            fault => strike(fault),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        strike(enter(&self.record, Call::Flush))?;
        self.inner.flush()
    }

    fn sync_all(&mut self) -> std::io::Result<()> {
        strike(enter(&self.record, Call::SyncFile))?;
        self.inner.sync_all()
    }
}
