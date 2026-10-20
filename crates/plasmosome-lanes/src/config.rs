use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The longest tag a virtio-fs device carries.
pub const MAX_TAG_BYTES: usize = 36;

/// The uid a probe runs as when its configuration names none.
pub const WORKLOAD_UID: u32 = 1000;

/// What `lane-vmm` reads on its standard input: one lane on the host side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaneConfig {
    pub run_id: String,
    pub bundle: PathBuf,
    pub initramfs: PathBuf,
    pub root_image: PathBuf,
    pub vcpus: u8,
    pub memory_mib: u32,
    pub console: PathBuf,
    pub shares: Vec<Share>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub confinement: Option<Confinement>,
}

/// One host directory exported to the guest through `krun_add_virtiofs4`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Share {
    pub tag: String,
    pub path: PathBuf,
    pub shm_size: u64,
    pub read_only: bool,
    pub semantics: Semantics,
}

/// The permission semantics libkrun's virtio-fs server emulates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Semantics {
    Complete,
    Simplified,
}

impl Semantics {
    /// The pinned header's value: `KRUN_SEMANTICS_LINUX_COMPLETE` is 0, `..._SIMPLIFIED` is 1.
    pub fn krun_value(self) -> u32 {
        match self {
            Semantics::Complete => 0,
            Semantics::Simplified => 1,
        }
    }
}

/// An SBPL profile `lane-vmm` applies to itself before the VM starts, and the two directories
/// its self-check reads and writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confinement {
    pub profile: PathBuf,
    pub allowed: PathBuf,
    pub outside: PathBuf,
}

/// What `lane-init` reads from `/lane.json` in the initramfs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestConfig {
    pub run_id: String,
    pub mounts: Vec<GuestMount>,
    pub probes: Vec<Probe>,
}

/// One virtio-fs tag the guest mounts, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestMount {
    pub tag: String,
    pub path: PathBuf,
    pub options: Vec<MountOption>,
}

/// A mount flag `lane-init` passes to mount(2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MountOption {
    Ro,
    Nosuid,
    Nodev,
    Noexec,
}

/// One `lane-probe` run: its arguments and the uid it runs as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub id: String,
    #[serde(default = "workload_uid")]
    pub uid: u32,
    pub args: Vec<String>,
}

fn workload_uid() -> u32 {
    WORKLOAD_UID
}

/// Why a configuration was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

fn refuse<T>(message: String) -> Result<T, ConfigError> {
    Err(ConfigError(message))
}

fn check_absolute(field: &str, path: &Path) -> Result<(), ConfigError> {
    let text = path.to_string_lossy();
    if !path.is_absolute() {
        return refuse(format!("{field} must be an absolute path, not `{text}`"));
    }
    if text.contains('\0') {
        return refuse(format!("{field} must not hold a NUL byte: `{text}`"));
    }
    Ok(())
}

fn check_name(field: &str, value: &str) -> Result<(), ConfigError> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    if value.is_empty() || !value.chars().all(allowed) {
        return refuse(format!(
            "{field} `{value}` must be nonempty and use only letters, digits, `.`, `_` and `-`"
        ));
    }
    Ok(())
}

fn check_unique<'a>(kind: &str, values: impl Iterator<Item = &'a str>) -> Result<(), ConfigError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return refuse(format!("{kind} `{value}` appears more than once"));
        }
    }
    Ok(())
}

fn check_tag(tag: &str) -> Result<(), ConfigError> {
    if tag.is_empty() || tag.len() > MAX_TAG_BYTES || tag.contains('\0') {
        return refuse(format!(
            "tag `{tag}` must be 1 to {MAX_TAG_BYTES} bytes with no NUL"
        ));
    }
    Ok(())
}

fn parse<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, ConfigError> {
    serde_json::from_str(text).map_err(|error| ConfigError(error.to_string()))
}

impl LaneConfig {
    /// Parse and check one lane configuration. Unknown and missing fields are refused, every
    /// path must be absolute, and share tags must be unique.
    pub fn from_json(text: &str) -> Result<LaneConfig, ConfigError> {
        let config: LaneConfig = parse(text)?;
        check_name("run_id", &config.run_id)?;
        check_absolute("bundle", &config.bundle)?;
        check_absolute("initramfs", &config.initramfs)?;
        check_absolute("root_image", &config.root_image)?;
        check_absolute("console", &config.console)?;
        if config.vcpus == 0 || config.memory_mib == 0 {
            return refuse("vcpus and memory_mib must be positive".to_string());
        }
        for share in &config.shares {
            check_tag(&share.tag)?;
            check_absolute("a share path", &share.path)?;
        }
        check_unique(
            "share tag",
            config.shares.iter().map(|share| share.tag.as_str()),
        )?;
        if let Some(confinement) = &config.confinement {
            check_absolute("confinement.profile", &confinement.profile)?;
            check_absolute("confinement.allowed", &confinement.allowed)?;
            check_absolute("confinement.outside", &confinement.outside)?;
        }
        Ok(config)
    }

    /// The configuration as one JSON line.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

impl GuestConfig {
    /// Parse and check `/lane.json`. Unknown and missing fields are refused, except a probe's
    /// `uid`, which defaults to 1000; mount paths must be absolute, and tags and probe IDs unique.
    pub fn from_json(text: &str) -> Result<GuestConfig, ConfigError> {
        let config: GuestConfig = parse(text)?;
        check_name("run_id", &config.run_id)?;
        for mount in &config.mounts {
            check_tag(&mount.tag)?;
            check_absolute("a mount path", &mount.path)?;
        }
        check_unique(
            "mount tag",
            config.mounts.iter().map(|mount| mount.tag.as_str()),
        )?;
        for probe in &config.probes {
            check_name("probe id", &probe.id)?;
            if probe.args.is_empty() {
                return refuse(format!("probe `{}` names no subcommand", probe.id));
            }
        }
        check_unique(
            "probe id",
            config.probes.iter().map(|probe| probe.id.as_str()),
        )?;
        Ok(config)
    }

    /// The configuration as one JSON line.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The probe IDs in the order `lane-init` runs them.
    pub fn probe_ids(&self) -> Vec<String> {
        self.probes.iter().map(|probe| probe.id.clone()).collect()
    }
}
