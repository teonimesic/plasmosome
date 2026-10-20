use super::{Lane, Manifest, ManifestRefusal, Run};

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
    let _ = (manifest, runs);
    unimplemented!()
}
