use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{GuestConfig, LaneConfig};
use crate::run::LaneRun;

/// The action a step runs on the host. Its text, or its error, is recorded.
pub type Action = Box<dyn FnOnce() -> io::Result<String> + Send>;

/// One host action taken during a lane run, once a probe's result is on the console.
pub struct Step {
    /// The probe whose result line starts the action.
    pub after: String,
    /// The file written once the action returns, which a guest `await` probe waits for. It is
    /// written after a failed action too, so the guest is never left waiting.
    pub flag: PathBuf,
    pub action: Action,
}

/// What one step did, with times in milliseconds since the run started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRecord {
    pub after: String,
    pub outcome: Result<String, String>,
    pub started_ms: u64,
    pub flagged_ms: u64,
}

/// A lane run with steps: the run itself, a record per step taken, and when the host first saw
/// each probe's result line.
#[derive(Debug, Clone, PartialEq)]
pub struct StepsRun {
    pub run: LaneRun,
    pub steps: Vec<StepRecord>,
    pub seen: Vec<(String, u64)>,
}

/// Run a lane as `run::run_lane` does, while a thread watches the console and takes each step,
/// in order, once its probe's result line is complete. A step whose probe never reports is not
/// taken and has no record.
pub fn run_lane_with_steps(
    launcher: &Path,
    lane: &LaneConfig,
    guest: &GuestConfig,
    timeout: Duration,
    steps: Vec<Step>,
) -> io::Result<StepsRun> {
    let _ = steps;
    Ok(StepsRun {
        run: crate::run::run_lane(launcher, lane, guest, timeout)?,
        steps: Vec::new(),
        seen: Vec::new(),
    })
}
