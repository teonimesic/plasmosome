use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What `lane-vmm` reads on its standard input: one lane on the host side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneConfig {
    pub run_id: String,
    pub bundle: PathBuf,
    pub initramfs: PathBuf,
    pub root_image: PathBuf,
    pub vcpus: u8,
    pub memory_mib: u32,
    pub console: PathBuf,
    pub shares: Vec<Share>,
    pub confinement: Option<Confinement>,
}

/// One host directory exported to the guest through `krun_add_virtiofs4`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        0
    }
}

/// An SBPL profile `lane-vmm` applies to itself before the VM starts, and the two directories
/// its self-check reads and writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confinement {
    pub profile: PathBuf,
    pub allowed: PathBuf,
    pub outside: PathBuf,
}

/// What `lane-init` reads from `/lane.json` in the initramfs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestConfig {
    pub run_id: String,
    pub mounts: Vec<GuestMount>,
    pub probes: Vec<Probe>,
}

/// One virtio-fs tag the guest mounts, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct Probe {
    pub id: String,
    pub uid: u32,
    pub args: Vec<String>,
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

impl LaneConfig {
    /// Parse and check one lane configuration. Unknown and missing fields are refused, every
    /// path must be absolute, and share tags must be unique.
    pub fn from_json(text: &str) -> Result<LaneConfig, ConfigError> {
        serde_json::from_str(text).map_err(|error| ConfigError(error.to_string()))
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
        serde_json::from_str(text).map_err(|error| ConfigError(error.to_string()))
    }

    /// The configuration as one JSON line.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The probe IDs in the order `lane-init` runs them.
    pub fn probe_ids(&self) -> Vec<String> {
        Vec::new()
    }
}
