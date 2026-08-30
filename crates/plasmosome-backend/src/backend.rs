use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::universe::{OsObject, OsState, PluginId, UniverseOp, UniverseRemoval};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Handle(pub u64);

impl Handle {
    pub fn raw(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "h{}", self.0)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    SessionFile { path: String },
    UdsSocket { path: String },
    ProxyMap { host: String, route: String },
    Broker { pid: u32, name: String },
    Mount { source: String, target: String },
}

impl Capability {
    pub fn class_str(&self) -> &'static str {
        match self {
            Capability::SessionFile { .. } => "session-file",
            Capability::UdsSocket { .. } => "uds-path",
            Capability::ProxyMap { .. } => "proxy-map",
            Capability::Broker { .. } => "broker-pid",
            Capability::Mount { .. } => "mount",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub plugin: PluginId,
    pub capability: Capability,
    pub kind: GrantKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub handle: Handle,
    pub plugin: PluginId,
    pub capability: Capability,
    pub kind: GrantKind,
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
    UnknownHandle { handle: Handle },
    DrainTimedOut { handle: Handle, deadline_ms: u64 },
    UnknownObject { class: &'static str, key: String },
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
            BackendError::UnknownObject { class, key } => {
                write!(f, "no {class} object `{key}` in the verification universe")
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
    fn apply_removal(&mut self, removal: UniverseRemoval) -> Result<(), BackendError>;
    fn plant(&mut self, object: OsObject);
}
