use std::fmt;
use std::time::Duration;

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::recipe::{RecipeError, canonical_path, nul_free};
use crate::universe::{
    CellOwner, GrantId, OsObject, OsState, UniverseClass, UniverseOp, UniverseRemoval,
};
use crate::wire::{ObjectOnly, object_serde, validated_object_serde};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle {
    pub class: UniverseClass,
    pub id: GrantId,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Handle", deny_unknown_fields)]
struct HandleShape {
    class: UniverseClass,
    id: GrantId,
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.class.as_str(), self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrantKind {
    Hot,
    GenerationBound,
}

impl GrantKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            GrantKind::Hot => "hot",
            GrantKind::GenerationBound => "generation-bound",
        }
    }
}

/// A capability, identified by its class and its exact fields.
///
/// Decoding refuses a missing, unknown or positional field, and decoding and encoding both
/// refuse any value `validate` refuses. A value built in memory is not checked: call `validate`
/// before acting on it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    SessionFile { path: String },
    UdsSocket { path: String },
    ProxyMap { host: String, route: String },
    Broker { pid: u32, name: String },
    Mount { source: String, target: String },
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Capability", deny_unknown_fields)]
enum CapabilityShape {
    SessionFile { path: String },
    UdsSocket { path: String },
    ProxyMap { host: String, route: String },
    Broker { pid: u32, name: String },
    Mount { source: String, target: String },
}

impl Capability {
    pub fn class(&self) -> UniverseClass {
        match self {
            Capability::SessionFile { .. } => UniverseClass::SessionFile,
            Capability::UdsSocket { .. } => UniverseClass::UdsPath,
            Capability::ProxyMap { .. } => UniverseClass::ProxyMap,
            Capability::Broker { .. } => UniverseClass::BrokerPid,
            Capability::Mount { .. } => UniverseClass::Mount,
        }
    }

