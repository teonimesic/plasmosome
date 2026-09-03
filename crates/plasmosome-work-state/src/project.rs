use serde::Deserialize;

const SCHEMA_VERSION: u32 = 1;
const PROJECT_ID: &str = "plasmosome";
const REMOTE_NAME: &str = "origin";
const GIT_OBSERVATION_URL: &str = "https://github.com/teonimesic/plasmosome.git";
const DOLT_REMOTE_URL: &str = "git+https://github.com/teonimesic/plasmosome.git";
const DATA_REF: &str = "refs/dolt/data";

/// The checksum-bound remote pair that this installed wrapper may observe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectConfig {
    project_id: String,
    remote_name: String,
    git_observation_url: String,
    dolt_remote_url: String,
    data_ref: String,
}

/// A stable refusal raised by an unbound project configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectConfigError;

impl ProjectConfigError {
    /// Returns the stable machine-readable refusal code.
    pub fn code(&self) -> &'static str {
        "invalid_project_config"
    }
}

impl std::fmt::Display for ProjectConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProjectConfigError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProjectConfig {
    schema_version: u32,
    project_id: String,
    remote_name: String,
    git_observation_url: String,
    dolt_remote_url: String,
    data_ref: String,
}

impl ProjectConfig {
    /// Parses one exact, credential-free compiled project binding.
    pub fn parse(source: &str) -> Result<Self, ProjectConfigError> {
        let raw: RawProjectConfig = toml::from_str(source).map_err(|_| ProjectConfigError)?;
        if raw.schema_version != SCHEMA_VERSION
            || raw.project_id != PROJECT_ID
            || raw.remote_name != REMOTE_NAME
            || raw.git_observation_url != GIT_OBSERVATION_URL
            || raw.dolt_remote_url != DOLT_REMOTE_URL
            || raw.data_ref != DATA_REF
        {
            return Err(ProjectConfigError);
        }
        Ok(Self {
            project_id: raw.project_id,
            remote_name: raw.remote_name,
            git_observation_url: raw.git_observation_url,
            dolt_remote_url: raw.dolt_remote_url,
            data_ref: raw.data_ref,
        })
    }

    /// Returns the fixed project identifier.
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// Returns the fixed Dolt remote name.
    pub fn remote_name(&self) -> &str {
        &self.remote_name
    }

    /// Returns the fixed plain-HTTPS URL used only for Git observation.
    pub fn git_observation_url(&self) -> &str {
        &self.git_observation_url
    }

    /// Returns the fixed canonical Dolt transport URL.
    pub fn dolt_remote_url(&self) -> &str {
        &self.dolt_remote_url
    }

    /// Returns the fixed observed Dolt data ref.
    pub fn data_ref(&self) -> &str {
        &self.data_ref
    }
}

/// Loads the project binding compiled into the checksum-bound installed wrapper.
pub fn compiled_project_config() -> Result<ProjectConfig, ProjectConfigError> {
    ProjectConfig::parse(include_str!("../../../tools/work-state-project.toml"))
}
