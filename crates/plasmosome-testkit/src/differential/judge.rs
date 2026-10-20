use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::{FUSE, Lane, Manifest, ManifestRefusal, Observation, Run};

/// What a finding is about: a declared probe, or the harness transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Probe(String),
    Transcript,
}

/// Why a subject did not pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// `lane` has no runs, or one of its runs has no completion marker for the subject, or the
    /// subject failed there before the step it observes.
    NotRun { lane: Lane },
    /// The two runs of `lane` differ in `field`.
    Noise { lane: Lane, field: String },
    /// A completed open run and a completed absent run of a reach probe are equal, leaving out
    /// the fields a rule covers.
    NoDiscrimination,
    /// The cell lane differs from the absent lane in `field`, and no allowance admits the
    /// cell's value there.
    Differs { field: String },
    /// `lane` does not show the manifest's neutral value for `name`.
    NotNeutral { lane: Lane, name: NeutralName },
}

/// A name spec 027 §6 fixes to a neutral value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeutralName {
    /// `key` of the mount the mounts probe reports at `path`.
    Mount { path: String, key: MountKey },
    /// The cgroup probe's `path`.
    CgroupPath,
}

/// A key of one mount in the mounts probe. `Subtype` is checked only on a `fuse` mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountKey {
    Source,
    Fstype,
    Options,
    Subtype,
}

/// One reason one subject did not pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub subject: Subject,
    pub reason: Reason,
}

/// The differential's verdict. `NotRun` wins over `Fail`, and still lists the failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail(Vec<Finding>),
    NotRun {
        not_run: Vec<Finding>,
        failures: Vec<Finding>,
    },
}

/// Why runs cannot be judged at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunRefusal {
    /// The manifest breaks a rule [`super::load`] checks.
    Manifest(ManifestRefusal),
    /// A lane has runs, but not exactly two.
    RunCount { lane: Lane, count: usize },
    /// Two runs share one boot ID, so they are not two fresh boots.
    SameBoot {
        boot_id: String,
        first: Lane,
        second: Lane,
    },
    /// A run observed a probe the manifest does not declare.
    UnknownProbe { lane: Lane, probe: String },
    /// A run observed one probe twice.
    DuplicateObservation { lane: Lane, probe: String },
}

/// Judges `runs` against `manifest` under spec 027 §6. A lane has no runs, which leaves every
/// probe not run in it, or exactly two. The verdict passes only when every probe and the
/// transcript ran in every lane, no run differs from its lane's other run, every reach probe
/// tells the open lane from the absent lane, every lane shows the neutral names, and the cell
/// lane equals the absent lane except where an allowance admits the cell's exact value.
/// Neutral names are checked on every completed run, and every completed run of one lane is
/// compared with every completed run of the other. So a failure is listed even when a lane's
/// other run is not run, and the verdict does not depend on the order of a lane's runs. Every
/// field is compared exactly, because the manifest's rules are not applied yet. That can only
/// find more noise and more differences. Discrimination leaves out the fields a rule covers, so
/// a value a rule would replace cannot make a reach probe look as if it discriminated.
pub fn judge(manifest: &Manifest, runs: &[Run]) -> Result<Verdict, RunRefusal> {
    manifest.validate().map_err(RunRefusal::Manifest)?;
    let lanes = pair(manifest, runs)?;
    let mut findings = Vec::new();
    for probe in &manifest.probes {
        let subject = Subject::Probe(probe.id.clone());
        let mut ran = Vec::new();
        for (lane, pair) in &lanes {
            let observations = pair.map(|runs| runs.map(|run| completed(&probe.id, run)));
            if !matches!(observations, Some([Some(_), Some(_)])) {
                not_run(&subject, *lane, &mut findings);
            }
            let Some(observations) = observations else {
                continue;
            };
            let shown: Vec<&Observation> = observations.iter().flatten().copied().collect();
            for name in not_neutral(manifest, &probe.id, &shown) {
                let reason = Reason::NotNeutral { lane: *lane, name };
                push(&mut findings, &subject, reason);
            }
            let fields = observations.map(|seen| seen.map(|seen| seen.fields.clone()));
            ran.push((*lane, fields));
        }
        let allowed = |field: &str, value: &Value| {
            manifest.allowances.iter().any(|allowance| {
                allowance.probe == probe.id
                    && allowance.field == field
                    && allowance.cell_value == *value
            })
        };
        let ruled: Vec<&str> = manifest
            .rules
            .iter()
            .filter(|rule| rule.probe == probe.id)
            .map(|rule| rule.field.as_str())
            .collect();
        let reach = probe.reach.map(|_| ruled.as_slice());
        compare(&subject, &ran, reach, allowed, &mut findings);
    }
    let mut ran = Vec::new();
    for (lane, pair) in &lanes {
        let transcripts = pair.map(|runs| runs.map(|run| run.transcript.clone()));
        if !matches!(transcripts, Some([Some(_), Some(_)])) {
            not_run(&Subject::Transcript, *lane, &mut findings);
        }
        if let Some(transcripts) = transcripts {
            ran.push((*lane, transcripts));
        }
    }
    compare(
        &Subject::Transcript,
        &ran,
        None,
        |_, _| false,
        &mut findings,
    );
    let (not_run, failures): (Vec<Finding>, Vec<Finding>) = findings
        .into_iter()
        .partition(|finding| matches!(finding.reason, Reason::NotRun { .. }));
    Ok(match (not_run.is_empty(), failures.is_empty()) {
        (true, true) => Verdict::Pass,
        (true, false) => Verdict::Fail(failures),
        (false, _) => Verdict::NotRun { not_run, failures },
    })
}

type Fields = Map<String, Value>;