    pub fn class_str(&self) -> &'static str {
        self.class().as_str()
    }

    pub fn key(&self) -> String {
        match self {
            Capability::SessionFile { path } => format!("session/{path}"),
            Capability::UdsSocket { path } => path.clone(),
            Capability::ProxyMap { host, .. } => host.clone(),
            Capability::Broker { pid, .. } => format!("broker/{pid}"),
            Capability::Mount { target, .. } => target.clone(),
        }
    }

    /// Returns the first rule this capability breaks, in field order. `SessionFile.path`,
    /// `UdsSocket.path`, `Mount.source` and `Mount.target` are canonical absolute paths as
    /// `RecipeError` defines them. `host`, `route` and `name` are exact selection names, so they
    /// only need to be NUL-free.
    pub fn validate(&self) -> Result<(), RecipeError> {
        match self {
            Capability::SessionFile { path } | Capability::UdsSocket { path } => {
                canonical_path("path", path)
            }
            Capability::ProxyMap { host, route } => {
                nul_free("host", host)?;
                nul_free("route", route)
            }
            Capability::Broker { name, .. } => nul_free("name", name),
            Capability::Mount { source, target } => {
                canonical_path("source", source)?;
                canonical_path("target", target)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub owner: CellOwner,
    pub capability: Capability,
    pub kind: GrantKind,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Grant")]
struct GrantShape {
    owner: CellOwner,
    capability: Capability,
    kind: GrantKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub handle: Handle,
    pub owner: CellOwner,
    pub capability: Capability,
    pub kind: GrantKind,
}

impl LedgerEntry {
    pub fn object(&self) -> OsObject {
        OsObject {
            id: self.handle.id,
            owner: self.owner.clone(),
            capability: self.capability.clone(),
        }
    }

    pub fn removal(&self) -> UniverseRemoval {
        UniverseRemoval {
            id: self.handle.id,
            capability: self.capability.clone(),
        }
    }
}

impl Serialize for LedgerEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.handle.class != self.capability.class() {
            return Err(S::Error::custom(
                "ledger entry handle class does not match its capability",
            ));
        }

        #[derive(Serialize)]
        struct Wire<'a> {
            handle: &'a Handle,
            owner: &'a CellOwner,
            capability: &'a Capability,
            kind: &'a GrantKind,
        }

        Wire {
            handle: &self.handle,
            owner: &self.owner,
            capability: &self.capability,
            kind: &self.kind,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LedgerEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(expecting = "struct LedgerEntry", deny_unknown_fields)]
        struct Wire {
            handle: Handle,
            owner: CellOwner,
            capability: Capability,
            kind: GrantKind,
        }

        let wire = Wire::deserialize(ObjectOnly::new(deserializer))?;
        if wire.handle.class != wire.capability.class() {
            return Err(D::Error::custom(
                "ledger entry handle class does not match its capability",
            ));
        }
        Ok(LedgerEntry {
            handle: wire.handle,
            owner: wire.owner,
            capability: wire.capability,
            kind: wire.kind,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevokePolicy {
    Graceful,
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainSpec {
    pub deadline: Duration,
    pub policy: RevokePolicy,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "DrainSpec")]
struct DrainSpecShape {
    #[serde(deserialize_with = "crate::wire::object")]
    deadline: Duration,
    policy: RevokePolicy,
}

object_serde!(
    Handle through HandleShape,
    Grant through GrantShape,
    DrainSpec through DrainSpecShape
);
validated_object_serde!(Capability through CapabilityShape);

impl DrainSpec {
    /// A drain that waits up to `deadline` for the holding's admitted work to finish before it
    /// releases. A zero deadline checks once: a holding that has not drained returns
    /// `DrainTimedOut` and is kept. Zero never means `RevokePolicy::Force`.
    pub fn graceful(deadline: Duration) -> DrainSpec {
        DrainSpec {
            deadline,
            policy: RevokePolicy::Graceful,
        }
    }

    pub fn forcing() -> DrainSpec {
        DrainSpec {
            deadline: Duration::ZERO,
            policy: RevokePolicy::Force,
        }
    }

    /// The deadline in whole milliseconds for `DrainTimedOut`, rounded up, so only a zero
    /// deadline reports 0.
    pub fn deadline_ms(&self) -> u64 {
        u64::try_from(self.deadline.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    UnknownHandle {
        handle: Handle,
    },
    DrainTimedOut {
        handle: Handle,
        deadline_ms: u64,
    },
    UnknownObject {
        class: &'static str,
        key: String,
        owner: CellOwner,
        id: GrantId,
    },
    IdentityConflict {
        class: &'static str,
        id: GrantId,
    },
    InvalidOperation {
        class: &'static str,
        id: GrantId,
        error: RecipeError,
    },
    Fault(String),
    Unimplemented(&'static str),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::UnknownHandle { handle } => write!(f, "unknown handle {handle}"),
            BackendError::DrainTimedOut {
                handle,
                deadline_ms,
            } => {
                write!(
                    f,
                    "handle {handle} did not drain within its {deadline_ms} ms deadline"
                )
            }
            BackendError::UnknownObject {
                class,
                key,
                owner,
                id,
            } => {
                write!(f, "`{owner}` holds no {class} object `{key}` at grant {id}")
            }
            BackendError::IdentityConflict { class, id } => {
                write!(
                    f,
                    "{class} grant identity {id} is already held by another object"
                )
            }
            BackendError::InvalidOperation { class, id, error } => {
                write!(f, "invalid {class} operation {id}: {error}")
            }
            BackendError::Fault(cause) => write!(f, "injected backend fault: {cause}"),
            BackendError::Unimplemented(what) => write!(f, "unimplemented in this track: {what}"),
        }
    }
}

impl std::error::Error for BackendError {}

pub trait EnforcementBackend {
    fn grant(&mut self, grant: Grant) -> LedgerEntry;
    fn revoke(&mut self, handle: Handle, drain: DrainSpec) -> Result<LedgerEntry, BackendError>;
    fn snapshot_os_state(&self) -> OsState;
    fn apply(&mut self, op: UniverseOp) -> Result<(), BackendError>;
    /// Withdraws only the holding at the removal's exact address whose full capability and
    /// cell-qualified owner match. It resolves that holding before draining: a mismatch returns
    /// `UnknownObject` and changes nothing. A graceful drain that cannot finish within its
    /// deadline returns `DrainTimedOut` naming the exact address and keeps the holding, its
    /// issued record and every peer; `RevokePolicy::Force` then withdraws only that holding. A
    /// successful removal of a granted holding, under either policy, also retires its issued
    /// record, so a later `revoke` of its handle returns `UnknownHandle`.
    fn apply_removal(
        &mut self,
        removal: UniverseRemoval,
        owner: &CellOwner,
        drain: DrainSpec,
    ) -> Result<(), BackendError>;
    fn plant(&mut self, object: OsObject) -> Result<(), BackendError>;
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::recipe::RecipeError;
    use crate::universe::{CellId, PluginId};

    pub(crate) fn owner() -> CellOwner {
        CellOwner {
            cell: CellId::from("cell-1"),
            plugin: PluginId::from("github-pr"),
        }
    }

    fn path_faults(field: &'static str) -> Vec<(String, RecipeError)> {
        vec![
            (
                "skills/pr.md".to_string(),
                RecipeError::NotAbsolute {
                    field,
                    value: "skills/pr.md".to_string(),
                },
            ),
            (
                "/skills//pr.md".to_string(),
                RecipeError::NotCanonical {
                    field,
                    value: "/skills//pr.md".to_string(),
                },
            ),
            (
                "/skills/../pr.md".to_string(),
                RecipeError::NotCanonical {
                    field,
                    value: "/skills/../pr.md".to_string(),
                },
            ),
            (
                "/skills/pr\0.md".to_string(),
                RecipeError::ContainsNul { field },
            ),
        ]
    }

    fn invalid_capabilities() -> Vec<(Capability, RecipeError)> {
        let mut cases = Vec::new();
        for (path, error) in path_faults("path") {
            cases.push((
                Capability::SessionFile { path: path.clone() },
                error.clone(),
            ));
            cases.push((Capability::UdsSocket { path }, error));
        }
        for (source, error) in path_faults("source") {
            let target = "/workspace".to_string();
            cases.push((Capability::Mount { source, target }, error));
        }
        for (target, error) in path_faults("target") {
            let source = "/srv/repo".to_string();
            cases.push((Capability::Mount { source, target }, error));
        }
        let nul = |field| RecipeError::ContainsNul { field };
        cases.push((
            Capability::ProxyMap {
                host: "api.github\0.com".to_string(),
                route: "splice".to_string(),
            },
            nul("host"),
        ));
        cases.push((
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "spl\0ice".to_string(),
            },
            nul("route"),
        ));
        cases.push((
            Capability::Broker {
                pid: 4242,
                name: "egress\0d".to_string(),
            },
            nul("name"),
        ));
        cases
    }

    fn capability_json(capability: &Capability) -> Value {
        match capability {
            Capability::SessionFile { path } => json!({"SessionFile": {"path": path}}),
            Capability::UdsSocket { path } => json!({"UdsSocket": {"path": path}}),
            Capability::ProxyMap { host, route } => {
                json!({"ProxyMap": {"host": host, "route": route}})
            }
            Capability::Broker { pid, name } => json!({"Broker": {"pid": pid, "name": name}}),
            Capability::Mount { source, target } => {
                json!({"Mount": {"source": source, "target": target}})
            }
        }
    }

    pub(crate) fn operation(id: GrantId, capability: Capability) -> UniverseOp {
        let owner = owner();
        match capability {
            Capability::SessionFile { path } => UniverseOp::WriteSessionFile { id, path, owner },
            Capability::UdsSocket { path } => UniverseOp::BindUds { id, path, owner },
            Capability::ProxyMap { host, route } => UniverseOp::SetProxyMap {
                id,
                host,
                route,
                owner,
            },
            Capability::Broker { pid, name } => UniverseOp::SpawnBroker {
                id,
                pid,
                name,
                owner,
            },
            Capability::Mount { source, target } => UniverseOp::AddMount {
                id,
                source,
                target,
                owner,
            },
        }
    }

    fn operation_json(id: GrantId, capability: &Capability) -> Value {
        let (variant, mut fields) = match capability_json(capability) {
            Value::Object(outer) => outer.into_iter().next().expect("one variant"),
            other => panic!("a capability encodes as an object, not {other}"),
        };
        let fields = fields.as_object_mut().expect("variant fields");
        fields.insert("id".to_string(), json!(id));
        fields.insert("owner".to_string(), json!(owner()));
        let variant = match variant.as_str() {
            "SessionFile" => "WriteSessionFile",
            "UdsSocket" => "BindUds",
            "ProxyMap" => "SetProxyMap",
            "Broker" => "SpawnBroker",
            "Mount" => "AddMount",
            other => panic!("no operation for {other}"),
        };
        json!({ variant: fields })
    }

    fn assert_refused<T: Serialize + serde::de::DeserializeOwned>(
        value: &T,
        encoded: Value,
        expected: &RecipeError,
    ) {
        let decoded = serde_json::from_value::<T>(encoded.clone()).map(drop);
        let text = serde_json::from_str::<T>(&encoded.to_string()).map(drop);
        let written = serde_json::to_string(value).map(drop);
        for (path, outcome) in [("decode", decoded), ("text", text), ("encode", written)] {
            let error = outcome.expect_err(&format!("{path} must refuse {encoded}"));
            assert!(
                error.to_string().contains(&expected.to_string()),
                "{path} of {encoded} gave `{error}`, not `{expected}`"
            );
        }
    }

    #[test]
    fn a_capability_refuses_nul_and_non_canonical_paths_wherever_it_travels() {
        for (capability, expected) in invalid_capabilities() {
            assert_eq!(
                capability.validate(),
                Err(expected.clone()),
                "{capability:?}"
            );
            assert_refused(&capability, capability_json(&capability), &expected);
            let id = GrantId::new();
            let op = operation(id, capability.clone());
            assert_eq!(op.validate(), Err(expected.clone()), "{op:?}");
            assert_refused(&op, operation_json(id, &capability), &expected);
            let object = OsObject {
                id,
                owner: owner(),
                capability: capability.clone(),
            };
            let object_json =
                json!({"id": id, "owner": owner(), "capability": capability_json(&capability)});
            assert_refused(&object, object_json, &expected);
            let removal = UniverseRemoval {
                id,
                capability: capability.clone(),
            };
            let removal_json = json!({"id": id, "capability": capability_json(&capability)});
            assert_refused(&removal, removal_json, &expected);
        }
    }

    #[test]
    fn validate_reports_the_first_broken_field_in_field_order() {
        for (capability, first) in [
            (
                Capability::Mount {
                    source: "srv/repo".to_string(),
                    target: "/workspace/".to_string(),
                },
                RecipeError::NotAbsolute {
                    field: "source",
                    value: "srv/repo".to_string(),
                },
            ),
            (
                Capability::ProxyMap {
                    host: "api.github\0.com".to_string(),
                    route: "spl\0ice".to_string(),
                },
                RecipeError::ContainsNul { field: "host" },
            ),
        ] {
            assert_eq!(capability.validate(), Err(first.clone()), "{capability:?}");
            assert_eq!(operation(GrantId::new(), capability).validate(), Err(first));
        }
    }

    #[test]
    fn selection_names_need_only_be_nul_free_and_valid_values_round_trip() {
        for capability in [
            Capability::SessionFile {
                path: "/skills/.pr..md".to_string(),
            },
            Capability::UdsSocket {
                path: "/run/plasmosome/egressd.uds".to_string(),
            },
            Capability::ProxyMap {
                host: "not a dns name".to_string(),
                route: "../relative/route/".to_string(),
            },
            Capability::Broker {
                pid: 4242,
                name: "./egressd".to_string(),
            },
            Capability::Mount {
                source: "/srv/repo".to_string(),
                target: "/workspace".to_string(),
            },
        ] {
            assert_eq!(capability.validate(), Ok(()), "{capability:?}");
            let encoded = serde_json::to_value(&capability).unwrap();
            assert_eq!(encoded, capability_json(&capability));
            assert_eq!(
                serde_json::from_value::<Capability>(encoded).unwrap(),
                capability
            );
            let id = GrantId::new();
            let op = operation(id, capability.clone());
            assert_eq!(op.validate(), Ok(()));
            let encoded = serde_json::to_value(&op).unwrap();
            assert_eq!(encoded, operation_json(id, &capability));
            assert_eq!(serde_json::from_value::<UniverseOp>(encoded).unwrap(), op);
        }
    }
}
