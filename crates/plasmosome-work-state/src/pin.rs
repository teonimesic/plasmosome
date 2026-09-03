use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::command::{CommandRunner, CommandSpec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinError {
    code: &'static str,
}
impl PinError {
    pub fn code(&self) -> &'static str {
        self.code
    }
}
impl std::fmt::Display for PinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code)
    }
}
impl std::error::Error for PinError {}

fn error(code: &'static str) -> PinError {
    PinError { code }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinManifest {
    pub version: String,
    pub release: String,
    pub source_commit: String,
    pub license: String,
    pub checksums_url: String,
    pub checksums_sha256: String,
    pub targets: Vec<PinnedTarget>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedTarget {
    pub target: String,
    pub archive: String,
    pub archive_sha256: String,
    pub binary_sha256: String,
}

impl PinManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PinError> {
        let source = fs::read_to_string(path).map_err(|_| error("invalid_beads_pin"))?;
        Self::parse(&source)
    }

    /// Parses and validates pinned release contents embedded by an installed wrapper.
    pub fn parse(source: &str) -> Result<Self, PinError> {
        let pin: Self = toml::from_str(source).map_err(|_| error("invalid_beads_pin"))?;
        let names: BTreeSet<_> = pin
            .targets
            .iter()
            .map(|target| target.target.as_str())
            .collect();
        let hashes_are_valid = std::iter::once(&pin.checksums_sha256)
            .chain(
                pin.targets
                    .iter()
                    .flat_map(|target| [&target.archive_sha256, &target.binary_sha256]),
            )
            .all(|hash| hash.len() == 64 && hex(hash));
        if pin.version != "1.1.2"
            || !https(&pin.release)
            || !https(&pin.checksums_url)
            || pin.source_commit.len() != 40
            || !hex(&pin.source_commit)
            || pin.license != "MIT"
            || names.len() != pin.targets.len()
            || !hashes_are_valid
            || pin.targets.iter().any(|target| {
                !target.archive.starts_with("beads_1.1.2_") || !target.archive.ends_with(".tar.gz")
            })
        {
            return Err(error("invalid_beads_pin"));
        }
        Ok(pin)
    }
}

fn https(value: &str) -> bool {
    value.starts_with("https://")
}
fn hex(value: &str) -> bool {
    value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn checksum(path: &Path) -> Result<String, PinError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(fs::read(path).map_err(|_| error("beads_checksum_mismatch"))?)
    ))
}

#[derive(Debug, Clone)]
pub struct VerifiedBeads {
    pub target: String,
}

/// A pinned Beads executable verified after it has been installed in a generation.
#[derive(Debug, Clone)]
pub struct InstalledBeads {
    /// The pinned host target selected for this executable.
    pub target: String,
}

impl VerifiedBeads {
    pub fn verify<R: CommandRunner>(
        manifest: &PinManifest,
        target: &str,
        archive: &Path,
        binary: &Path,
        runner: &mut R,
    ) -> Result<Self, PinError> {
        Self::verify_with_environment(manifest, target, archive, binary, BTreeMap::new(), runner)
    }

    pub fn verify_with_environment<R: CommandRunner>(
        manifest: &PinManifest,
        target: &str,
        archive: &Path,
        binary: &Path,
        environment: BTreeMap<String, String>,
        runner: &mut R,
    ) -> Result<Self, PinError> {
        let pin = manifest
            .targets
            .iter()
            .find(|candidate| candidate.target == target)
            .ok_or_else(|| error("unsupported_beads_platform"))?;
        if archive.file_name().and_then(|name| name.to_str()) != Some(&pin.archive)
            || checksum(archive)? != pin.archive_sha256
            || checksum(binary)? != pin.binary_sha256
        {
            return Err(error("beads_checksum_mismatch"));
        }
        let output = runner
            .run(CommandSpec {
                program: binary.to_path_buf(),
                argv: vec!["--version".into()],
                cwd: None,
                environment,
                redacted_argv_positions: Vec::new(),
            })
            .map_err(|_| error("unsupported_beads_version"))?;
        if output.status != 0 || !valid_version(&output.stdout) {
            return Err(error("unsupported_beads_version"));
        }
        Ok(Self {
            target: target.to_owned(),
        })
    }
}

impl InstalledBeads {
    /// Verifies an installed regular executable without consulting an archive or extraction tree.
    pub fn verify<R: CommandRunner>(
        manifest: &PinManifest,
        target: &str,
        binary: &Path,
        environment: BTreeMap<String, String>,
        runner: &mut R,
    ) -> Result<Self, PinError> {
        let pin = manifest
            .targets
            .iter()
            .find(|candidate| candidate.target == target)
            .ok_or_else(|| error("unsupported_beads_platform"))?;
        let metadata = fs::symlink_metadata(binary).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                error("installed_beads_missing")
            } else {
                error("installed_beads_invalid")
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(error("installed_beads_invalid"));
        }
        if checksum(binary)? != pin.binary_sha256 {
            return Err(error("beads_checksum_mismatch"));
        }
        let output = runner
            .run(CommandSpec {
                program: binary.to_path_buf(),
                argv: vec!["--version".into()],
                cwd: None,
                environment,
                redacted_argv_positions: Vec::new(),
            })
            .map_err(|_| error("unsupported_beads_version"))?;
        if output.status != 0 || !valid_version(&output.stdout) {
            return Err(error("unsupported_beads_version"));
        }
        Ok(Self {
            target: target.to_owned(),
        })
    }
}

fn valid_version(value: &str) -> bool {
    let Some(body) = value.strip_prefix("bd version 1.1.2 (") else {
        return false;
    };
    body.ends_with(")\n")
        && !body[..body.len() - 2].is_empty()
        && !body[..body.len() - 2].contains('\n')
}
