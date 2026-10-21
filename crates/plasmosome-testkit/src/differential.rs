mod judge;
mod normalize;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use plasmosome_core::runtime::strict_json;
use serde::Deserialize;
use serde::de::{self, Deserializer};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

pub use judge::{Finding, MountKey, NeutralName, Reason, RunRefusal, Subject, Verdict, judge};

/// One of spec 027 §5's five ways a workload can observe the membrane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
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

/// The three lanes of spec 027 §6: the cell lane under test, and the open and absent
/// reference lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lane {
    Cell,
    Open,
    Absent,
}

impl Lane {
    /// Every lane, in the order findings list them.
    pub const ALL: [Lane; 3] = [Lane::Cell, Lane::Open, Lane::Absent];
}

/// One probe as the manifest declares it. `reach` is set only on a reach probe.
/// `workload_set` is required, even when empty: it names the fields whose values the workload
/// itself set, such as a time set with `utimensat`, and no rule may cover them. Fields are the
/// top-level keys of the probe's observation, and a rule covers a field's whole value, so a
/// value the workload set needs a top-level field of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeSpec {
    pub id: String,
    pub channel: Channel,
    pub step: String,
    pub reach: Option<ReachKind>,
    pub workload_set: Vec<String>,
}

/// One row of the normalization table. In each run, [`judge`] replaces each value of `field` in
/// `probe` with a placeholder, numbered within that probe and run. A placeholder names `kind`,
/// such as `<pid:2>`, except that a parent mount ID prints `<mount_id:N>`. A value that names
/// something, such as a process ID or an inode, is numbered by first appearance across every
/// field of the probe ruled with its kind, taking the fields in the byte order of their names,
/// so equal values stay equal and distinct ones distinct, even in two fields. A parent mount ID
/// is numbered with the mount IDs. A system time is numbered by rank within its field, earliest
/// first, and equal times share a number. A memory or load counter is numbered by first
/// appearance within its field. Every duration becomes `<duration>`, with no number, so
/// durations keep no relation. The field must hold one value of `kind` or a list of them, or
/// `judge` refuses the runs. No kind holds an object, so no workload-set field can sit under a
/// ruled field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub probe: String,
    pub field: String,
    pub kind: RuleKind,
}

/// The only normalization rules spec 027 §6 lists. A new one is an edit to that spec. A process
/// ID, mount ID, parent mount ID, device, inode or memory counter holds a whole number; an
/// ephemeral port a whole number up to 65535; a system time, duration or load counter any
/// number; a boot ID a UUID; and a resolved address an IPv4 or IPv6 address. A list of system
/// times must not mix integers with numbers held as `f64`: those written with a fraction or an
/// exponent and, when read from JSON text, `-0` and integers outside the `i64` and `u64` range.
/// A duration or load counter list may mix, and in a load counter list `5` and `5.0` get two
/// placeholders.
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
        match self {
            RuleKind::Pid => "pid",
            RuleKind::BootId => "boot_id",
            RuleKind::MountId => "mount_id",
            RuleKind::ParentMountId => "parent_mount_id",
            RuleKind::Device => "device",
            RuleKind::Inode => "inode",
            RuleKind::SystemTime => "system_time",
            RuleKind::Duration => "duration",
            RuleKind::EphemeralPort => "ephemeral_port",
            RuleKind::ResolvedAddress => "resolved_address",
            RuleKind::MemoryCounter => "memory_counter",
            RuleKind::LoadCounter => "load_counter",
        }
    }
}

/// A difference the cell lane may show: in `field` of `probe`, the cell lane may hold exactly
/// `cell_value` where the absent lane holds something else. [`load`] refuses a key repeated
/// inside `cell_value` at any depth, so the value a reader sees is the value compared.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowance {
    pub probe: String,
    pub field: String,
    #[serde(deserialize_with = "strict_value")]
    pub cell_value: Value,
}

