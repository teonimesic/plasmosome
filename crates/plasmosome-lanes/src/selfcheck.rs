use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;

use crate::probe::errno_name;

const PROBE_FILE: &str = ".lane-selfcheck";

const NAMES: [&str; 4] = [
    "read inside",
    "write inside",
    "read outside",
    "write outside",
];

const CONFINED: [bool; 4] = [true, false, false, false];

const UNCONFINED: [bool; 4] = [true, true, true, true];

fn expected(phase: Phase) -> [bool; 4] {
    match phase {
        Phase::Unconfined => UNCONFINED,
        Phase::Confined => CONFINED,
    }
}

/// Which side of `sandbox_init` the four checks run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Before the profile is applied, as a control.
    Unconfined,
    /// After the profile is applied.
    Confined,
}

/// One of the four confinement checks and what the process observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub expected_allowed: bool,
    pub observed: Result<(), String>,
}

impl Check {
    /// True when the observation is the expected one. A refusal counts only as `EPERM`, which
    /// is what the sandbox returns; `EACCES` from file modes and `ENOENT` are not refusals.
    pub fn holds(&self) -> bool {
        match (&self.observed, self.expected_allowed) {
            (Ok(()), allowed) => allowed,
            (Err(errno), false) => errno == "EPERM",
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

/// Read inside `allowed`, write inside it, read `outside` and write inside `outside`. Unconfined,
/// all four are expected to succeed, which shows the directories refuse nothing by themselves.
/// Confined, only the first is. A write that succeeds is removed again.
pub fn self_check(phase: Phase, allowed: &Path, outside: &Path) -> Vec<Check> {
    let results = [read(allowed), write(allowed), read(outside), write(outside)];
    NAMES
        .into_iter()
        .zip(expected(phase))
        .zip(results)
        .map(|((name, expected_allowed), result)| Check {
            name,
            expected_allowed,
            observed: observed(result),
        })
        .collect()
}

/// True when all four checks ran in order with `phase`'s expectations and each holds.
pub fn passed(phase: Phase, checks: &[Check]) -> bool {
    checks.len() == NAMES.len()
        && checks
            .iter()
            .zip(NAMES.into_iter().zip(expected(phase)))
            .all(|(check, (name, allowed))| {
                check.name == name && check.expected_allowed == allowed && check.holds()
            })
}
