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
    /// A reach probe is equal in the open and absent lanes.
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
/// Every field is compared exactly: the manifest's rules are not applied yet, so a rule never
/// loosens a comparison.
pub fn judge(manifest: &Manifest, runs: &[Run]) -> Result<Verdict, RunRefusal> {
    manifest.validate().map_err(RunRefusal::Manifest)?;
    let lanes = pair(manifest, runs)?;
    let mut findings = Vec::new();
    for probe in &manifest.probes {
        let subject = Subject::Probe(probe.id.clone());
        let mut ran = Vec::new();
        for (lane, pair) in &lanes {
            let Some(observations) = pair.and_then(|runs| completed(&probe.id, runs)) else {
                not_run(&subject, *lane, &mut findings);
                continue;
            };
            for name in observations
                .iter()
                .flat_map(|seen| not_neutral(manifest, seen))
            {
                let reason = Reason::NotNeutral { lane: *lane, name };
                push(&mut findings, &subject, reason);
            }
            ran.push((*lane, observations.map(|seen| seen.fields.clone())));
        }
        let allowed = |field: &str, value: &Value| {
            manifest.allowances.iter().any(|allowance| {
                allowance.probe == probe.id
                    && allowance.field == field
                    && allowance.cell_value == *value
            })
        };
        let reach = probe.reach.is_some();
        compare(&subject, &ran, reach, allowed, &mut findings);
    }
    let mut ran = Vec::new();
    for (lane, pair) in &lanes {
        let transcripts = pair.and_then(|[first, second]| {
            Some([first.transcript.clone()?, second.transcript.clone()?])
        });
        match transcripts {
            Some(transcripts) => ran.push((*lane, transcripts)),
            None => not_run(&Subject::Transcript, *lane, &mut findings),
        }
    }
    compare(
        &Subject::Transcript,
        &ran,
        false,
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

fn completed<'a>(probe: &str, runs: [&'a Run; 2]) -> Option<[&'a Observation; 2]> {
    let [first, second] = runs.map(|run| {
        run.observations
            .iter()
            .find(|seen| seen.probe == probe)
            .filter(|seen| seen.completed && !seen.failed_before_step)
    });
    Some([first?, second?])
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

fn not_neutral(manifest: &Manifest, observation: &Observation) -> Vec<NeutralName> {
    let neutral = &manifest.neutral;
    let mut names = Vec::new();
    if observation.probe == neutral.mounts_probe {
        for mount in &neutral.mounts {
            let mut expected = vec![
                (MountKey::Source, &mount.source),
                (MountKey::Fstype, &mount.fstype),
                (MountKey::Options, &mount.options),
            ];
            if mount.fstype == FUSE {
                expected.push((MountKey::Subtype, &neutral.fuse_subtype));
            }
            let shown = observation.fields.get(&mount.path);
            for (key, value) in expected {
                let name = shown.and_then(|shown| shown.get(key.name()));
                if name.and_then(Value::as_str) != Some(value.as_str()) {
                    let path = mount.path.clone();
                    names.push(NeutralName::Mount { path, key });
                }
            }
        }
    }
    let cgroup = observation.fields.get("path").and_then(Value::as_str);
    if observation.probe == neutral.cgroup_probe && cgroup != Some(neutral.cgroup_path.as_str()) {
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

fn compare(
    subject: &Subject,
    ran: &[(Lane, [Fields; 2])],
    reach: bool,
    allowed: impl Fn(&str, &Value) -> bool,
    findings: &mut Vec<Finding>,
) {
    for (lane, [first, second]) in ran {
        for field in differing(first, second) {
            push(findings, subject, Reason::Noise { lane: *lane, field });
        }
    }
    let lane = |wanted| {
        ran.iter()
            .find(|(lane, _)| *lane == wanted)
            .map(|(_, runs)| runs)
    };
    if let (Some(open), Some(absent)) = (lane(Lane::Open), lane(Lane::Absent))
        && reach
        && open.iter().zip(absent).any(|(open, absent)| open == absent)
    {
        push(findings, subject, Reason::NoDiscrimination);
    }
    if let (Some(cell), Some(absent)) = (lane(Lane::Cell), lane(Lane::Absent)) {
        let mut fields = BTreeSet::new();
        for (cell, absent) in cell.iter().zip(absent) {
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
}

fn differing(first: &Fields, second: &Fields) -> Vec<String> {
    let keys: BTreeSet<&String> = first.keys().chain(second.keys()).collect();
    keys.into_iter()
        .filter(|key| first.get(*key) != second.get(*key))
        .cloned()
        .collect()
}
