use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InstanceName(String);

impl InstanceName {
    pub fn parse(text: &str) -> Result<InstanceName, InstanceNameError> {
        if text.is_empty() {
            return Err(InstanceNameError::Empty);
        }
        if text.contains('/')
            || text.contains('\\')
            || text == "."
            || text == ".."
            || text.contains('\0')
        {
            return Err(InstanceNameError::NotAName(text.to_string()));
        }
        Ok(InstanceName(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn registry_path(&self) -> String {
        format!("instances/{}", self.0)
    }
}

impl fmt::Display for InstanceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for InstanceName {
    fn from(value: &str) -> Self {
        InstanceName(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceNameError {
    Empty,
    NotAName(String),
}

impl fmt::Display for InstanceNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstanceNameError::Empty => write!(f, "an instance name must not be empty"),
            InstanceNameError::NotAName(text) => {
                write!(
                    f,
                    "`{text}` is not a valid instance name (no path separators, `.`, or `..`)"
                )
            }
        }
    }
}

impl std::error::Error for InstanceNameError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MockMode {
    Simulate,
    Capture,
    #[default]
    Passthrough,
}

impl MockMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            MockMode::Simulate => "simulate",
            MockMode::Capture => "capture",
            MockMode::Passthrough => "passthrough",
        }
    }

    pub fn parse(text: &str) -> Option<MockMode> {
        match text {
            "simulate" => Some(MockMode::Simulate),
            "capture" => Some(MockMode::Capture),
            "passthrough" => Some(MockMode::Passthrough),
            _ => None,
        }
    }

    pub fn list_tag(&self) -> String {
        match self {
            MockMode::Simulate | MockMode::Capture => format!("[mock:{}]", self.as_str()),
            MockMode::Passthrough => "[real]".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlasmidRecord {
    pub plasmid: String,
    pub mock: MockMode,
}

impl PlasmidRecord {
    pub fn list_label(&self) -> String {
        format!("{} {}", self.plasmid, self.mock.list_tag())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CellId(String);

impl From<&str> for CellId {
    fn from(value: &str) -> Self {
        CellId(value.to_string())
    }
}

impl fmt::Display for CellId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CellStatus {
    Germinating,
    Ready,
    Draining,
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenomeName(String);

impl From<&str> for GenomeName {
    fn from(value: &str) -> Self {
        GenomeName(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellRecord {
    pub id: CellId,
    pub genome: Option<GenomeName>,
    pub status: CellStatus,
    pub plasmids: Vec<PlasmidRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceRecord {
    pub name: InstanceName,
    pub cells: Vec<CellRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ControllerState {
    pub instances: Vec<InstanceRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_names_reject_path_shaped_text() {
        assert_eq!(InstanceName::parse("work"), Ok(InstanceName::from("work")));
        assert_eq!(InstanceName::parse(""), Err(InstanceNameError::Empty));
        assert_eq!(
            InstanceName::parse("a/b"),
            Err(InstanceNameError::NotAName("a/b".to_string()))
        );
        assert_eq!(
            InstanceName::parse(".."),
            Err(InstanceNameError::NotAName("..".to_string()))
        );
        assert_eq!(
            InstanceName::parse("."),
            Err(InstanceNameError::NotAName(".".to_string()))
        );
    }

    #[test]
    fn an_instance_name_resolves_to_its_registry_path() {
        let name = InstanceName::parse("work").unwrap();
        assert_eq!(name.registry_path(), "instances/work");
    }

    #[test]
    fn bare_mock_means_simulate_and_absent_means_passthrough() {
        assert_eq!(MockMode::default(), MockMode::Passthrough);
        assert_eq!(MockMode::parse("simulate"), Some(MockMode::Simulate));
        assert_eq!(MockMode::parse("capture"), Some(MockMode::Capture));
        assert_eq!(MockMode::parse("passthrough"), Some(MockMode::Passthrough));
        assert_eq!(
            MockMode::parse("recorded"),
            None,
            "the D2 vocabulary is closed"
        );
    }

    #[test]
    fn plasmid_list_labels_show_the_mock_mode() {
        let mocked = PlasmidRecord {
            plasmid: "github-pr".to_string(),
            mock: MockMode::Simulate,
        };
        let real = PlasmidRecord {
            plasmid: "model-provider".to_string(),
            mock: MockMode::Passthrough,
        };
        assert_eq!(mocked.list_label(), "github-pr [mock:simulate]");
        assert_eq!(real.list_label(), "model-provider [real]");
    }

    #[test]
    fn controller_state_round_trips_through_json_with_lowercase_tags() {
        let state = ControllerState {
            instances: vec![InstanceRecord {
                name: InstanceName::from("work"),
                cells: vec![CellRecord {
                    id: CellId::from("cell-1"),
                    genome: Some(GenomeName::from("researcher")),
                    status: CellStatus::Ready,
                    plasmids: vec![PlasmidRecord {
                        plasmid: "github-pr".to_string(),
                        mock: MockMode::Capture,
                    }],
                }],
            }],
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"mock\":\"capture\""), "{json}");
        assert!(json.contains("\"status\":\"ready\""), "{json}");
        assert!(json.contains("\"name\":\"work\""), "{json}");
        let back: ControllerState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn every_controller_state_type_is_wire_serde() {
        fn wire_serde<T: Serialize + serde::de::DeserializeOwned>() {}
        wire_serde::<ControllerState>();
        wire_serde::<InstanceRecord>();
        wire_serde::<InstanceName>();
        wire_serde::<CellRecord>();
        wire_serde::<CellId>();
        wire_serde::<CellStatus>();
        wire_serde::<GenomeName>();
        wire_serde::<PlasmidRecord>();
        wire_serde::<MockMode>();
    }
}
