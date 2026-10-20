#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One of spec 027 §5's five ways a workload can observe the membrane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Refusal,
    Hint,
    Operation,
    Facility,
    Disturbance,
}

impl Channel {
    /// Every channel, in §5's order.
    pub const ALL: [Channel; 5] = [
        Channel::Refusal,
        Channel::Hint,
        Channel::Operation,
        Channel::Facility,
        Channel::Disturbance,
    ];
}

/// What a reach probe reaches outside the granted set: a host, a path or a kernel facility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachKind {
    Host,
    Path,
    Facility,
}

impl ReachKind {
    /// Every kind, in §6's order.
    pub const ALL: [ReachKind; 3] = [ReachKind::Host, ReachKind::Path, ReachKind::Facility];
}

/// One probe as the manifest declares it. `reach` is set only on a reach probe.
/// `workload_set` names the fields whose values the workload itself set, such as a time set
/// with `utimensat`; those are always compared exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeSpec {
    pub id: String,
    pub channel: Channel,
    pub step: String,
    #[serde(default)]
    pub reach: Option<ReachKind>,
    #[serde(default)]
    pub workload_set: Vec<String>,
}

/// One row of the normalization table: the values of `field` in `probe` are replaced by
/// placeholders of `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub probe: String,
    pub field: String,
    pub kind: RuleKind,
}

/// The only normalization rules spec 027 §6 lists. A new one is an edit to that spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RuleKind {
    Pid,
    BootId,
    MountId,
    ParentMountId,
    Device,
    Inode,
    SystemTime,
    Duration,
    EphemeralPort,
    ResolvedAddress,
    MemoryCounter,
    LoadCounter,
}

impl RuleKind {
    /// Every rule kind, in §6's order.
    pub const ALL: [RuleKind; 12] = [
        RuleKind::Pid,
        RuleKind::BootId,
        RuleKind::MountId,
        RuleKind::ParentMountId,
        RuleKind::Device,
        RuleKind::Inode,
        RuleKind::SystemTime,
        RuleKind::Duration,
        RuleKind::EphemeralPort,
        RuleKind::ResolvedAddress,
        RuleKind::MemoryCounter,
        RuleKind::LoadCounter,
    ];

    /// The name a manifest uses for this kind, such as `system_time`.
    pub fn name(self) -> &'static str {
        unimplemented!()
    }

    /// Whether placeholders of this kind keep the order of the values they replace. Only
    /// the system's times and durations do.
    pub fn keeps_order(self) -> bool {
        unimplemented!()
    }
}

/// A difference the cell lane may show: in `field` of `probe`, the cell lane may hold exactly
/// `cell_value` where the absent lane holds something else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowance {
    pub probe: String,
    pub field: String,
    pub cell_value: Value,
}

/// The fixed neutral names every lane must show. `mounts_probe` reports one field per mount
/// path, an object with `source`, `fstype` and `options`, plus `subtype` on a mount whose
/// type is `fuse`. `cgroup_probe` reports the workload's cgroup path in its field `path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Neutral {
    pub mounts_probe: String,
    pub cgroup_probe: String,
    pub mounts: Vec<NeutralMount>,
    pub fuse_subtype: String,
    pub cgroup_path: String,
}

/// The neutral source, type and options of the mount at `path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralMount {
    pub path: String,
    pub source: String,
    pub fstype: String,
    pub options: String,
}

/// The probe manifest, fixed before the run. Build it with [`load`], which refuses one that
/// breaks spec 027 §6's measurement rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub probes: Vec<ProbeSpec>,
    pub rules: Vec<Rule>,
    pub allowances: Vec<Allowance>,
    pub neutral: Neutral,
}

/// Why a manifest cannot be used. Each variant names what it refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestRefusal {
    Malformed {
        reason: String,
    },
    DuplicateProbe {
        id: String,
    },
    ChannelShort {
        channel: Channel,
        count: usize,
    },
    KindShort {
        kind: ReachKind,
        count: usize,
    },
    UnknownRuleKind {
        probe: String,
        field: String,
        kind: String,
    },
    UnknownProbe {
        probe: String,
        named_by: &'static str,
    },
    RuleOnWorkloadSetField {
        probe: String,
        field: String,
    },
    DuplicateRule {
        probe: String,
        field: String,
    },
    DuplicateAllowance {
        probe: String,
        field: String,
    },
    NeutralMountMissing {
        mount: &'static str,
    },
}

/// Parses a manifest from JSON and refuses it before any comparison if spec 027 §6's
/// measurement rules do not hold.
pub fn load(json: &str) -> Result<Manifest, ManifestRefusal> {
    let _ = json;
    unimplemented!()
}

impl Manifest {
    /// Checks the measurement rules [`load`] checks, for a manifest built by hand.
    pub fn validate(&self) -> Result<(), ManifestRefusal> {
        unimplemented!()
    }
}
