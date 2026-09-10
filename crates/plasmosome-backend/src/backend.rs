use std::fmt;
use std::time::Duration;

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::universe::{
    GrantId, OsObject, OsState, PluginId, UniverseClass, UniverseOp, UniverseRemoval,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handle {
    pub class: UniverseClass,
    pub id: GrantId,
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Capability {
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub plugin: PluginId,
    pub capability: Capability,
    pub kind: GrantKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub handle: Handle,
    pub plugin: PluginId,
    pub capability: Capability,
    pub kind: GrantKind,
}

impl LedgerEntry {
    pub fn object(&self) -> OsObject {
        OsObject {
            id: self.handle.id,
            owner: self.plugin.clone(),
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
            plugin: &'a PluginId,
            capability: &'a Capability,
            kind: &'a GrantKind,
        }

        Wire {
            handle: &self.handle,
            plugin: &self.plugin,
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
        #[serde(deny_unknown_fields)]
        struct Wire {
            handle: Handle,
            plugin: PluginId,
            capability: Capability,
            kind: GrantKind,
        }

        let wire = Wire::deserialize(deserializer)?;
        if wire.handle.class != wire.capability.class() {
            return Err(D::Error::custom(
                "ledger entry handle class does not match its capability",
            ));
        }
        Ok(LedgerEntry {
            handle: wire.handle,
            plugin: wire.plugin,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainSpec {
    pub deadline: Duration,
    pub policy: RevokePolicy,
}

impl DrainSpec {
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
        owner: PluginId,
        id: GrantId,
    },
    IdentityConflict {
        class: &'static str,
        id: GrantId,
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
    /// Withdraws only the object matching the exact removal and named owner.
    fn apply_removal(
        &mut self,
        removal: UniverseRemoval,
        owner: &PluginId,
    ) -> Result<(), BackendError>;
    fn plant(&mut self, object: OsObject) -> Result<(), BackendError>;
}
