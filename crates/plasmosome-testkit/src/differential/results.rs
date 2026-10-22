use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use super::{
    Finding, Lane, Manifest, ManifestRefusal, NeutralName, Observation, Reason, Run, Subject,
    Verdict, judge,
};

/// The digests a stranger needs to rerun every lane, each the SHA-256 of what it names as 64
/// lowercase hex digits. `harness` and `session` may be `None` only when no run carries a
/// harness transcript and no reader refused one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digests {
    pub image: String,
    pub kernel: String,
    pub libkrun: String,
    pub fixtures: String,
    pub harness: Option<String>,
    pub session: Option<String>,
    pub probe_suite: String,
    pub manifest: String,
}

/// How one lane was configured, such as its command line, vCPUs, memory and devices, as names
/// and values. `settings` holds at least one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneConfig {
    pub lane: Lane,
    pub settings: BTreeMap<String, String>,
}

/// What a results file records beyond the manifest and the runs. `date` is a calendar date
/// written `YYYY-MM-DD` in ASCII digits, and `name` is lowercase ASCII letters, digits and
/// hyphens, so the file is `<date>-<name>.md`. `configs` holds at most one configuration per
/// lane, and must hold one for each lane with runs and for a lane whose output a reader refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub date: String,
    pub name: String,
    pub configs: Vec<LaneConfig>,
    pub digests: Digests,
}

/// A reader of the output a guest wrote in `lane` refused it, for `subject`: a probe's output or
/// the transcript holding a number spec 027 §6 says to refuse, or a transcript whose fixture
/// endpoint refused a request the recorded session does not hold. `reason` says what was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRefusal {
    pub lane: Lane,
    pub subject: Subject,
    pub reason: String,
}

/// What the lanes gave the results file. A refusal is written as a failed run.
#[derive(Debug, Clone, Copy)]
pub enum Outcome<'a> {
    /// [`super::load`] refused the manifest, so no lane ran.
    ManifestRefused(&'a ManifestRefusal),
    /// A reader refused a lane's output, so no run was judged, and the file holds none.
    ReadRefused {
        manifest: &'a Manifest,
        refusal: &'a ReadRefusal,
    },
    /// Every run was read. [`write_results`] judges them with [`judge`].
    Read {
        manifest: &'a Manifest,
        runs: &'a [Run],
    },
}

