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

fn judged(row: &RowSpec) -> bool {
    row.runs.iter().any(|run| run.expect != Expect::Observe)
}

fn observed(record: &Record, row: &str) -> bool {
    record
        .observations
        .get(row)
        .is_some_and(|seen| !seen.is_empty())
}

/// A row's verdict over its runs: `NotRun` when any run it reads has no record, when a judged
/// run has no verdict for the row, or when one is `NotRun`, a mutant's included; `Pass` when
/// every `Pass` run passed and every `Fail` run failed; `Fail` otherwise, naming each run that
/// differs. An `Observe` run never changes a judged row's verdict once its record exists. A row
/// of observations only is `NotRun` when a run recorded no observation for it.
pub fn row_verdict(row: &RowSpec, records: &[Record]) -> Verdict {
    let judged = judged(row);
    let mut differ = Vec::new();
    for spec in &row.runs {
        let Some(record) = records.iter().find(|record| record.run == spec.run) else {
            return Verdict::NotRun(format!("run `{}` has no record", spec.run));
        };
        if spec.expect == Expect::Observe {
            if !judged && !observed(record, &row.id) {
                return Verdict::NotRun(format!(
                    "run `{}` recorded no observation for {}",
                    spec.run, row.id
                ));
            }
            continue;
        }
        let Some(verdict) = record.verdicts.get(&row.id) else {
            return Verdict::NotRun(format!("run `{}` has no verdict for {}", spec.run, row.id));
        };
        match (spec.expect, verdict) {
            (_, Verdict::NotRun(why)) => {
                return Verdict::NotRun(format!("run `{}`: {why}", spec.run));
            }
            (Expect::Pass, Verdict::Fail(why)) => {
                differ.push(format!("`{}` failed: {why}", spec.run));
            }
            (Expect::Fail, Verdict::Pass) => {
                differ.push(format!("the mutant `{}` passed", spec.run));
            }
            _ => {}
        }
    }
    if differ.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Fail(differ.join("; "))
    }
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn verdict_text(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Pass => "pass".to_string(),
        Verdict::Fail(why) => format!("fail: {why}"),
        Verdict::NotRun(why) => format!("not run: {why}"),
    }
}

fn row_text(row: &RowSpec, verdict: &Verdict) -> String {
    let judged = judged(row);
    match verdict {
        Verdict::Pass if !judged => "**recorded**".to_string(),
        Verdict::NotRun(why) if !judged => format!("**not recorded**: {why}"),
        Verdict::Pass => "**pass**".to_string(),
        Verdict::Fail(why) => format!("**fail**: {why}"),
        Verdict::NotRun(why) => format!("**not run**: {why}"),
    }
}

fn expect_text(expect: Expect) -> &'static str {
    match expect {
        Expect::Pass => "pass",
        Expect::Fail => "fail (the mutant is caught)",
        Expect::Observe => "recorded",
    }
}

fn observed_text(row: &RowSpec, spec: &RunSpec, records: &[Record]) -> String {
    let Some(record) = records.iter().find(|record| record.run == spec.run) else {
        return "not run: no record".to_string();
    };
    match record.verdicts.get(&row.id) {
        Some(verdict) => verdict_text(verdict),
        None if spec.expect == Expect::Observe && observed(record, &row.id) => {
            "recorded".to_string()
        }
        None if spec.expect == Expect::Observe => "not recorded".to_string(),
        None => "not run: no verdict".to_string(),
    }
}

fn run_names(rows: &[RowSpec], records: &[Record]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mentioned = rows
        .iter()
        .flat_map(|row| row.runs.iter().map(|run| run.run.clone()));
    for name in mentioned.chain(records.iter().map(|record| record.run.clone())) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

fn section(name: &str, records: &[Record]) -> String {
    let Some(record) = records.iter().find(|record| record.run == name) else {
        return format!("### {name}\n\nNot run: this run left no record.\n\n");
    };
    let mut text = format!("### {name}\n\n**Configuration**\n\n```json\n");
    text.push_str(
        &serde_json::to_string_pretty(&record.configuration).unwrap_or_else(|_| "null".into()),
    );
    text.push_str("\n```\n\n");
    if !record.digests.is_empty() {
        text.push_str("**Digests**\n\n| File | SHA-256 |\n| --- | --- |\n");
        for (file, digest) in &record.digests {
            text.push_str(&format!("| {} | `{digest}` |\n", cell(file)));
        }
        text.push('\n');
    }
    if !record.verdicts.is_empty() {
        text.push_str("**Verdicts**\n\n");
        for (row, verdict) in &record.verdicts {
            text.push_str(&format!("- {row}: {}\n", verdict_text(verdict)));
        }
        text.push('\n');
    }
    if !record.observations.is_empty() {
        text.push_str("**Observations**\n\n");
        for (row, seen) in &record.observations {
            for line in seen {
                text.push_str(&format!("- {row}: {line}\n"));
            }
        }
        text.push('\n');
    }
    if !record.notes.is_empty() {
        text.push_str("**Notes**\n\n```text\n");
        for note in &record.notes {
            text.push_str(note);
            text.push('\n');
        }
        text.push_str("```\n\n");
    }
    text.push_str("**Result lines**\n\n```text\n");
    for line in &record.lines {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("```\n\n");
    text
}

fn observation_table(rows: &[RowSpec], records: &[Record]) -> String {
    let mut text =
        "\n## Observations\n\n| Row | Run | Observed |\n| --- | --- | --- |\n".to_string();
    for row in rows {
        for spec in row
            .runs
            .iter()
            .filter(|spec| spec.expect == Expect::Observe)
        {
            let seen = match records.iter().find(|record| record.run == spec.run) {
                Some(record) if observed(record, &row.id) => record.observations[&row.id].clone(),
                Some(_) => vec!["not recorded".to_string()],
                None => vec!["not run: no record".to_string()],
            };
            for line in seen {
                text.push_str(&format!(
                    "| {} | {} | {} |\n",
                    cell(&row.id),
                    cell(&spec.run),
                    cell(&line)
                ));
            }
        }
    }
    text
}

/// The results file: `preamble`, then the row verdicts, then a table of what each observation
/// saw, then one section per run with its configuration, digests, notes and every result line. A
/// run a row reads with no record reads `not run`, never omitted.
pub fn render(preamble: &str, rows: &[RowSpec], records: &[Record]) -> String {
    let mut text = preamble.to_string();
    text.push_str("\n## Verdicts\n\n| Row | Claim | Verdict |\n| --- | --- | --- |\n");
    for row in rows {
        let verdict = row_verdict(row, records);
        text.push_str(&format!(
            "| {} | {} | {} |\n",
            cell(&row.id),
            cell(&row.claim),
            cell(&row_text(row, &verdict))
        ));
    }
    text.push_str(&observation_table(rows, records));
    text.push_str(
        "\n## Each run by row\n\n| Row | Run | What it is | Expected | Observed |\n| --- | --- | --- | --- | --- |\n",
    );
    for row in rows {
        for spec in &row.runs {
            text.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                cell(&row.id),
                cell(&spec.run),
                cell(&spec.what),
                expect_text(spec.expect),
                cell(&observed_text(row, spec, records))
            ));
        }
    }
    text.push_str("\n## Runs\n\n");
    for name in run_names(rows, records) {
        text.push_str(&section(&name, records));
    }
    text
}
