use std::collections::{BTreeMap, btree_map};
use std::fmt;

use serde::de::Error as _;
use serde::ser::{SerializeSeq, SerializeStruct};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::{Uuid, Variant, Version};

use crate::backend::{BackendError, Capability};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrantId(Uuid);

impl GrantId {
    /// Creates a fresh probabilistically unique grant identity.
    pub fn new() -> GrantId {
        GrantId(Uuid::new_v4())
    }
}

impl Default for GrantId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for GrantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.hyphenated())
    }
}

impl Serialize for GrantId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for GrantId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let parsed = Uuid::parse_str(&value).map_err(D::Error::custom)?;
        if parsed.is_nil() {
            return Err(D::Error::custom("grant identity must not be nil"));
        }
        if parsed.get_variant() != Variant::RFC4122 || parsed.get_version() != Some(Version::Random)
        {
            return Err(D::Error::custom(
                "grant identity must be an RFC 4122 UUID v4 value",
            ));
        }
        if value != parsed.hyphenated().to_string() {
            return Err(D::Error::custom(
                "grant identity must be a canonical lower-case hyphenated UUID",
            ));
        }
        Ok(GrantId(parsed))
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
#[serde(deny_unknown_fields)]
pub struct OsObject {
    pub id: GrantId,
    pub owner: PluginId,
    pub capability: Capability,
}

impl OsObject {
    pub fn class(&self) -> UniverseClass {
        self.capability.class()
    }

    pub fn key(&self) -> String {
        self.capability.key()
    }

    pub fn describe(&self) -> String {
        format!(
            "{} `{}` grant {} owned by `{}` with {:?}",
            self.class().as_str(),
            self.key(),
            self.id,
            self.owner,
            self.capability
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsState {
    objects: BTreeMap<(UniverseClass, GrantId), OsObject>,
}

impl OsState {
    pub fn new() -> OsState {
        OsState::default()
    }

    pub fn insert(&mut self, object: OsObject) -> Result<bool, BackendError> {
        let address = (object.class(), object.id);
        match self.objects.entry(address) {
            btree_map::Entry::Vacant(slot) => {
                slot.insert(object);
                Ok(true)
            }
            btree_map::Entry::Occupied(slot) if slot.get() == &object => Ok(false),
            btree_map::Entry::Occupied(_) => Err(BackendError::IdentityConflict {
                class: address.0.as_str(),
                id: address.1,
            }),
        }
    }

    /// Takes only the object matching the exact removal, capability, and owner.
    pub fn remove(&mut self, removal: &UniverseRemoval, owner: &PluginId) -> Option<OsObject> {
        let address = (removal.class(), removal.id);
        let matches = self.objects.get(&address).is_some_and(|object| {
            object.owner == *owner && object.capability == removal.capability
        });
        matches.then(|| {
            self.objects
                .remove(&address)
                .expect("the exact object was observed before removal")
        })
    }

    pub fn contains(&self, class: UniverseClass, key: &str) -> bool {
        self.objects
            .values()
            .any(|object| object.class() == class && object.key() == key)
    }

    pub fn objects(&self) -> impl Iterator<Item = &OsObject> {
        self.objects.values()
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Compares owner, complete capability, and multiplicity while ignoring grant identities.
    pub fn canonically_equivalent(&self, other: &OsState) -> bool {
        self.canonical_multiset() == other.canonical_multiset()
    }

    pub(crate) fn contains_id(&self, id: GrantId) -> bool {
        [
            UniverseClass::SessionFile,
            UniverseClass::UdsPath,
            UniverseClass::ProxyMap,
            UniverseClass::BrokerPid,
            UniverseClass::Mount,
        ]
        .into_iter()
        .any(|class| self.objects.contains_key(&(class, id)))
    }

    fn canonical_multiset(&self) -> BTreeMap<(&PluginId, &Capability), usize> {
        let mut counts = BTreeMap::new();
        for object in self.objects.values() {
            *counts
                .entry((&object.owner, &object.capability))
                .or_default() += 1;
        }
        counts
    }
}

struct Objects<'a>(&'a BTreeMap<(UniverseClass, GrantId), OsObject>);

impl Serialize for Objects<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for object in self.0.values() {
            sequence.serialize_element(object)?;
        }
        sequence.end()
    }
}

impl Serialize for OsState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("OsState", 1)?;
        state.serialize_field("objects", &Objects(&self.objects))?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for OsState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct State {
            objects: Vec<OsObject>,
        }

