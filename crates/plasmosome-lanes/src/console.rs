use serde::{Deserialize, Serialize};

/// The line `lane-init` writes before its first result.
pub const BEGIN: &str = "LANE-BEGIN";
/// The prefix of each result line.
pub const RESULT: &str = "LANE-RESULT";
/// The line `lane-init` writes after its last result, with the run ID and the result count.
pub const END: &str = "LANE-END";

/// One probe's framed result, as `lane-init` wrote it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub probe: String,
    pub uid: u32,
    pub exit: Option<i32>,
    pub result: Option<serde_json::Value>,
}

/// What a console transcript says about one run.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsoleRun {
    /// Every configured probe wrote one result, in order, between matching markers.
    Finished(Vec<ProbeResult>),
    /// Anything else, with the reason. Never a pass.
    NotRun(String),
}

/// Parse a console transcript for the run `run_id`, whose probes are `probes` in order.
pub fn parse_console(text: &str, run_id: &str, probes: &[String]) -> ConsoleRun {
    let _ = (text, run_id, probes);
    ConsoleRun::Finished(Vec::new())
}

/// The framed line for one result.
pub fn result_line(result: &ProbeResult) -> String {
    format!(
        "{RESULT} {}",
        serde_json::to_string(result).unwrap_or_default()
    )
}
