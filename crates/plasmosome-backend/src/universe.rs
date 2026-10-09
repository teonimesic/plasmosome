use std::collections::{BTreeMap, btree_map};
use std::fmt;

use serde::de::{Error as _, MapAccess, Visitor};
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

/// The cell a plugin is attached to. It is a bare name with no validation, and it serializes as
/// a bare JSON string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CellId(String);

impl CellId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CellId {
    fn from(value: &str) -> Self {
        CellId(value.to_string())
    }
}

impl From<String> for CellId {
    fn from(value: String) -> Self {
        CellId(value)
    }
}

impl fmt::Display for CellId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a plasmid's calls are served in a cell: `Simulate`, `Capture` or `Passthrough`.
/// `Passthrough`, the default, sends them to the real service. The vocabulary is closed. It
/// serializes as the lower-case name, and decoding refuses any other name, including a
/// capitalized one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MockMode {
    Simulate,
    Capture,
    #[default]
    Passthrough,
}

impl MockMode {
    /// The lower-case wire name: `simulate`, `capture` or `passthrough`.
    pub fn as_str(&self) -> &'static str {
        match self {
            MockMode::Simulate => "simulate",
            MockMode::Capture => "capture",
            MockMode::Passthrough => "passthrough",
        }
    }

    /// Reads exactly `simulate`, `capture` or `passthrough`; any other text, including a
    /// capitalized name, is `None`.
    pub fn parse(text: &str) -> Option<MockMode> {
        match text {
            "simulate" => Some(MockMode::Simulate),
            "capture" => Some(MockMode::Capture),
            "passthrough" => Some(MockMode::Passthrough),
            _ => None,
        }
    }

    /// The status-list tag: `[mock:simulate]`, `[mock:capture]` or `[real]`.
    pub fn list_tag(&self) -> String {
        match self {
            MockMode::Simulate | MockMode::Capture => format!("[mock:{}]", self.as_str()),
            MockMode::Passthrough => "[real]".to_string(),
        }
    }
}

/// The owner of a holding: a plugin as attached to one cell. The same plugin in two cells is two
/// owners, so every ownership comparison must compare both fields. Its JSON is exactly the object
/// `{"cell": ..., "plugin": ...}`; an array, a missing, repeated or unknown field is refused. It
/// displays as both names quoted, `"cell"/"plugin"`, because neither name is validated yet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct CellOwner {
    pub cell: CellId,
    pub plugin: PluginId,
}

impl fmt::Display for CellOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}/{:?}", self.cell.as_str(), self.plugin.as_str())
    }
}

impl<'de> Deserialize<'de> for CellOwner {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<CellOwner, D::Error> {
        deserializer.deserialize_map(CellOwnerVisitor)
    }
}

struct CellOwnerVisitor;

impl<'de> Visitor<'de> for CellOwnerVisitor {
    type Value = CellOwner;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a cell owner object with exactly a cell and a plugin")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<CellOwner, A::Error> {
        let mut cell: Option<CellId> = None;
        let mut plugin: Option<PluginId> = None;
        while let Some(key) = entries.next_key::<String>()? {
            match key.as_str() {
                "cell" if cell.is_some() => return Err(A::Error::duplicate_field("cell")),
                "cell" => cell = Some(entries.next_value()?),
                "plugin" if plugin.is_some() => return Err(A::Error::duplicate_field("plugin")),
                "plugin" => plugin = Some(entries.next_value()?),
                other => return Err(A::Error::unknown_field(other, &["cell", "plugin"])),
            }
        }
        Ok(CellOwner {
            cell: cell.ok_or_else(|| A::Error::missing_field("cell"))?,
            plugin: plugin.ok_or_else(|| A::Error::missing_field("plugin"))?,
        })
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
    pub owner: CellOwner,
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

    /// Takes the object at the removal's exact address when its full capability and its owner,
    /// cell included, both match, and returns it. Returns `None` and changes nothing otherwise.
    pub fn remove(&mut self, removal: &UniverseRemoval, owner: &CellOwner) -> Option<OsObject> {
        let address = (removal.class(), removal.id);
        self.selects(removal, owner).then(|| {
            self.objects
                .remove(&address)
                .expect("the exact object was observed before removal")
        })
    }

