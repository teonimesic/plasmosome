use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PluginId(String);

impl PluginId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PluginId {
    fn from(value: &str) -> Self {
        PluginId(value.to_string())
    }
}

impl From<String> for PluginId {
    fn from(value: String) -> Self {
        PluginId(value)
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum UniverseClass {
    SessionFile,
    UdsPath,
    ProxyMap,
    BrokerPid,
    Mount,
}

impl UniverseClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            UniverseClass::SessionFile => "session-file",
            UniverseClass::UdsPath => "uds-path",
            UniverseClass::ProxyMap => "proxy-map",
            UniverseClass::BrokerPid => "broker-pid",
            UniverseClass::Mount => "mount",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OsObject {
    pub class: UniverseClass,
    pub key: String,
    pub owner: PluginId,
}

impl OsObject {
    pub fn describe(&self) -> String {
        format!(
            "{} `{}` owned by `{}`",
            self.class.as_str(),
            self.key,
            self.owner
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsState {
    objects: BTreeSet<OsObject>,
}

impl OsState {
    pub fn new() -> OsState {
        OsState::default()
    }

    pub fn insert(&mut self, object: OsObject) -> bool {
        self.objects.insert(object)
    }

    pub fn remove(&mut self, class: UniverseClass, key: &str) -> Option<OsObject> {
        let owner = self.owner_of(class, key)?;
        self.objects.take(&OsObject {
            class,
            key: key.to_string(),
            owner,
        })
    }

    pub fn owner_of(&self, class: UniverseClass, key: &str) -> Option<PluginId> {
        self.objects
            .iter()
            .find(|o| o.class == class && o.key == key)
            .map(|o| o.owner.clone())
    }

    pub fn contains(&self, class: UniverseClass, key: &str) -> bool {
        self.objects
            .iter()
            .any(|o| o.class == class && o.key == key)
    }

    pub fn objects(&self) -> impl Iterator<Item = &OsObject> {
        self.objects.iter()
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    pub added: Vec<OsObject>,
    pub removed: Vec<OsObject>,
}

impl Diff {
    pub fn between(before: &OsState, after: &OsState) -> Diff {
        let added = after
            .objects
            .iter()
            .filter(|o| !before.objects.contains(o))
            .cloned()
            .collect();
        let removed = before
            .objects
            .iter()
            .filter(|o| !after.objects.contains(o))
            .cloned()
            .collect();
        Diff { added, removed }
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UniverseOp {
    WriteSessionFile {
        path: String,
        owner: PluginId,
    },
    BindUds {
        path: String,
        owner: PluginId,
    },
    SetProxyMap {
        host: String,
        route: String,
        owner: PluginId,
    },
    SpawnBroker {
        pid: u32,
        name: String,
        owner: PluginId,
    },
    AddMount {
        source: String,
        target: String,
        owner: PluginId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UniverseRemoval {
    RemoveSessionFile { path: String },
    UnbindUds { path: String },
    RemoveProxyMap { host: String },
    KillBroker { pid: u32 },
    RemoveMount { target: String },
}

impl UniverseOp {
    pub fn object(&self) -> OsObject {
        match self {
            UniverseOp::WriteSessionFile { path, owner } => OsObject {
                class: UniverseClass::SessionFile,
                key: format!("session/{path}"),
                owner: owner.clone(),
            },
            UniverseOp::BindUds { path, owner } => OsObject {
                class: UniverseClass::UdsPath,
                key: path.clone(),
                owner: owner.clone(),
            },
            UniverseOp::SetProxyMap { host, owner, .. } => OsObject {
                class: UniverseClass::ProxyMap,
                key: host.clone(),
                owner: owner.clone(),
            },
            UniverseOp::SpawnBroker { pid, owner, .. } => OsObject {
                class: UniverseClass::BrokerPid,
                key: format!("broker/{pid}"),
                owner: owner.clone(),
            },
            UniverseOp::AddMount { target, owner, .. } => OsObject {
                class: UniverseClass::Mount,
                key: target.clone(),
                owner: owner.clone(),
            },
        }
    }
}

impl UniverseRemoval {
    pub fn class(&self) -> UniverseClass {
        match self {
            UniverseRemoval::RemoveSessionFile { .. } => UniverseClass::SessionFile,
            UniverseRemoval::UnbindUds { .. } => UniverseClass::UdsPath,
            UniverseRemoval::RemoveProxyMap { .. } => UniverseClass::ProxyMap,
            UniverseRemoval::KillBroker { .. } => UniverseClass::BrokerPid,
            UniverseRemoval::RemoveMount { .. } => UniverseClass::Mount,
        }
    }

    pub fn key(&self) -> String {
        match self {
            UniverseRemoval::RemoveSessionFile { path } => format!("session/{path}"),
            UniverseRemoval::UnbindUds { path } => path.clone(),
            UniverseRemoval::RemoveProxyMap { host } => host.clone(),
            UniverseRemoval::KillBroker { pid } => format!("broker/{pid}"),
            UniverseRemoval::RemoveMount { target } => target.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResidueReport {
    Empty,
    Residue {
        leaked: Vec<OsObject>,
        lost: Vec<OsObject>,
        assertions: Vec<String>,
    },
}

impl ResidueReport {
    pub fn from_diff(diff: Diff, assertions: Vec<String>) -> ResidueReport {
        if diff.is_empty() && assertions.is_empty() {
            ResidueReport::Empty
        } else {
            ResidueReport::Residue {
                leaked: diff.added,
                lost: diff.removed,
                assertions,
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, ResidueReport::Empty)
    }
}

impl fmt::Display for ResidueReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResidueReport::Empty => write!(
                f,
                "residue report: EMPTY (no residue in the verification universe)"
            ),
            ResidueReport::Residue {
                leaked,
                lost,
                assertions,
            } => {
                writeln!(
                    f,
                    "residue report: {} named item(s)",
                    leaked.len() + lost.len()
                )?;
                for object in leaked {
                    writeln!(f, "  LEAKED   {}", object.describe())?;
                }
                for object in lost {
                    writeln!(f, "  LOST     {}", object.describe())?;
                }
                for assertion in assertions {
                    writeln!(f, "  ASSERTED (RevokePolicy::Force) {assertion}")?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(class: UniverseClass, key: &str, owner: &str) -> OsObject {
        OsObject {
            class,
            key: key.to_string(),
            owner: PluginId::from(owner),
        }
    }

    #[test]
    fn the_universe_models_all_five_classes() {
        let mut state = OsState::new();
        assert!(state.insert(object(
            UniverseClass::SessionFile,
            "session/skills/pr.md",
            "github-pr"
        )));
        assert!(state.insert(object(
            UniverseClass::UdsPath,
            "/run/ak/egressd.uds",
            "network"
        )));
        assert!(state.insert(object(UniverseClass::ProxyMap, "api.github.com", "github")));
        assert!(state.insert(object(UniverseClass::BrokerPid, "broker/4242", "network")));
        assert!(state.insert(object(UniverseClass::Mount, "/workspace", "workspace-bind")));
        assert_eq!(state.len(), 5);
        assert_eq!(
            state
                .owner_of(UniverseClass::ProxyMap, "api.github.com")
                .as_ref()
                .map(PluginId::as_str),
            Some("github")
        );
    }

    #[test]
    fn diff_reports_additions_and_removals_in_class_order() {
        let before = OsState::new();
        let mut after = OsState::new();
        after.insert(object(UniverseClass::Mount, "/workspace", "workspace-bind"));
        after.insert(object(
            UniverseClass::SessionFile,
            "session/skills/pr.md",
            "github-pr",
        ));
        let diff = Diff::between(&before, &after);
        assert_eq!(diff.added.len(), 2);
        assert_eq!(diff.added[0].class, UniverseClass::SessionFile);
        assert_eq!(diff.added[1].class, UniverseClass::Mount);
        assert!(diff.removed.is_empty());
        assert!(!diff.is_empty());
    }

    #[test]
    fn diff_between_equal_states_is_empty() {
        let mut a = OsState::new();
        let mut b = OsState::new();
        a.insert(object(
            UniverseClass::UdsPath,
            "/run/ak/egressd.uds",
            "network",
        ));
        b.insert(object(
            UniverseClass::UdsPath,
            "/run/ak/egressd.uds",
            "network",
        ));
        assert!(Diff::between(&a, &b).is_empty());
    }

    #[test]
    fn a_change_of_owner_is_both_a_loss_and_a_leak() {
        let mut a = OsState::new();
        let mut b = OsState::new();
        a.insert(object(UniverseClass::ProxyMap, "api.github.com", "github"));
        b.insert(object(
            UniverseClass::ProxyMap,
            "api.github.com",
            "someone-else",
        ));
        let diff = Diff::between(&a, &b);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
    }

    #[test]
    fn removal_takes_the_whole_attributed_object() {
        let mut state = OsState::new();
        state.insert(object(UniverseClass::BrokerPid, "broker/4242", "network"));
        let removed = state.remove(UniverseClass::BrokerPid, "broker/4242");
        assert_eq!(removed.unwrap().owner, PluginId::from("network"));
        assert!(state.is_empty());
        assert!(
            state
                .remove(UniverseClass::BrokerPid, "broker/4242")
                .is_none()
        );
    }

    #[test]
    fn empty_diff_with_no_assertions_is_an_empty_report() {
        let report =
            ResidueReport::from_diff(Diff::between(&OsState::new(), &OsState::new()), vec![]);
        assert_eq!(report, ResidueReport::Empty);
        assert!(report.is_empty());
    }

    #[test]
    fn a_planted_leak_is_named_with_its_owner() {
        let mut leaked_state = OsState::new();
        leaked_state.insert(object(
            UniverseClass::SessionFile,
            "session/cache/github-tokens",
            "github",
        ));
        let report =
            ResidueReport::from_diff(Diff::between(&OsState::new(), &leaked_state), vec![]);
        let ResidueReport::Residue {
            leaked,
            lost,
            assertions,
        } = report
        else {
            panic!("a non-empty diff must never produce an empty report");
        };
        assert_eq!(leaked.len(), 1);
        assert_eq!(leaked[0].owner, PluginId::from("github"));
        assert!(leaked[0].describe().contains("github"));
        assert!(lost.is_empty());
        assert!(assertions.is_empty());
    }

    #[test]
    fn a_force_assertion_is_recorded_alongside_any_residue() {
        let report = ResidueReport::from_diff(
            Diff::between(&OsState::new(), &OsState::new()),
            vec!["operator `stefano` asserted github emission is acceptable".to_string()],
        );
        let is_empty = report.is_empty();
        let ResidueReport::Residue {
            leaked, assertions, ..
        } = report
        else {
            panic!("an assertion is itself reportable residue");
        };
        assert!(leaked.is_empty());
        assert_eq!(assertions.len(), 1);
        assert!(!is_empty);
    }
}
