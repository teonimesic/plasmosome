use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::{GuestConfig, LaneConfig};
use crate::console::{ProbeResult, RESULT};
use crate::run::{LaneRun, MAX_CONSOLE_BYTES, run_lane};

const POLL: Duration = Duration::from_millis(5);

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

struct Watch {
    console: PathBuf,
    start: Instant,
    pending: VecDeque<Step>,
    records: Vec<StepRecord>,
    seen: Vec<(String, u64)>,
}

fn millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn complete_result_probes(text: &str) -> Vec<String> {
    let prefix = format!("{RESULT} ");
    let complete = match text.rfind('\n') {
        Some(end) => &text[..end],
        None => return Vec::new(),
    };
    complete
        .lines()
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix(&prefix))
        .filter_map(|json| serde_json::from_str::<ProbeResult>(json).ok())
        .map(|result| result.probe)
        .collect()
}

impl Watch {
    fn observe(&mut self, take: bool) {
        let Ok(bytes) = fs::read(&self.console) else {
            return;
        };
        let length = bytes.len().min(MAX_CONSOLE_BYTES as usize);
        let text = String::from_utf8_lossy(&bytes[..length]);
        for probe in complete_result_probes(&text)
            .into_iter()
            .skip(self.seen.len())
        {
            self.seen.push((probe, millis(self.start)));
        }
        while let Some(step) = self.pending.front().filter(|_| take) {
            if !self.seen.iter().any(|(probe, _)| *probe == step.after) {
                return;
            }
            let Some(step) = self.pending.pop_front() else {
                return;
            };
            self.take(step);
        }
    }

    fn take(&mut self, step: Step) {
        let started_ms = millis(self.start);
        let outcome = (step.action)().map_err(|error| error.to_string());
        let outcome = match fs::write(&step.flag, b"go\n") {
            Ok(()) => outcome,
            Err(error) => Err(format!(
                "the flag {} could not be written: {error}",
                step.flag.display()
            )),
        };
        self.records.push(StepRecord {
            after: step.after,
            outcome,
            started_ms,
            flagged_ms: millis(self.start),
        });
    }
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
    match fs::remove_file(&lane.console) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let done = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&done);
    let mut watch = Watch {
        console: lane.console.clone(),
        start: Instant::now(),
        pending: steps.into(),
        records: Vec::new(),
        seen: Vec::new(),
    };
    let watcher = thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            watch.observe(true);
            thread::sleep(POLL);
        }
        watch.observe(false);
        watch
    });
    let run = run_lane(launcher, lane, guest, timeout);
    done.store(true, Ordering::SeqCst);
    let watch = watcher
        .join()
        .map_err(|_| io::Error::other("the console watch panicked"))?;
    Ok(StepsRun {
        run: run?,
        steps: watch.records,
        seen: watch.seen,
    })
}
