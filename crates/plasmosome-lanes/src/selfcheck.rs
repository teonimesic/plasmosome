use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;

use crate::probe::errno_name;

const PROBE_FILE: &str = ".lane-selfcheck";

const EXPECTED: [(&str, bool); 4] = [
    ("read inside", true),
    ("write inside", false),
    ("read outside", false),
    ("write outside", false),
];

/// One of the four confinement checks and what the process observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub expected_allowed: bool,
    pub observed: Result<(), String>,
}

impl Check {
    /// True when the observation is the expected one. A refusal counts only as `EPERM` or
    /// `EACCES`; a missing directory is not a refusal.
    pub fn holds(&self) -> bool {
        match (&self.observed, self.expected_allowed) {
            (Ok(()), allowed) => allowed,
            (Err(errno), false) => errno == "EPERM" || errno == "EACCES",
            (Err(_), true) => false,
        }
    }
}

fn observed(result: io::Result<()>) -> Result<(), String> {
    result.map_err(|error| match error.raw_os_error() {
        Some(code) => errno_name(code),
        None => error.to_string(),
    })
}

fn read(dir: &Path) -> io::Result<()> {
    fs::read_dir(dir)?.try_for_each(|entry| entry.map(drop))
}

fn write(dir: &Path) -> io::Result<()> {
    let path = dir.join(PROBE_FILE);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let _ = fs::remove_file(path);
    Ok(())
}

/// Read inside `allowed` (expected to succeed), then write inside it, read `outside` and write
/// inside `outside` (each expected to be refused). A write that succeeds is removed again.
pub fn self_check(allowed: &Path, outside: &Path) -> Vec<Check> {
    let results = [read(allowed), write(allowed), read(outside), write(outside)];
    EXPECTED
        .into_iter()
        .zip(results)
        .map(|((name, expected_allowed), result)| Check {
            name,
            expected_allowed,
            observed: observed(result),
        })
        .collect()
}

/// True when every check holds and all four ran.
pub fn passed(checks: &[Check]) -> bool {
    checks.len() == EXPECTED.len()
        && checks.iter().zip(EXPECTED).all(|(check, (name, allowed))| {
            check.name == name && check.expected_allowed == allowed && check.holds()
        })
}
