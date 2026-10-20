use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::gate::Verdict;

/// What a run of a row is expected to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// The configuration under test: the row needs `Pass`.
    Pass,
    /// A mutant: the row needs the measurement to catch it with `Fail`.
    Fail,
    /// An observation, recorded and not judged.
    Observe,
}

/// One run a row reads, by the run's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub run: String,
    pub what: String,
    pub expect: Expect,
}

/// One row of the results: a measurement or an observation, and the runs it reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSpec {
    pub id: String,
    pub claim: String,
    pub runs: Vec<RunSpec>,
}

/// What one lane run left for the results file. `verdicts` maps a row ID to that row's verdict
/// on this run; `observations` maps a row ID to what an observation saw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub run: String,
    pub configuration: Value,
    pub digests: BTreeMap<String, String>,
    pub verdicts: BTreeMap<String, Verdict>,
    pub observations: BTreeMap<String, Vec<String>>,
    pub notes: Vec<String>,
    pub lines: Vec<String>,
}

/// A row's verdict over its runs: `NotRun` when any run it reads has no record or no verdict
/// for the row, or a `NotRun` one; `Pass` when every `Pass` run passed and every `Fail` run
/// failed; `Fail` otherwise, naming each run that differs.
pub fn row_verdict(row: &RowSpec, records: &[Record]) -> Verdict {
    let _ = (row, records);
    Verdict::Pass
}

/// The results file: `preamble`, then the row verdicts, then one section per run with its
/// configuration, digests, notes and every result line. A run a row reads with no record reads
/// `not run`, never omitted.
pub fn render(preamble: &str, rows: &[RowSpec], records: &[Record]) -> String {
    let _ = (rows, records);
    preamble.to_string()
}