/// The fixed neutral names every lane must show. `mounts_probe` reports one field per mount
/// path, an object with `source`, `fstype` and `options`, plus `subtype` on a mount whose
/// type is `fuse`. `cgroup_probe` reports the workload's cgroup path in its field `path`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Neutral {
    pub mounts_probe: String,
    pub cgroup_probe: String,
    pub mounts: Vec<NeutralMount>,
    pub fuse_subtype: String,
    pub cgroup_path: String,
}

/// The neutral source, type and options of the mount at `path`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralMount {
    pub path: String,
    pub source: String,
    pub fstype: String,
    pub options: String,
}

/// The probe manifest, fixed before the run. Build it with [`load`]. It checks only the
/// manifest's own tables, the cases [`ManifestRefusal`] lists; the rules about runs are not
/// checked here.
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
    WorkloadSetMissing {
        probe: String,
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
        named_by: NamedBy,
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
    RuleAndAllowance {
        probe: String,
        field: String,
    },
    DuplicateNeutralMount {
        path: String,
    },
    NeutralMountMissing {
        mount: RequiredMount,
    },
}

/// The manifest key that named an undeclared probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedBy {
    /// `rules[].probe`
    Rule,
    /// `allowances[].probe`
    Allowance,
    /// `neutral.mounts_probe`
    MountsProbe,
    /// `neutral.cgroup_probe`
    CgroupProbe,
}

/// A neutral mount the manifest must declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredMount {
    /// A mount at `/opt`.
    Opt,
    /// A mount at `/usr/local/bin`.
    UsrLocalBin,
    /// A mount of type `fuse` at a path other than `/opt` and `/usr/local/bin`.
    FuseWorkspace,
}