type Lanes<'a> = Vec<(Lane, Option<[&'a Run; 2]>)>;

fn pair<'a>(manifest: &Manifest, runs: &'a [Run]) -> Result<Lanes<'a>, RunRefusal> {
    let mut boots = BTreeMap::new();
    for run in runs {
        if let Some(first) = boots.insert(&run.boot_id, run.lane) {
            return Err(RunRefusal::SameBoot {
                boot_id: run.boot_id.clone(),
                first,
                second: run.lane,
            });
        }
        let mut seen = BTreeSet::new();
        for observation in &run.observations {
            let (lane, probe) = (run.lane, observation.probe.clone());
            if !manifest.probes.iter().any(|declared| declared.id == probe) {
                return Err(RunRefusal::UnknownProbe { lane, probe });
            }
            if !seen.insert(&observation.probe) {
                return Err(RunRefusal::DuplicateObservation { lane, probe });
            }
        }
    }
    Lane::ALL
        .into_iter()
        .map(|lane| {
            let of_lane: Vec<&Run> = runs.iter().filter(|run| run.lane == lane).collect();
            match of_lane[..] {
                [] => Ok((lane, None)),
                [first, second] => Ok((lane, Some([first, second]))),
                _ => Err(RunRefusal::RunCount {
                    lane,
                    count: of_lane.len(),
                }),
            }
        })
        .collect()
}

fn completed<'a>(probe: &str, run: &'a Run) -> Option<&'a Observation> {
    run.observations
        .iter()
        .find(|seen| seen.probe == probe)
        .filter(|seen| seen.completed && !seen.failed_before_step)
}

fn push(findings: &mut Vec<Finding>, subject: &Subject, reason: Reason) {
    let finding = Finding {
        subject: subject.clone(),
        reason,
    };
    if !findings.contains(&finding) {
        findings.push(finding);
    }
}

fn not_run(subject: &Subject, lane: Lane, findings: &mut Vec<Finding>) {
    push(findings, subject, Reason::NotRun { lane });
}

fn not_neutral(manifest: &Manifest, probe: &str, shown: &[&Observation]) -> Vec<NeutralName> {
    let neutral = &manifest.neutral;
    let mut names = Vec::new();
    if probe == neutral.mounts_probe {
        for mount in &neutral.mounts {
            let mut expected = vec![
                (MountKey::Source, &mount.source),
                (MountKey::Fstype, &mount.fstype),
                (MountKey::Options, &mount.options),
            ];
            if mount.fstype == FUSE {
                expected.push((MountKey::Subtype, &neutral.fuse_subtype));
            }
            for (key, value) in expected {
                let differs = |seen: &&Observation| {
                    let reported = seen.fields.get(&mount.path);
                    let name = reported.and_then(|reported| reported.get(key.name()));
                    name.and_then(Value::as_str) != Some(value.as_str())
                };
                if shown.iter().any(differs) {
                    let path = mount.path.clone();
                    names.push(NeutralName::Mount { path, key });
                }
            }
        }
    }
    let expected = Some(neutral.cgroup_path.as_str());
    let differs = |seen: &&Observation| seen.fields.get("path").and_then(Value::as_str) != expected;
    if probe == neutral.cgroup_probe && shown.iter().any(differs) {
        names.push(NeutralName::CgroupPath);
    }
    names
}

impl MountKey {
    fn name(self) -> &'static str {
        match self {
            MountKey::Source => "source",
            MountKey::Fstype => "fstype",
            MountKey::Options => "options",
            MountKey::Subtype => "subtype",
        }
    }
}

type Ran = [(Lane, [Option<Fields>; 2])];

fn compare(
    subject: &Subject,
    ran: &Ran,
    reach: Option<&[&str]>,
    allowed: impl Fn(&str, &Value) -> bool,
    findings: &mut Vec<Finding>,
) {
    for (lane, runs) in ran {
        if let [Some(first), Some(second)] = runs {
            for field in differing(first, second) {
                push(findings, subject, Reason::Noise { lane: *lane, field });
            }
        }
    }
    if let Some(ruled) = reach {
        let unruled = |fields: &Fields| {
            let mut fields = fields.clone();
            fields.retain(|field, _| !ruled.contains(&field.as_str()));
            fields
        };
        let pairs = completed_pairs(ran, Lane::Open, Lane::Absent);
        if pairs
            .iter()
            .any(|(open, absent)| unruled(open) == unruled(absent))
        {
            push(findings, subject, Reason::NoDiscrimination);
        }
    }
    let mut fields = BTreeSet::new();
    for (cell, absent) in completed_pairs(ran, Lane::Cell, Lane::Absent) {
        for field in differing(cell, absent) {
            if !cell.get(&field).is_some_and(|value| allowed(&field, value)) {
                fields.insert(field);
            }
        }
    }
    for field in fields {
        push(findings, subject, Reason::Differs { field });
    }
}

fn completed_pairs(ran: &Ran, left: Lane, right: Lane) -> Vec<(&Fields, &Fields)> {
    let runs = |wanted| {
        ran.iter()
            .find(|(lane, _)| *lane == wanted)
            .map(|(_, runs)| runs)
    };
    let (Some(left), Some(right)) = (runs(left), runs(right)) else {
        return Vec::new();
    };
    left.iter()
        .flatten()
        .flat_map(|left| right.iter().flatten().map(move |right| (left, right)))
        .collect()
}

fn differing(first: &Fields, second: &Fields) -> Vec<String> {
    let keys: BTreeSet<&String> = first.keys().chain(second.keys()).collect();
    keys.into_iter()
        .filter(|key| first.get(*key) != second.get(*key))
        .cloned()
        .collect()
}
