use std::path::PathBuf;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::state::{InstanceName, InstanceNameError};

/// Everything `plasmosomed` needs to start: the path it answers on, and which
/// named instance it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub control_socket: PathBuf,
    pub name: InstanceName,
}

/// Why a config text is not a config.
#[derive(Debug)]
pub enum ConfigError {
    NotConfig(serde_json::Error),
    NotAnInstanceName(InstanceNameError),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NotConfig(error) => write!(f, "cannot read the config as JSON: {error}"),
            ConfigError::NotAnInstanceName(error) => write!(f, "`name` is unusable: {error}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::NotConfig(error) => Some(error),
            ConfigError::NotAnInstanceName(error) => Some(error),
        }
    }
}

/// Reads a config out of JSON text, or says which part of it is not a config.
///
/// `control_socket` and `name` are both required, and a key the daemon does
/// not know is refused rather than ignored, so a misspelled setting stops the
/// daemon instead of silently not applying. `name` is parsed into an
/// `InstanceName`, so a path-shaped name never reaches the socket layer.
pub fn parse_config(text: &str) -> Result<DaemonConfig, ConfigError> {
    let written: Written = serde_json::from_str(text).map_err(ConfigError::NotConfig)?;
    let name = InstanceName::parse(&written.name).map_err(ConfigError::NotAnInstanceName)?;
    Ok(DaemonConfig {
        control_socket: written.control_socket,
        name,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    #[serde(deserialize_with = "control_socket")]
    control_socket: PathBuf,
    #[serde(deserialize_with = "name")]
    name: String,
}

fn control_socket<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
    PathBuf::deserialize(deserializer)
        .map_err(|error| D::Error::custom(format!("`control_socket` is unusable: {error}")))
}

fn name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    String::deserialize(deserializer)
        .map_err(|error| D::Error::custom(format!("`name` is unusable: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_config_parses_and_each_malformed_config_is_refused_by_name() {
        let parsed = parse_config(r#"{"control_socket": "/tmp/c.uds", "name": "work"}"#)
            .expect("a full config parses");
        assert_eq!(
            parsed,
            DaemonConfig {
                control_socket: PathBuf::from("/tmp/c.uds"),
                name: InstanceName::parse("work").expect("`work` is an instance name"),
            }
        );

        for (text, offender) in [
            (r#"not json"#, "JSON"),
            (r#"{"name": "work"}"#, "control_socket"),
            (r#"{"control_socket": "/tmp/c.uds"}"#, "name"),
            (r#"{"control_socket": 7, "name": "work"}"#, "control_socket"),
            (r#"{"control_socket": "/tmp/c.uds", "name": 7}"#, "name"),
            (
                r#"{"control_socket": "/tmp/c.uds", "name": "work", "socket": "/tmp/x.uds"}"#,
                "socket",
            ),
            (r#"{"control_socket": "/tmp/c.uds", "name": ""}"#, "empty"),
            (r#"{"control_socket": "/tmp/c.uds", "name": "a/b"}"#, "a/b"),
            (r#"{"control_socket": "/tmp/c.uds", "name": ".."}"#, ".."),
        ] {
            let refusal = parse_config(text)
                .err()
                .unwrap_or_else(|| panic!("{text} is refused"));
            assert!(
                refusal.to_string().contains(offender),
                "the refusal of {text} names {offender}, got: {refusal}"
            );
        }
    }
}