/// Parses a manifest from JSON and refuses it, before any comparison, if its own tables break
/// spec 027 §6. Every key is required except `reach`, `rules` and `allowances`. An unknown key
/// at any level is refused, and so is a key repeated in one object at any depth.
pub fn load(json: &str) -> Result<Manifest, ManifestRefusal> {
    let raw: RawManifest =
        serde_json::from_str(json).map_err(|error| ManifestRefusal::Malformed {
            reason: error.to_string(),
        })?;
    let rules = raw
        .rules
        .into_iter()
        .map(|rule| {
            let kind = RuleKind::ALL
                .into_iter()
                .find(|kind| kind.name() == rule.kind)
                .ok_or_else(|| ManifestRefusal::UnknownRuleKind {
                    probe: rule.probe.clone(),
                    field: rule.field.clone(),
                    kind: rule.kind.clone(),
                })?;
            Ok(Rule {
                probe: rule.probe,
                field: rule.field,
                kind,
            })
        })
        .collect::<Result<Vec<Rule>, ManifestRefusal>>()?;
    let probes = raw
        .probes
        .into_iter()
        .map(|probe| {
            let workload_set =
                probe
                    .workload_set
                    .ok_or_else(|| ManifestRefusal::WorkloadSetMissing {
                        probe: probe.id.clone(),
                    })?;
            Ok(ProbeSpec {
                id: probe.id,
                channel: probe.channel,
                step: probe.step,
                reach: probe.reach,
                workload_set,
            })
        })
        .collect::<Result<Vec<ProbeSpec>, ManifestRefusal>>()?;
    let manifest = Manifest {
        probes,
        rules,
        allowances: raw.allowances,
        neutral: raw.neutral,
    };
    manifest.validate()?;
    Ok(manifest)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    probes: Vec<RawProbe>,
    #[serde(default)]
    rules: Vec<RawRule>,
    #[serde(default)]
    allowances: Vec<Allowance>,
    neutral: Neutral,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProbe {
    id: String,
    channel: Channel,
    step: String,
    #[serde(default)]
    reach: Option<ReachKind>,
    workload_set: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    probe: String,
    field: String,
    kind: String,
}

fn strict_value<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Value, D::Error> {
    let raw = Box::<RawValue>::deserialize(deserializer)?;
    strict_json::parse_value(raw.get().as_bytes()).map_err(de::Error::custom)
}

const FUSE: &str = "fuse";

impl Manifest {
    /// Checks the measurement rules [`load`] checks, for a manifest built by hand.
    pub fn validate(&self) -> Result<(), ManifestRefusal> {
        let mut ids = BTreeSet::new();
        for probe in &self.probes {
            if !ids.insert(probe.id.as_str()) {
                return Err(ManifestRefusal::DuplicateProbe {
                    id: probe.id.clone(),
                });
            }
        }
        for channel in Channel::ALL {
            let count = self
                .probes
                .iter()
                .filter(|probe| probe.channel == channel)
                .count();
            if count < 3 {
                return Err(ManifestRefusal::ChannelShort { channel, count });
            }
        }
        for kind in ReachKind::ALL {
            let count = self
                .probes
                .iter()
                .filter(|probe| probe.reach == Some(kind))
                .count();
            if count < 3 {
                return Err(ManifestRefusal::KindShort { kind, count });
            }
        }
        let known = |probe: &str, named_by| match ids.contains(probe) {
            true => Ok(()),
            false => Err(ManifestRefusal::UnknownProbe {
                probe: probe.to_string(),
                named_by,
            }),
        };
        let mut ruled = BTreeSet::new();
        for rule in &self.rules {
            known(&rule.probe, NamedBy::Rule)?;
            let set_by_workload = self
                .probes
                .iter()
                .any(|probe| probe.id == rule.probe && probe.workload_set.contains(&rule.field));
            if set_by_workload {
                return Err(ManifestRefusal::RuleOnWorkloadSetField {
                    probe: rule.probe.clone(),
                    field: rule.field.clone(),
                });
            }
            if !ruled.insert((&rule.probe, &rule.field)) {
                return Err(ManifestRefusal::DuplicateRule {
                    probe: rule.probe.clone(),
                    field: rule.field.clone(),
                });
            }
        }
        let mut allowed = BTreeSet::new();
        for allowance in &self.allowances {
            known(&allowance.probe, NamedBy::Allowance)?;
            if !allowed.insert((&allowance.probe, &allowance.field)) {
                return Err(ManifestRefusal::DuplicateAllowance {
                    probe: allowance.probe.clone(),
                    field: allowance.field.clone(),
                });
            }
            if ruled.contains(&(&allowance.probe, &allowance.field)) {
                return Err(ManifestRefusal::RuleAndAllowance {
                    probe: allowance.probe.clone(),
                    field: allowance.field.clone(),
                });
            }
        }
        known(&self.neutral.mounts_probe, NamedBy::MountsProbe)?;
        known(&self.neutral.cgroup_probe, NamedBy::CgroupProbe)?;
        let mounts = &self.neutral.mounts;
        let mut paths = BTreeSet::new();
        for mount in mounts {
            if !paths.insert(mount.path.as_str()) {
                return Err(ManifestRefusal::DuplicateNeutralMount {
                    path: mount.path.clone(),
                });
            }
        }
        let software = [
            (RequiredMount::Opt, "/opt"),
            (RequiredMount::UsrLocalBin, "/usr/local/bin"),
        ];
        for (mount, path) in software {
            if !paths.contains(path) {
                return Err(ManifestRefusal::NeutralMountMissing { mount });
            }
        }
        let workspace = mounts.iter().any(|neutral| {
            neutral.fstype == FUSE && software.iter().all(|(_, path)| neutral.path != *path)
        });
        if !workspace {
            return Err(ManifestRefusal::NeutralMountMissing {
                mount: RequiredMount::FuseWorkspace,
            });
        }
        Ok(())
    }
}

/// What one probe reported in one run. `completed` is its completion marker, and
/// `failed_before_step` says it failed before the step it observes. Each field is compared
/// whole. A field a rule covers is compared after its values are replaced by placeholders, and
/// every other field exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub probe: String,
    pub completed: bool,
    pub failed_before_step: bool,
    pub fields: Map<String, Value>,
}

/// One run of one lane from a fresh boot. `transcript` holds what the harness did, such as its
/// tool calls, results, exit statuses and files left behind, each field compared exactly; it is
/// `None` when the harness left no transcript, which leaves the transcript not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub lane: Lane,
    pub boot_id: String,
    pub observations: Vec<Observation>,
    pub transcript: Option<Map<String, Value>>,
}
