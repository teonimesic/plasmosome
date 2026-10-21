use std::fmt;
use std::time::Duration;

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::recipe::{
    BrokerLaunch, MountRecipe, ProxyRecipe, RecipeError, SessionFileRecipe, UdsRecipe,
    canonical_path, nul_free,
};
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

/// A capability: its class, its exact fields, and the complete recipe or launch that creates
/// it. A broker carries its name and launch, never a process ID.
///
/// Decoding refuses a missing or unknown field and a positional array, in the capability and in
/// its recipe, and decoding and encoding both refuse any value `validate` refuses. No field has
/// a default. A value built in memory is not checked: call `validate` before acting on it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    SessionFile {
        path: String,
        recipe: SessionFileRecipe,
    },
    UdsSocket {
        path: String,
        recipe: UdsRecipe,
    },
    ProxyMap {
        host: String,
        route: String,
        recipe: ProxyRecipe,
    },
    Broker {
        name: String,
        launch: BrokerLaunch,
    },
    Mount {
        source: String,
        target: String,
        recipe: MountRecipe,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Capability", deny_unknown_fields)]
enum CapabilityShape {
    SessionFile {
        path: String,
        recipe: SessionFileRecipe,
    },
    UdsSocket {
        path: String,
        recipe: UdsRecipe,
    },
    ProxyMap {
        host: String,
        route: String,
        recipe: ProxyRecipe,
    },
    Broker {
        name: String,
        launch: BrokerLaunch,
    },
    Mount {
        source: String,
        target: String,
        recipe: MountRecipe,
    },
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

    /// The diagnostic key, not an address: `session/{path}`, the socket path, the proxy host,
    /// `broker/{name}` or the mount target. Two different capabilities can share a key.
    pub fn key(&self) -> String {
        match self {
            Capability::SessionFile { path, .. } => format!("session/{path}"),
            Capability::UdsSocket { path, .. } => path.clone(),
            Capability::ProxyMap { host, .. } => host.clone(),
            Capability::Broker { name, .. } => format!("broker/{name}"),
            Capability::Mount { target, .. } => target.clone(),
        }
    }

    /// Returns the first rule this capability breaks. Its own strings come first, in field
    /// order: `SessionFile.path`, `UdsSocket.path`, `Mount.source` and `Mount.target` are
    /// canonical absolute paths as `RecipeError` defines them, and `host`, `route` and `name`
    /// are exact selection names, so they only need to be NUL-free. Then the recipe's or the
    /// launch's own `validate` runs.
    pub fn validate(&self) -> Result<(), RecipeError> {
        match self {
            Capability::SessionFile { path, recipe } => session_file_rules(path, recipe),
            Capability::UdsSocket { path, recipe } => uds_socket_rules(path, recipe),
            Capability::ProxyMap {
                host,
                route,
                recipe,
            } => proxy_map_rules(host, route, recipe),
            Capability::Broker { name, launch } => broker_rules(name, launch),
            Capability::Mount {
                source,
                target,
                recipe,
            } => mount_rules(source, target, recipe),
        }
    }
}

pub(crate) fn session_file_rules(
    path: &str,
    recipe: &SessionFileRecipe,
) -> Result<(), RecipeError> {
    canonical_path("path", path)?;
    recipe.validate()
}

pub(crate) fn uds_socket_rules(path: &str, recipe: &UdsRecipe) -> Result<(), RecipeError> {
    canonical_path("path", path)?;
    recipe.validate()
}

pub(crate) fn proxy_map_rules(
    host: &str,
    route: &str,
    recipe: &ProxyRecipe,
) -> Result<(), RecipeError> {
    nul_free("host", host)?;
    nul_free("route", route)?;
    recipe.validate()
}

pub(crate) fn broker_rules(name: &str, launch: &BrokerLaunch) -> Result<(), RecipeError> {
    nul_free("name", name)?;
    launch.validate()
}

pub(crate) fn mount_rules(
    source: &str,
    target: &str,
    recipe: &MountRecipe,
) -> Result<(), RecipeError> {
    canonical_path("source", source)?;
    canonical_path("target", target)?;
    recipe.validate()
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
#[serde(remote = "DrainSpec", deny_unknown_fields)]
struct DrainSpecShape {
    #[serde(deserialize_with = "crate::wire::object")]
    deadline: Duration,
    policy: RevokePolicy,
}

object_serde!(
    Handle through HandleShape,
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

    /// The deadline `membraned` sends with the 4090 `shutdown` when it stops a guest under this
    /// drain (spec 023 §7). It is whole milliseconds, at least 1 ms, and strictly below the
    /// drain's deadline. `None` with `RevokePolicy::Force`, and when under 1 ms would be left:
    /// then no `shutdown` is sent, and the helper is killed at the drain deadline.
    pub fn shutdown_deadline(&self) -> Option<Duration> {
        const MS: Duration = Duration::from_millis(1);
        match self.policy {
            RevokePolicy::Force => None,
            RevokePolicy::Graceful => {
                let left = self.deadline.saturating_sub(MS.max(self.deadline / 10));
                let whole = Duration::new(left.as_secs(), left.subsec_millis() * 1_000_000);
                (whole >= MS).then_some(whole)
            }
        }
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
    /// Creates the holding `operation` describes at the address its caller chose and prepared,
    /// and returns the issued entry: that address, the operation's owner and capability, and
    /// `kind`. The backend never changes or allocates the address. An operation whose
    /// `validate` fails returns `InvalidOperation` before any change. A retry of the same
    /// standing issued operation and kind returns the original entry and changes nothing.
    /// Another owner, capability or kind at an issued address, or a grant at the address of an
    /// object that was applied or planted rather than granted, returns `IdentityConflict` and
    /// changes nothing: a grant never promotes an object into an issued record.
    fn grant(
        &mut self,
        operation: UniverseOp,
        kind: GrantKind,
    ) -> Result<LedgerEntry, BackendError>;
    fn revoke(&mut self, handle: Handle, drain: DrainSpec) -> Result<LedgerEntry, BackendError>;
    fn snapshot_os_state(&self) -> OsState;
    /// Records `op` under the identity and owner it names. An operation whose `validate` fails
    /// returns `InvalidOperation` before any change.
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
    /// Records an observed object, such as residue, under its own identity. An object whose
    /// capability fails `validate` returns `InvalidOperation` before any change.
    fn plant(&mut self, object: OsObject) -> Result<(), BackendError>;
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::fixtures::{self, broker, mount, proxy_map, session_file, uds_socket};
    use crate::recipe::{
        BrokerLaunch, FileAccess, MAX_SESSION_FILE_BYTES, ProxyRecipe, ProxyTransport, RecipeError,
        SessionFileRecipe, UdsRecipe,
    };
    use crate::universe::{CellId, PluginId};

    const ID: &str = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

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
            cases.push((session_file(&path), error.clone()));
            cases.push((uds_socket(&path), error));
        }
        for (source, error) in path_faults("source") {
            cases.push((mount(&source, "/workspace"), error));
        }
        for (target, error) in path_faults("target") {
            cases.push((mount("/srv/repo", &target), error));
        }
        let nul = |field| RecipeError::ContainsNul { field };
        cases.push((proxy_map("api.github\0.com", "splice"), nul("host")));
        cases.push((proxy_map("api.github.com", "spl\0ice"), nul("route")));
        cases.push((broker("egress\0d"), nul("name")));
        cases.push((
            Capability::SessionFile {
                path: "/skills/pr.md".to_string(),
                recipe: SessionFileRecipe {
                    guest_path: "run/session".to_string(),
                    ..fixtures::session_recipe()
                },
            },
            RecipeError::NotAbsolute {
                field: "guest_path",
                value: "run/session".to_string(),
            },
        ));
        cases.push((
            Capability::UdsSocket {
                path: "/run/plasmosome/egressd.uds".to_string(),
                recipe: UdsRecipe {
                    guest_path: "agent.sock".to_string(),
                    ..fixtures::uds_recipe()
                },
            },
            RecipeError::NotAbsolute {
                field: "guest_path",
                value: "agent.sock".to_string(),
            },
        ));
        cases.push((
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
                recipe: ProxyRecipe {
                    port: 0,
                    ..fixtures::proxy_recipe()
                },
            },
            RecipeError::ZeroPort,
        ));
        cases.push((
            Capability::Broker {
                name: "egressd".to_string(),
                launch: BrokerLaunch {
                    command: Vec::new(),
                    ..fixtures::launch()
                },
            },
            RecipeError::EmptyCommand,
        ));
        cases.push((
            Capability::Broker {
                name: "egressd".to_string(),
                launch: BrokerLaunch {
                    data_socket: fixtures::launch().control_socket,
                    ..fixtures::launch()
                },
            },
            RecipeError::SharedEndpoint {
                path: fixtures::launch().control_socket,
            },
        ));
        cases
    }

    fn capability_json(capability: &Capability) -> Value {
        match capability {
            Capability::SessionFile { path, recipe } => json!({"SessionFile": {
                "path": path,
                "recipe": {
                    "contents": recipe.contents,
                    "mode": recipe.mode,
                    "guest_path": recipe.guest_path,
                },
            }}),
            Capability::UdsSocket { path, recipe } => json!({"UdsSocket": {
                "path": path,
                "recipe": {"upstream": recipe.upstream, "guest_path": recipe.guest_path},
            }}),
            Capability::ProxyMap {
                host,
                route,
                recipe,
            } => json!({"ProxyMap": {
                "host": host,
                "route": route,
                "recipe": {
                    "transport": recipe.transport,
                    "destination": recipe.destination,
                    "port": recipe.port,
                    "allow_private": recipe.allow_private,
                },
            }}),
            Capability::Broker { name, launch } => json!({"Broker": {
                "name": name,
                "launch": {
                    "command": launch.command,
                    "control_socket": launch.control_socket,
                    "data_socket": launch.data_socket,
                },
            }}),
            Capability::Mount {
                source,
                target,
                recipe,
            } => json!({"Mount": {
                "source": source,
                "target": target,
                "recipe": {"access": recipe.access},
            }}),
        }
    }

    pub(crate) fn operation(id: GrantId, capability: Capability) -> UniverseOp {
        fixtures::op(id, owner(), capability)
    }

    fn operation_json(id: GrantId, capability: Value) -> Value {
        let (variant, mut fields) = match capability {
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
    fn an_invalid_capability_is_refused_wherever_it_travels() {
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
            assert_refused(
                &op,
                operation_json(id, capability_json(&capability)),
                &expected,
            );
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
            let handle = Handle {
                class: capability.class(),
                id,
            };
            let entry = LedgerEntry {
                handle,
                owner: owner(),
                capability: capability.clone(),
                kind: GrantKind::Hot,
            };
            let entry_json = json!({
                "handle": handle,
                "owner": owner(),
                "capability": capability_json(&capability),
                "kind": "Hot",
            });
            assert_refused(&entry, entry_json, &expected);
        }
    }

    #[test]
    fn validate_reports_the_first_broken_field_in_field_order() {
        for (capability, first) in [
            (
                mount("srv/repo", "/workspace/"),
                RecipeError::NotAbsolute {
                    field: "source",
                    value: "srv/repo".to_string(),
                },
            ),
            (
                proxy_map("api.github\0.com", "spl\0ice"),
                RecipeError::ContainsNul { field: "host" },
            ),
            (
                Capability::SessionFile {
                    path: "skills/pr.md".to_string(),
                    recipe: SessionFileRecipe {
                        contents: vec![0; MAX_SESSION_FILE_BYTES + 1],
                        ..fixtures::session_recipe()
                    },
                },
                RecipeError::NotAbsolute {
                    field: "path",
                    value: "skills/pr.md".to_string(),
                },
            ),
            (
                Capability::UdsSocket {
                    path: "run/egressd.uds".to_string(),
                    recipe: UdsRecipe {
                        upstream: "upstream.sock".to_string(),
                        ..fixtures::uds_recipe()
                    },
                },
                RecipeError::NotAbsolute {
                    field: "path",
                    value: "run/egressd.uds".to_string(),
                },
            ),
            (
                Capability::ProxyMap {
                    host: "api.github\0.com".to_string(),
                    route: "splice".to_string(),
                    recipe: ProxyRecipe {
                        port: 0,
                        ..fixtures::proxy_recipe()
                    },
                },
                RecipeError::ContainsNul { field: "host" },
            ),
            (
                Capability::ProxyMap {
                    host: "api.github.com".to_string(),
                    route: "spl\0ice".to_string(),
                    recipe: ProxyRecipe {
                        port: 0,
                        ..fixtures::proxy_recipe()
                    },
                },
                RecipeError::ContainsNul { field: "route" },
            ),
            (
                Capability::Broker {
                    name: "egress\0d".to_string(),
                    launch: BrokerLaunch {
                        command: Vec::new(),
                        ..fixtures::launch()
                    },
                },
                RecipeError::ContainsNul { field: "name" },
            ),
        ] {
            assert_eq!(capability.validate(), Err(first.clone()), "{capability:?}");
            assert_eq!(operation(GrantId::new(), capability).validate(), Err(first));
        }
    }

    #[test]
    fn decoding_names_a_broken_recipe_before_its_capabilitys_own_strings() {
        for (capability, decoded_first, validated_first) in [
            (
                Capability::SessionFile {
                    path: "skills/pr.md".to_string(),
                    recipe: SessionFileRecipe {
                        guest_path: "run/session".to_string(),
                        ..fixtures::session_recipe()
                    },
                },
                RecipeError::NotAbsolute {
                    field: "guest_path",
                    value: "run/session".to_string(),
                },
                RecipeError::NotAbsolute {
                    field: "path",
                    value: "skills/pr.md".to_string(),
                },
            ),
            (
                Capability::ProxyMap {
                    host: "api.github\0.com".to_string(),
                    route: "splice".to_string(),
                    recipe: ProxyRecipe {
                        port: 0,
                        ..fixtures::proxy_recipe()
                    },
                },
                RecipeError::ZeroPort,
                RecipeError::ContainsNul { field: "host" },
            ),
        ] {
            assert_eq!(capability.validate(), Err(validated_first));
            let decoded = serde_json::from_value::<Capability>(capability_json(&capability));
            assert_eq!(
                decoded.unwrap_err().to_string(),
                decoded_first.to_string(),
                "{capability:?}"
            );
        }
    }

    #[test]
    fn selection_names_need_only_be_nul_free_and_valid_values_round_trip() {
        for capability in [
            session_file("/skills/.pr..md"),
            uds_socket("/run/plasmosome/egressd.uds"),
            proxy_map("not a dns name", "../relative/route/"),
            broker("./egressd"),
            mount("/srv/repo", "/workspace"),
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
            assert_eq!(encoded, operation_json(id, capability_json(&capability)));
            assert_eq!(serde_json::from_value::<UniverseOp>(encoded).unwrap(), op);
        }
    }

    #[test]
    fn each_capability_and_its_operation_encode_as_the_designed_json_text() {
        let id: GrantId = serde_json::from_str(&format!("\"{ID}\"")).unwrap();
        let owner_text = r#"{"cell":"cell-1","plugin":"github-pr"}"#;
        for (capability, variant, text) in [
            (
                Capability::SessionFile {
                    path: "/skills/pr.md".to_string(),
                    recipe: SessionFileRecipe {
                        contents: b"hi".to_vec(),
                        mode: FileAccess::ReadWrite,
                        guest_path: "/etc/plasmosome/seed".to_string(),
                    },
                },
                "WriteSessionFile",
                r#"{"SessionFile":{"path":"/skills/pr.md","recipe":{"contents":[104,105],"mode":"read_write","guest_path":"/etc/plasmosome/seed"}}}"#,
            ),
            (
                Capability::UdsSocket {
                    path: "/run/plasmosome/egressd.uds".to_string(),
                    recipe: UdsRecipe {
                        upstream: "/run/plasmosome/upstream.sock".to_string(),
                        guest_path: "/run/agent.sock".to_string(),
                    },
                },
                "BindUds",
                r#"{"UdsSocket":{"path":"/run/plasmosome/egressd.uds","recipe":{"upstream":"/run/plasmosome/upstream.sock","guest_path":"/run/agent.sock"}}}"#,
            ),
            (
                Capability::ProxyMap {
                    host: "api.github.com".to_string(),
                    route: "splice".to_string(),
                    recipe: ProxyRecipe {
                        transport: ProxyTransport::Tcp,
                        destination: "api.github.com".to_string(),
                        port: 443,
                        allow_private: false,
                    },
                },
                "SetProxyMap",
                r#"{"ProxyMap":{"host":"api.github.com","route":"splice","recipe":{"transport":"tcp","destination":"api.github.com","port":443,"allow_private":false}}}"#,
            ),
            (
                Capability::Broker {
                    name: "egressd".to_string(),
                    launch: BrokerLaunch {
                        command: vec!["/usr/libexec/egressd".to_string(), "--serve".to_string()],
                        control_socket: "/run/plasmosome/egressd.control".to_string(),
                        data_socket: "/run/plasmosome/egressd.data".to_string(),
                    },
                },
                "SpawnBroker",
                r#"{"Broker":{"name":"egressd","launch":{"command":["/usr/libexec/egressd","--serve"],"control_socket":"/run/plasmosome/egressd.control","data_socket":"/run/plasmosome/egressd.data"}}}"#,
            ),
            (
                mount("/srv/repo", "/workspace"),
                "AddMount",
                r#"{"Mount":{"source":"/srv/repo","target":"/workspace","recipe":{"access":"read_only"}}}"#,
            ),
        ] {
            assert_eq!(serde_json::to_string(&capability).unwrap(), text);
            assert_eq!(
                serde_json::from_str::<Capability>(text).unwrap(),
                capability
            );
            let fields = &text[text.find(":{").unwrap() + 2..text.len() - 2];
            let operation_text =
                format!(r#"{{"{variant}":{{"id":"{ID}",{fields},"owner":{owner_text}}}}}"#);
            let op = operation(id, capability);
            assert_eq!(serde_json::to_string(&op).unwrap(), operation_text);
            assert_eq!(
                serde_json::from_str::<UniverseOp>(&operation_text).unwrap(),
                op
            );
        }
    }

    fn edited(
        capability: &Capability,
        edit: impl FnOnce(&mut serde_json::Map<String, Value>),
    ) -> Value {
        let mut encoded = capability_json(capability);
        edit(
            encoded
                .as_object_mut()
                .and_then(|outer| outer.values_mut().next())
                .and_then(Value::as_object_mut)
                .expect("a capability encodes its fields as an object"),
        );
        encoded
    }

    fn inside(
        record: &str,
        edit: impl FnOnce(&mut serde_json::Map<String, Value>),
    ) -> impl FnOnce(&mut serde_json::Map<String, Value>) {
        move |fields| edit(fields[record].as_object_mut().expect("a nested object"))
    }

    #[test]
    fn decoding_refuses_a_missing_misshapen_or_pid_bearing_recipe_or_launch() {
        let egressd = broker("egressd");
        let proxy = proxy_map("api.github.com", "splice");
        let mut rows = Vec::new();
        for (capability, field) in [
            (session_file("/skills/pr.md"), "recipe"),
            (uds_socket("/run/plasmosome/egressd.uds"), "recipe"),
            (proxy.clone(), "recipe"),
            (egressd.clone(), "launch"),
            (mount("/srv/repo", "/workspace"), "recipe"),
        ] {
            let missing = edited(&capability, |fields| {
                fields.remove(field);
            });
            rows.push((missing, format!("missing field `{field}`")));
        }
        let pid = |fields: &mut serde_json::Map<String, Value>| {
            fields.insert("pid".to_string(), json!(31337));
        };
        rows.push((edited(&egressd, pid), "unknown field `pid`".to_string()));
        rows.push((
            edited(&egressd, |fields| {
                fields.remove("launch");
                pid(fields);
            }),
            "unknown field `pid`".to_string(),
        ));
        rows.push((
            edited(
                &egressd,
                inside("launch", |launch| {
                    launch.remove("data_socket");
                }),
            ),
            "missing field `data_socket`".to_string(),
        ));
        rows.push((
            edited(
                &proxy,
                inside("recipe", |recipe| {
                    recipe.insert("extra".to_string(), json!(true));
                }),
            ),
            "unknown field `extra`".to_string(),
        ));
        rows.push((
            edited(&mount("/srv/repo", "/workspace"), |fields| {
                fields["recipe"] = json!(["read_only"]);
            }),
            "invalid type: sequence".to_string(),
        ));
        for (encoded, expected) in rows {
            let operation = operation_json(GrantId::new(), encoded.clone());
            for outcome in [
                serde_json::from_value::<Capability>(encoded.clone()).map(drop),
                serde_json::from_str::<Capability>(&encoded.to_string()).map(drop),
                serde_json::from_value::<UniverseOp>(operation.clone()).map(drop),
                serde_json::from_str::<UniverseOp>(&operation.to_string()).map(drop),
            ] {
                let error = outcome
                    .expect_err(&format!("{encoded} must be refused"))
                    .to_string();
                assert!(
                    error.contains(&expected),
                    "{encoded} gave `{error}`, not `{expected}`"
                );
            }
        }
    }

    #[test]
    fn a_broker_is_keyed_by_its_name_and_compared_with_its_whole_launch() {
        let first = broker("egressd");
        let relaunched = Capability::Broker {
            name: "egressd".to_string(),
            launch: BrokerLaunch {
                command: vec![
                    "/usr/libexec/plasmosome-broker".to_string(),
                    "--audit".to_string(),
                ],
                ..fixtures::launch()
            },
        };
        assert_eq!(first.key(), "broker/egressd");
        assert_eq!(relaunched.key(), first.key());
        assert_ne!(relaunched, first);
        assert_eq!(relaunched.class(), UniverseClass::BrokerPid);
        assert_eq!(broker("auditd").key(), "broker/auditd");
    }

    #[test]
    fn each_capability_is_keyed_as_its_doc_says() {
        assert_eq!(session_file("/skills/pr.md").key(), "session//skills/pr.md");
        assert_eq!(
            uds_socket("/run/plasmosome/egressd.uds").key(),
            "/run/plasmosome/egressd.uds"
        );
        assert_eq!(
            proxy_map("api.github.com", "splice").key(),
            "api.github.com"
        );
        assert_eq!(broker("egressd").key(), "broker/egressd");
        assert_eq!(mount("/srv/repo", "/workspace").key(), "/workspace");
    }

    fn ms(count: u64) -> Duration {
        Duration::from_millis(count)
    }

    #[test]
    fn a_shutdown_deadline_stays_below_its_drain_deadline() {
        let graceful = |deadline| DrainSpec::graceful(deadline).shutdown_deadline();
        assert_eq!(graceful(Duration::ZERO), None);
        assert_eq!(graceful(ms(1)), None);
        assert_eq!(graceful(Duration::from_micros(1_999)), None);
        assert_eq!(graceful(ms(2)), Some(ms(1)));
        assert_eq!(graceful(ms(3)), Some(ms(2)));
        assert_eq!(graceful(ms(10)), Some(ms(9)));
        assert_eq!(graceful(ms(20)), Some(ms(18)));
        assert_eq!(graceful(Duration::from_micros(10_700)), Some(ms(9)));
        assert_eq!(graceful(ms(9_000)), Some(ms(8_100)));
        assert_eq!(DrainSpec::forcing().shutdown_deadline(), None);
        assert_eq!(
            DrainSpec {
                deadline: ms(30_000),
                policy: RevokePolicy::Force,
            }
            .shutdown_deadline(),
            None
        );
    }

    proptest::proptest! {
        #[test]
        fn a_shutdown_deadline_is_whole_milliseconds_a_tenth_below_the_drain(
            nanos in 0u64..=200_000_000_000,
        ) {
            let deadline = Duration::from_nanos(nanos);
            let margin = ms(1).max(deadline / 10);
            match DrainSpec::graceful(deadline).shutdown_deadline() {
                Some(shutdown) => {
                    proptest::prop_assert!(shutdown >= ms(1));
                    proptest::prop_assert!(shutdown.subsec_nanos().is_multiple_of(1_000_000));
                    proptest::prop_assert!(shutdown < deadline);
                    proptest::prop_assert!(deadline - shutdown >= margin);
                    proptest::prop_assert!(deadline - shutdown < margin + ms(1));
                }
                None => proptest::prop_assert!(deadline < ms(2)),
            }
            let forced = DrainSpec {
                deadline,
                policy: RevokePolicy::Force,
            };
            proptest::prop_assert_eq!(forced.shutdown_deadline(), None);
        }
    }
}
