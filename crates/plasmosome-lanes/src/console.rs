use serde::{Deserialize, Serialize};

/// The line `lane-init` writes before its first result.
pub const BEGIN: &str = "LANE-BEGIN";
/// The prefix of each result line.
pub const RESULT: &str = "LANE-RESULT";
/// The line `lane-init` writes after its last result, with the run ID and the result count.
pub const END: &str = "LANE-END";

/// One probe's framed result, as `lane-init` wrote it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

fn framed(text: &str, run_id: &str) -> Result<Vec<ProbeResult>, String> {
    let mut lines = text.lines().map(|line| line.trim_end_matches('\r'));
    let begin = format!("{BEGIN} ");
    let opening = lines
        .by_ref()
        .find(|line| line.starts_with(&begin))
        .ok_or_else(|| format!("the console has no {BEGIN} line"))?;
    let opened_by = &opening[begin.len()..];
    if opened_by != run_id {
        return Err(format!(
            "{BEGIN} names run `{opened_by}`, not the configured `{run_id}`"
        ));
    }
    let end = format!("{END} ");
    let result = format!("{RESULT} ");
    let mut results = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix(&end) {
            let expected = format!("{run_id} {}", results.len());
            if rest == expected {
                return Ok(results);
            }
            return Err(format!(
                "{END} says `{rest}`, but this is run `{run_id}` with {} result lines (count)",
                results.len()
            ));
        }
        let json = line
            .strip_prefix(&result)
            .ok_or_else(|| format!("a line between the markers is not a result: `{line}`"))?;
        let parsed: ProbeResult = serde_json::from_str(json)
            .map_err(|error| format!("a {RESULT} line does not parse ({error}): `{line}`"))?;
        results.push(parsed);
    }
    Err(format!("the console has no {END} line"))
}

/// Parse a console transcript for the run `run_id`, whose probes are `probes` in order.
pub fn parse_console(text: &str, run_id: &str, probes: &[String]) -> ConsoleRun {
    let results = match framed(text, run_id) {
        Ok(results) => results,
        Err(reason) => return ConsoleRun::NotRun(reason),
    };
    for (index, probe) in probes.iter().enumerate() {
        match results.get(index) {
            None => {
                return ConsoleRun::NotRun(format!("probe `{probe}` wrote no result line"));
            }
            Some(found) if &found.probe != probe => {
                return ConsoleRun::NotRun(format!(
                    "result {} is for `{}`, but the configured probe there is `{probe}`",
                    index + 1,
                    found.probe
                ));
            }
            Some(found) if found.result.is_none() => {
                return ConsoleRun::NotRun(format!("probe `{probe}` wrote no result line"));
            }
            Some(_) => {}
        }
    }
    if let Some(extra) = results.get(probes.len()) {
        return ConsoleRun::NotRun(format!(
            "a result for `{}` is not for a configured probe",
            extra.probe
        ));
    }
    ConsoleRun::Finished(results)
}

/// The framed line for one result.
pub fn result_line(result: &ProbeResult) -> String {
    format!(
        "{RESULT} {}",
        serde_json::to_string(result).unwrap_or_default()
    )
}