        let wire = State::deserialize(deserializer)?;
        let mut state = OsState::new();
        for object in wire.objects {
            let address = (object.class(), object.id);
            if state.objects.contains_key(&address) {
                return Err(D::Error::custom(format!(
                    "duplicate grant address {} {}",
                    address.0.as_str(),
                    address.1
                )));
            }
            state
                .insert(object)
                .map_err(|error| D::Error::custom(error.to_string()))?;
        }
        Ok(state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diff {
    pub added: Vec<OsObject>,
    pub removed: Vec<OsObject>,
}

impl Diff {
    pub fn between(before: &OsState, after: &OsState) -> Diff {
        let added = after
            .objects
            .iter()
            .filter(|(address, object)| before.objects.get(*address) != Some(*object))
            .map(|(_, object)| object.clone())
            .collect();
        let removed = before
            .objects
            .iter()
            .filter(|(address, object)| after.objects.get(*address) != Some(*object))
            .map(|(_, object)| object.clone())
            .collect();
        Diff { added, removed }
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum UniverseOp {
    WriteSessionFile {
        id: GrantId,
        path: String,
        owner: PluginId,
    },
    BindUds {
        id: GrantId,
        path: String,
        owner: PluginId,
    },
    SetProxyMap {
        id: GrantId,
        host: String,
        route: String,
        owner: PluginId,
    },
    SpawnBroker {
        id: GrantId,
        pid: u32,
        name: String,
        owner: PluginId,
    },
    AddMount {
        id: GrantId,
        source: String,
        target: String,
        owner: PluginId,
    },
}

impl UniverseOp {
    pub fn id(&self) -> GrantId {
        match self {
            UniverseOp::WriteSessionFile { id, .. }
            | UniverseOp::BindUds { id, .. }
            | UniverseOp::SetProxyMap { id, .. }
            | UniverseOp::SpawnBroker { id, .. }
            | UniverseOp::AddMount { id, .. } => *id,
        }
    }

    pub fn class(&self) -> UniverseClass {
        match self {
            UniverseOp::WriteSessionFile { .. } => UniverseClass::SessionFile,
            UniverseOp::BindUds { .. } => UniverseClass::UdsPath,
            UniverseOp::SetProxyMap { .. } => UniverseClass::ProxyMap,
            UniverseOp::SpawnBroker { .. } => UniverseClass::BrokerPid,
            UniverseOp::AddMount { .. } => UniverseClass::Mount,
        }
    }

    pub fn object(&self) -> OsObject {
        let owner = match self {
            UniverseOp::WriteSessionFile { owner, .. }
            | UniverseOp::BindUds { owner, .. }
            | UniverseOp::SetProxyMap { owner, .. }
            | UniverseOp::SpawnBroker { owner, .. }
            | UniverseOp::AddMount { owner, .. } => owner.clone(),
        };
        OsObject {
            id: self.id(),
            owner,
            capability: self.capability(),
        }
    }

    pub fn removal(&self) -> UniverseRemoval {
        UniverseRemoval {
            id: self.id(),
            capability: self.capability(),
        }
    }

    fn capability(&self) -> Capability {
        match self {
            UniverseOp::WriteSessionFile { path, .. } => {
                Capability::SessionFile { path: path.clone() }
            }
            UniverseOp::BindUds { path, .. } => Capability::UdsSocket { path: path.clone() },
            UniverseOp::SetProxyMap { host, route, .. } => Capability::ProxyMap {
                host: host.clone(),
                route: route.clone(),
            },
            UniverseOp::SpawnBroker { pid, name, .. } => Capability::Broker {
                pid: *pid,
                name: name.clone(),
            },
            UniverseOp::AddMount { source, target, .. } => Capability::Mount {
                source: source.clone(),
                target: target.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UniverseRemoval {
    pub id: GrantId,
    pub capability: Capability,
}

impl UniverseRemoval {
    pub fn class(&self) -> UniverseClass {
        self.capability.class()
    }

    pub fn key(&self) -> String {
        self.capability.key()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

    fn object(owner: &str, capability: Capability) -> OsObject {
        OsObject {
            id: GrantId::new(),
            owner: PluginId::from(owner),
            capability,
        }
    }

    fn proxy(owner: &str, route: &str) -> OsObject {
        object(
            owner,
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: route.to_string(),
            },
        )
    }

    #[test]
    fn grant_identity_defaults_are_fresh_and_non_rfc_variants_are_refused() {
        assert_ne!(GrantId::default(), GrantId::default());
        for invalid in [
            "00000000-0000-4000-0000-000000000001",
            "00000000-0000-4000-c000-000000000001",
            "00000000-0000-4000-e000-000000000001",
        ] {
            let deserializer =
                serde::de::value::StrDeserializer::<serde::de::value::Error>::new(invalid);
            assert!(GrantId::deserialize(deserializer).is_err());
        }
    }

    #[test]
    fn inserting_one_address_is_idempotent_but_changed_payload_conflicts() {
        let mut state = OsState::new();
        let original = proxy("deploy", "splice");
        assert!(state.insert(original.clone()).unwrap());
        assert!(!state.insert(original.clone()).unwrap());
        let conflicting = OsObject {
            owner: PluginId::from("audit"),
            ..original.clone()
        };
        assert_eq!(
            state.insert(conflicting).unwrap_err(),
            BackendError::IdentityConflict {
                class: "proxy-map",
                id: original.id,
            }
        );
        assert_eq!(state.objects().collect::<Vec<_>>(), vec![&original]);
    }

    #[test]
    fn removal_requires_exact_identity_owner_and_capability() {
        let mut state = OsState::new();
        let held = proxy("deploy", "splice");
        let neighbour = proxy("deploy", "audit-route");
        state.insert(held.clone()).unwrap();
        state.insert(neighbour.clone()).unwrap();

        let wrong_capability = UniverseRemoval {
            id: held.id,
            capability: neighbour.capability.clone(),
        };
        assert!(
            state
                .remove(&wrong_capability, &PluginId::from("deploy"))
                .is_none()
        );
        assert!(
            state
                .remove(&held.removal(), &PluginId::from("audit"))
                .is_none()
        );
        assert_eq!(state.len(), 2);
        assert_eq!(
            state.remove(&held.removal(), &PluginId::from("deploy")),
            Some(held)
        );
        assert_eq!(state.objects().collect::<Vec<_>>(), vec![&neighbour]);
    }

    #[test]
    fn canonical_equivalence_ignores_only_identity_and_keeps_multiplicity() {
        let capability = Capability::Mount {
            source: "/secrets".to_string(),
            target: "/workspace".to_string(),
        };
        let mut left = OsState::new();
        let mut right = OsState::new();
        for _ in 0..2 {
            left.insert(object("workspace", capability.clone()))
                .unwrap();
            right
                .insert(object("workspace", capability.clone()))
                .unwrap();
        }
        assert_ne!(left, right);
        assert!(left.canonically_equivalent(&right));

        let survivor = right.objects().next().cloned().unwrap();
        right
            .remove(&survivor.removal(), &PluginId::from("workspace"))
            .unwrap();
        assert!(!left.canonically_equivalent(&right));
        right.insert(object("audit", capability)).unwrap();
        assert!(!left.canonically_equivalent(&right));
    }

    #[test]
    fn canonical_equivalence_detects_complete_capability_changes() {
        for (left_capability, right_capability) in [
            (
                Capability::Mount {
                    source: "/source-a".to_string(),
                    target: "/workspace".to_string(),
                },
                Capability::Mount {
                    source: "/source-b".to_string(),
                    target: "/workspace".to_string(),
                },
            ),
            (
                Capability::ProxyMap {
                    host: "api.github.com".to_string(),
                    route: "splice".to_string(),
                },
                Capability::ProxyMap {
                    host: "api.github.com".to_string(),
                    route: "audit".to_string(),
                },
            ),
            (
                Capability::Broker {
                    pid: 31337,
                    name: "egressd".to_string(),
                },
                Capability::Broker {
                    pid: 31337,
                    name: "auditd".to_string(),
                },
            ),
        ] {
            let mut left = OsState::new();
            left.insert(object("network", left_capability)).unwrap();
            let mut right = OsState::new();
            right.insert(object("network", right_capability)).unwrap();
            assert!(!left.canonically_equivalent(&right));
        }
    }

    #[test]
    fn exact_diff_reports_address_or_payload_replacement_as_loss_and_leak() {
        let before_object = proxy("deploy", "splice");
        let after_object = OsObject {
            id: GrantId::new(),
            ..before_object.clone()
        };
        let after_id = after_object.id;
        let baseline = object(
            "baseline",
            Capability::SessionFile {
                path: "skills/baseline.md".to_string(),
            },
        );
        let mut before = OsState::new();
        let mut after = OsState::new();
        before.insert(baseline.clone()).unwrap();
        after.insert(baseline.clone()).unwrap();
        before.insert(before_object.clone()).unwrap();
        after.insert(after_object.clone()).unwrap();
        let diff = Diff::between(&before, &after);
        assert_eq!(diff.removed, vec![before_object.clone()]);
        assert_eq!(diff.added, vec![after_object.clone()]);
        assert!(before.canonically_equivalent(&after));

        let mut peers = after.clone();
        peers.insert(before_object.clone()).unwrap();
        let partial = Diff::between(&peers, &after);
        assert_eq!(partial.removed, vec![before_object]);
        assert!(partial.added.is_empty());

        let conflicting_object = OsObject {
            owner: PluginId::from("audit"),
            ..after
                .objects()
                .find(|object| object.id == after_id)
                .unwrap()
                .clone()
        };
        let mut conflicting = OsState::new();
        conflicting.insert(baseline).unwrap();
        conflicting.insert(conflicting_object.clone()).unwrap();
        let diff = Diff::between(&after, &conflicting);
        assert_eq!(diff.removed, vec![after_object]);
        assert_eq!(diff.added, vec![conflicting_object]);
    }

    #[test]
    fn residue_report_names_exact_identified_objects() {
        let leaked = object(
            "github",
            Capability::SessionFile {
                path: "cache/github-tokens".to_string(),
            },
        );
        let mut after = OsState::new();
        after.insert(leaked.clone()).unwrap();
        let report = ResidueReport::from_diff(Diff::between(&OsState::new(), &after), vec![]);
        let ResidueReport::Residue {
            leaked: reported,
            lost,
            assertions,
        } = report
        else {
            panic!("a non-empty diff must produce residue");
        };
        assert_eq!(reported, vec![leaked.clone()]);
        assert!(leaked.describe().contains(&leaked.id.to_string()));
        assert!(lost.is_empty());
        assert!(assertions.is_empty());
    }

    impl OsObject {
        fn removal(&self) -> UniverseRemoval {
            UniverseRemoval {
                id: self.id,
                capability: self.capability.clone(),
            }
        }
    }
}