/// Why a results file was not written. Every refusal but `Io` comes before any file is created.
/// After `Io`, no file is left at the name, so a retry with the same name can write it.
#[derive(Debug)]
pub enum ResultsRefusal {
    BadDate { date: String },
    BadName { name: String },
    MissingConfig { lane: Lane },
    DuplicateConfig { lane: Lane },
    EmptyConfig { lane: Lane },
    BadDigest { name: &'static str },
    MissingDigest { name: &'static str },
    Io(io::Error),
}

/// Writes spec 027 §6's results to a new file `<date>-<name>.md` in `dir`, and returns its
/// path. It never overwrites a file. It judges the runs itself, so the verdict it writes is the
/// runs' own: a refusal of the runs, whether by [`super::load`], a reader or [`judge`], is
/// written as a failed run with its reason. The file states how many probes the manifest
/// declares, so a manifest with none, which `judge` refuses, reads `0` and fails. It holds each
/// lane's configuration, the eight digests, the normalization table as run, and every probe's
/// status and observation and the transcript and its status in every run of every lane, all as
/// run, not normalized. The text goes to a temporary name in `dir` first, and is linked to its
/// name only once it is whole and synced, so a write that fails leaves no file at the name.
pub fn write_results(
    outcome: Outcome<'_>,
    record: &Record,
    dir: &Path,
) -> Result<PathBuf, ResultsRefusal> {
    write_with(outcome, record, dir, |file, text| file.write_all(text))
}

static WRITES: AtomicU64 = AtomicU64::new(0);

pub(super) fn write_with(
    outcome: Outcome<'_>,
    record: &Record,
    dir: &Path,
    write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> Result<PathBuf, ResultsRefusal> {
    check(outcome, record)?;
    let text = render(outcome, record);
    let name = format!("{}-{}.md", record.date, record.name);
    let path = dir.join(&name);
    let count = WRITES.fetch_add(1, Ordering::Relaxed);
    let temporary = dir.join(format!(".{name}.{}-{count}.tmp", process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(ResultsRefusal::Io)?;
    let linked = write(&mut file, text.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::hard_link(&temporary, &path));
    drop(file);
    let removed = fs::remove_file(&temporary);
    if linked.is_ok() && removed.is_err() {
        fs::remove_file(&path).ok();
    }
    linked.and(removed).map_err(ResultsRefusal::Io)?;
    Ok(path)
}

fn check(outcome: Outcome<'_>, record: &Record) -> Result<(), ResultsRefusal> {
    if !is_date(&record.date) {
        return Err(ResultsRefusal::BadDate {
            date: record.date.clone(),
        });
    }
    let named = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-';
    if record.name.is_empty() || !record.name.bytes().all(named) {
        return Err(ResultsRefusal::BadName {
            name: record.name.clone(),
        });
    }
    let (ran, transcript): (Vec<Lane>, bool) = match outcome {
        Outcome::ManifestRefused(_) => (Vec::new(), false),
        Outcome::ReadRefused { refusal, .. } => {
            (vec![refusal.lane], refusal.subject == Subject::Transcript)
        }
        Outcome::Read { runs, .. } => (
            runs.iter().map(|run| run.lane).collect(),
            runs.iter().any(|run| run.transcript.is_some()),
        ),
    };
    for lane in Lane::ALL {
        let configs = record.configs.iter().filter(|config| config.lane == lane);
        match configs.count() {
            0 if ran.contains(&lane) => return Err(ResultsRefusal::MissingConfig { lane }),
            0 | 1 => {}
            _ => return Err(ResultsRefusal::DuplicateConfig { lane }),
        }
    }
    let empty = record
        .configs
        .iter()
        .find(|config| config.settings.is_empty());
    if let Some(config) = empty {
        return Err(ResultsRefusal::EmptyConfig { lane: config.lane });
    }
    for (name, digest) in digests(&record.digests) {
        match digest {
            Some(digest) if !is_sha256(digest) => return Err(ResultsRefusal::BadDigest { name }),
            None if transcript => return Err(ResultsRefusal::MissingDigest { name }),
            _ => {}
        }
    }
    Ok(())
}

fn is_date(date: &str) -> bool {
    let shape = date.len() == 10
        && date.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    let number = |range: Range<usize>| date.get(range).and_then(|part| part.parse::<u32>().ok());
    let (Some(year), Some(month), Some(day)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => 0,
    };
    shape && (1..=days).contains(&day)
}

fn is_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digests(digests: &Digests) -> [(&'static str, Option<&String>); 8] {
    [
        ("image", Some(&digests.image)),
        ("kernel", Some(&digests.kernel)),
        ("libkrun", Some(&digests.libkrun)),
        ("fixtures", Some(&digests.fixtures)),
        ("harness", digests.harness.as_ref()),
        ("session", digests.session.as_ref()),
        ("probe suite", Some(&digests.probe_suite)),
        ("manifest", Some(&digests.manifest)),
    ]
}

fn lane_name(lane: Lane) -> &'static str {
    match lane {
        Lane::Cell => "cell",
        Lane::Open => "open",
        Lane::Absent => "absent",
    }
}

fn block(language: &str, body: &str) -> String {
    format!("```{language}\n{body}\n```\n\n")
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

enum Judged {
    Verdict(Verdict),
    Refused(String),
}

fn judged(outcome: Outcome<'_>) -> Judged {
    match outcome {
        Outcome::ManifestRefused(refusal) => {
            Judged::Refused(format!("load refused the manifest: {refusal:?}"))
        }
        Outcome::ReadRefused { refusal, .. } => Judged::Refused(format!(
            "a reader refused the {} lane's output for {}: {:?}",
            lane_name(refusal.lane),
            subject_text(&refusal.subject),
            refusal.reason
        )),
        Outcome::Read { manifest, runs } => match judge(manifest, runs) {
            Ok(verdict) => Judged::Verdict(verdict),
            Err(refusal) => Judged::Refused(format!("judge refused the runs: {refusal:?}")),
        },
    }
}

fn render(outcome: Outcome<'_>, record: &Record) -> String {
    let (manifest, runs) = match outcome {
        Outcome::ManifestRefused(_) => (None, &[][..]),
        Outcome::ReadRefused { manifest, .. } => (Some(manifest), &[][..]),
        Outcome::Read { manifest, runs } => (Some(manifest), runs),
    };
    let judged = judged(outcome);
    let word = match &judged {
        Judged::Verdict(Verdict::Pass) => "pass",
        Judged::Verdict(Verdict::NotRun { .. }) => "not run",
        Judged::Verdict(Verdict::Fail(_)) | Judged::Refused(_) => "fail",
    };
    let probes = match manifest {
        Some(manifest) => format!("{} in the manifest", manifest.probes.len()),
        None => "0, since the manifest was refused before any run".to_string(),
    };
    let counts: Vec<String> = Lane::ALL
        .into_iter()
        .map(|lane| {
            let count = runs.iter().filter(|run| run.lane == lane).count();
            format!("{} {count}", lane_name(lane))
        })
        .collect();
    let unread = matches!(outcome, Outcome::ReadRefused { .. });
    let counts = if unread {
        "not read, since a reader refused a lane's output".to_string()
    } else {
        counts.join(", ")
    };
    let mut text = format!(
        "# Membrane differential: {}\n\nSpec 027 §6's differential, run on {}. \
         `differential::write_results` wrote this file and judged the runs itself.\n\n\
         - Verdict: **{word}**\n- Probes: {probes}\n- Runs: {counts}\n\n## Verdict\n\n",
        record.name, record.date,
    );
    text.push_str(&verdict_text(&judged));
    text.push_str("## Lanes\n\n");
    for lane in Lane::ALL {
        text.push_str(&format!("### {}\n\n", lane_name(lane)));
        match record.configs.iter().find(|config| config.lane == lane) {
            Some(config) => text.push_str(&block("json", &pretty(&json!(config.settings)))),
            None => text.push_str("No configuration given.\n\n"),
        }
    }
    text.push_str("## Digests\n\n| Input | SHA-256 |\n| --- | --- |\n");
    let none = if unread {
        "none given"
    } else {
        "none: no run carries a transcript"
    };
    for (name, digest) in digests(&record.digests) {
        let shown = digest.map_or(none.to_string(), |digest| format!("`{digest}`"));
        text.push_str(&format!("| {name} | {shown} |\n"));
    }
    text.push_str("\n## Normalization table as run\n\n");
    match manifest {
        Some(manifest) => {
            let rules: Vec<Value> = manifest
                .rules
                .iter()
                .map(|rule| {
                    json!({"probe": rule.probe, "field": rule.field, "kind": rule.kind.name()})
                })
                .collect();
            text.push_str(&block("json", &pretty(&Value::from(rules))));
        }
        None => text.push_str("None: the manifest was refused before any run.\n\n"),
    }
    text.push_str("## Runs\n\n");
    text.push_str(&runs_text(outcome));
    text
}

fn runs_text(outcome: Outcome<'_>) -> String {
    let (manifest, runs) = match outcome {
        Outcome::ManifestRefused(_) => {
            return "None: the manifest was refused before any run.\n".to_string();
        }
        Outcome::ReadRefused { .. } => {
            return "None: a reader refused a lane's output, so no run was read.\n".to_string();
        }
        Outcome::Read { manifest, runs } => (manifest, runs),
    };
    let mut text = String::new();
    for lane in Lane::ALL {
        let of_lane: Vec<&Run> = runs.iter().filter(|run| run.lane == lane).collect();
        if of_lane.is_empty() {
            text.push_str(&format!("### {}\n\nNo runs.\n\n", lane_name(lane)));
        }
        for (index, run) in of_lane.into_iter().enumerate() {
            text.push_str(&format!("### {}, run {}\n\n", lane_name(lane), index + 1));
            text.push_str(&block("json", &pretty(&run_json(manifest, run))));
        }
    }
    text
}

fn verdict_text(judged: &Judged) -> String {
    match judged {
        Judged::Refused(why) => format!(
            "The runs were refused, which is a failed run:\n\n{}",
            block("text", why)
        ),
        Judged::Verdict(Verdict::Pass) => "Every probe and the transcript ran in every lane, and \
             nothing differs that spec 027 §6 does not allow.\n\n"
            .to_string(),
        Judged::Verdict(Verdict::Fail(failures)) => listed("Failures", failures),
        Judged::Verdict(Verdict::NotRun { not_run, failures }) => {
            listed("Not run", not_run) + &listed("Failures", failures)
        }
    }
}

fn listed(title: &str, findings: &[Finding]) -> String {
    if findings.is_empty() {
        return format!("{title}: none.\n\n");
    }
    let lines: Vec<String> = findings.iter().map(finding_text).collect();
    format!("{title}:\n\n{}", block("text", &lines.join("\n")))
}

fn subject_text(subject: &Subject) -> String {
    match subject {
        Subject::Probe(probe) => format!("probe {probe:?}"),
        Subject::Transcript => "the transcript".to_string(),
    }
}

fn finding_text(finding: &Finding) -> String {
    let reason = match &finding.reason {
        Reason::NotRun { lane } => format!("not run in the {} lane", lane_name(*lane)),
        Reason::Noise { lane, field } => format!(
            "noise: the {} lane's two runs differ in field {field:?}",
            lane_name(*lane)
        ),
        Reason::NoDiscrimination => {
            "no discrimination: an open run and an absent run are equal".to_string()
        }
        Reason::Differs { field } => format!(
            "the cell lane differs from the absent lane in field {field:?}, and no allowance admits it"
        ),
        Reason::NotNeutral { lane, name } => {
            let name = match name {
                NeutralName::Mount { path, key } => {
                    format!("{} of the mount at {path:?}", key.name())
                }
                NeutralName::CgroupPath => "cgroup path".to_string(),
            };
            format!(
                "the {} lane does not show the neutral {name}",
                lane_name(*lane)
            )
        }
    };
    format!("{}: {reason}", subject_text(&finding.subject))
}

fn status(observation: &Observation, declared: bool) -> &'static str {
    match (
        declared,
        observation.completed,
        observation.failed_before_step,
    ) {
        (false, _, _) => "undeclared",
        (true, true, false) => "completed",
        (true, true, true) => "not run: failed before its step",
        (true, false, false) => "not run: no completion marker",
        (true, false, true) => "not run: no completion marker, and failed before its step",
    }
}

fn run_json(manifest: &Manifest, run: &Run) -> Value {
    let declared = |probe: &str| manifest.probes.iter().any(|spec| spec.id == probe);
    let mut observations: Vec<Value> = run
        .observations
        .iter()
        .map(|seen| {
            json!({
                "probe": seen.probe,
                "status": status(seen, declared(&seen.probe)),
                "fields": seen.fields,
            })
        })
        .collect();
    for spec in &manifest.probes {
        if !run.observations.iter().any(|seen| seen.probe == spec.id) {
            observations.push(json!({"probe": spec.id, "status": "not run: not observed"}));
        }
    }
    let transcript = match &run.transcript {
        Some(fields) => json!({"status": "completed", "fields": fields}),
        None => json!({"status": "not run: no transcript"}),
    };
    json!({
        "boot_id": run.boot_id,
        "observations": observations,
        "transcript": transcript,
    })
}
