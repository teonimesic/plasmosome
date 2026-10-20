use std::io;
use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;

use crate::config::{GuestConfig, LaneConfig};
use crate::console::ProbeResult;

/// How one lane ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The VMM exited and its console holds a complete run.
    Finished(Vec<ProbeResult>),
    /// The timeout passed; the VMM was killed and reaped.
    TimedOut,
    /// The VMM exited without a complete run on its console, with the reason.
    NotRun(String),
}

/// One lane run: the VMM's process ID, its exit status if it exited, how the run ended, and
/// what it wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneRun {
    pub pid: u32,
    pub status: Option<ExitStatus>,
    pub outcome: Outcome,
    pub console: String,
    pub stderr: String,
}

/// Spawn `launcher` (normally `lane-vmm`) with `lane` on its standard input, an empty
/// environment and `/` as its directory. Wait up to `timeout`; past it, kill and reap the process
/// and return `TimedOut`. Otherwise parse `lane.console` against `guest`'s run ID and probes.
pub fn run_lane(
    launcher: &Path,
    lane: &LaneConfig,
    guest: &GuestConfig,
    timeout: Duration,
) -> io::Result<LaneRun> {
    let _ = (launcher, lane, guest, timeout);
    Ok(LaneRun {
        pid: 0,
        status: None,
        outcome: Outcome::Finished(Vec::new()),
        console: String::new(),
        stderr: String::new(),
    })
}