    /// Reports whether `remove` with the same arguments would take an object, without changing
    /// anything. A backend calls it to resolve the exact holding before it starts any drain.
    pub fn selects(&self, removal: &UniverseRemoval, owner: &CellOwner) -> bool {
        self.objects
            .get(&(removal.class(), removal.id))
            .is_some_and(|object| object.owner == *owner && object.capability == removal.capability)
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

    fn canonical_multiset(&self) -> BTreeMap<(&CellOwner, &Capability), usize> {
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
        owner: CellOwner,
    },
    BindUds {
        id: GrantId,
        path: String,
        owner: CellOwner,
    },
    SetProxyMap {
        id: GrantId,
        host: String,
        route: String,
        owner: CellOwner,
    },
    SpawnBroker {
        id: GrantId,
        pid: u32,
        name: String,
        owner: CellOwner,
    },
    AddMount {
        id: GrantId,
        source: String,
        target: String,
        owner: CellOwner,
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
            owner: cell_owner("cell-1", owner),
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
            owner: cell_owner("cell-1", "audit"),
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
                .remove(&wrong_capability, &cell_owner("cell-1", "deploy"))
                .is_none()
        );
        assert!(
            state
                .remove(&held.removal(), &cell_owner("cell-1", "audit"))
                .is_none()
        );
        assert_eq!(state.len(), 2);
        assert_eq!(
            state.remove(&held.removal(), &cell_owner("cell-1", "deploy")),
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
            .remove(&survivor.removal(), &cell_owner("cell-1", "workspace"))
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
            owner: cell_owner("cell-1", "audit"),
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

    fn cell_owner(cell: &str, plugin: &str) -> CellOwner {
        CellOwner {
            cell: CellId::from(cell),
            plugin: PluginId::from(plugin),
        }
    }

    #[test]
    fn cell_owner_wire_is_exactly_its_cell_and_plugin() {
        let owner = cell_owner("cell-1", "github-pr");
        assert_eq!(
            serde_json::to_value(&owner).unwrap(),
            serde_json::json!({"cell": "cell-1", "plugin": "github-pr"})
        );
        assert_eq!(
            serde_json::to_value(CellId::from(String::from("cell-1"))).unwrap(),
            serde_json::json!("cell-1")
        );
        assert_eq!(
            serde_json::from_value::<CellOwner>(
                serde_json::json!({"cell": "cell-1", "plugin": "github-pr"})
            )
            .unwrap(),
            owner
        );
        for refused in [
            serde_json::json!({"cell": "cell-1"}),
            serde_json::json!({"plugin": "github-pr"}),
            serde_json::json!({"cell": "cell-1", "plugin": "github-pr", "instance": "work"}),
            serde_json::json!("github-pr"),
            serde_json::json!(["cell-1", "github-pr"]),
            serde_json::json!({"cell": "cell-1", "plugin": 7}),
        ] {
            assert!(
                serde_json::from_value::<CellOwner>(refused.clone()).is_err(),
                "{refused} must not decode as a cell owner"
            );
        }
        for (duplicated, field) in [
            (
                r#"{"cell": "cell-1", "plugin": "github-pr", "cell": "cell-2"}"#,
                "cell",
            ),
            (
                r#"{"cell": "cell-1", "plugin": "github-pr", "plugin": "audit"}"#,
                "plugin",
            ),
        ] {
            assert!(
                serde_json::from_str::<CellOwner>(duplicated)
                    .unwrap_err()
                    .to_string()
                    .contains(&format!("duplicate field `{field}`")),
                "{duplicated} must not decode as a cell owner"
            );
        }
        assert_eq!(owner.cell.as_str(), "cell-1");
        assert_eq!(owner.cell.to_string(), "cell-1");
    }

    #[test]
    fn a_cell_owner_displays_both_names_quoted_so_a_slash_cannot_blur_them() {
        assert_eq!(
            cell_owner("cell-1", "github-pr").to_string(),
            r#""cell-1"/"github-pr""#
        );
        assert_ne!(
            cell_owner("a/b", "c").to_string(),
            cell_owner("a", "b/c").to_string()
        );
        assert_eq!(cell_owner("a\"/\"b", "c").to_string(), r#""a\"/\"b"/"c""#);
    }

    #[test]
    fn mock_mode_vocabulary_is_closed_and_defaults_to_passthrough() {
        assert_eq!(MockMode::default(), MockMode::Passthrough);
        for mode in [MockMode::Simulate, MockMode::Capture, MockMode::Passthrough] {
            let name = match mode {
                MockMode::Simulate => "simulate",
                MockMode::Capture => "capture",
                MockMode::Passthrough => "passthrough",
            };
            assert_eq!(MockMode::parse(name), Some(mode));
            assert_eq!(mode.as_str(), name);
        }
        for refused in [
            "recorded",
            "Simulate",
            "Capture",
            "Passthrough",
            "CAPTURE",
            "",
            " simulate",
            "simulate\n",
        ] {
            assert_eq!(
                MockMode::parse(refused),
                None,
                "{refused:?} is not a mock mode"
            );
        }
    }

    #[test]
    fn mock_mode_serializes_as_lower_case_text() {
        for (mode, wire) in [
            (MockMode::Simulate, r#""simulate""#),
            (MockMode::Capture, r#""capture""#),
            (MockMode::Passthrough, r#""passthrough""#),
        ] {
            assert_eq!(serde_json::to_string(&mode).unwrap(), wire);
            assert_eq!(serde_json::from_str::<MockMode>(wire).unwrap(), mode);
            assert_eq!(mode.as_str(), wire.trim_matches('"'));
        }
    }

    #[test]
    fn mock_mode_refuses_unknown_and_capitalized_text() {
        for refused in [
            r#""Simulate""#,
            r#""Capture""#,
            r#""Passthrough""#,
            r#""recorded""#,
            r#""""#,
            "null",
        ] {
            assert!(
                serde_json::from_str::<MockMode>(refused).is_err(),
                "{refused} must not decode as a mock mode"
            );
        }
    }

    #[test]
    fn mock_mode_decodes_only_its_three_names() {
        let error = serde_json::from_value::<MockMode>(serde_json::json!("none"))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "unknown variant `none`, expected one of `simulate`, `capture`, `passthrough`"
        );
    }

    #[test]
    fn mock_mode_lists_only_passthrough_as_real() {
        assert_eq!(MockMode::Simulate.list_tag(), "[mock:simulate]");
        assert_eq!(MockMode::Capture.list_tag(), "[mock:capture]");
        assert_eq!(MockMode::Passthrough.list_tag(), "[real]");
    }

    #[test]
    fn removal_and_selection_refuse_the_same_plugins_object_in_another_cell() {
        let mut state = OsState::new();
        let held = OsObject {
            id: GrantId::new(),
            owner: cell_owner("cell-1", "deploy"),
            capability: Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
            },
        };
        state.insert(held.clone()).unwrap();
        let elsewhere = cell_owner("cell-2", "deploy");
        assert!(!state.selects(&held.removal(), &elsewhere));
        assert_eq!(state.remove(&held.removal(), &elsewhere), None);
        assert_eq!(state.objects().collect::<Vec<_>>(), vec![&held]);
        assert!(state.selects(&held.removal(), &held.owner));
        assert_eq!(state.remove(&held.removal(), &held.owner), Some(held));
        assert!(state.is_empty());
    }

    #[test]
    fn selection_requires_the_exact_address_capability_and_owner_and_changes_nothing() {
        let mut state = OsState::new();
        let held = OsObject {
            id: GrantId::new(),
            owner: cell_owner("cell-1", "workspace"),
            capability: Capability::Mount {
                source: "/code".to_string(),
                target: "/workspace".to_string(),
            },
        };
        state.insert(held.clone()).unwrap();
        let before = state.clone();
        let other_source = UniverseRemoval {
            id: held.id,
            capability: Capability::Mount {
                source: "/secrets".to_string(),
                target: "/workspace".to_string(),
            },
        };
        let other_id = UniverseRemoval {
            id: GrantId::new(),
            capability: held.capability.clone(),
        };
        assert!(!state.selects(&other_source, &held.owner));
        assert!(!state.selects(&other_id, &held.owner));
        assert!(!state.selects(&held.removal(), &cell_owner("cell-1", "audit")));
        assert!(state.selects(&held.removal(), &held.owner));
        assert_eq!(state, before);
    }

    #[test]
    fn canonical_equivalence_distinguishes_the_same_plugin_in_another_cell() {
        let capability = Capability::SessionFile {
            path: "skills/pr.md".to_string(),
        };
        let mut left = OsState::new();
        left.insert(OsObject {
            id: GrantId::new(),
            owner: cell_owner("cell-1", "github-pr"),
            capability: capability.clone(),
        })
        .unwrap();
        let mut right = OsState::new();
        let moved = OsObject {
            id: GrantId::new(),
            owner: cell_owner("cell-2", "github-pr"),
            capability,
        };
        right.insert(moved.clone()).unwrap();
        assert!(!left.canonically_equivalent(&right));
        assert!(moved.describe().contains(r#""cell-2"/"github-pr""#));
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
