//! The controller: plasmid manifests in the frozen grammar, the tool registry,
//! the kernel-owned append-only session log, the credential gatekeeper, the
//! plasmid lifecycle FSM, capability version selection, and the desired-state
//! reconciler over named instances, cells and genomes. A caller reaches it over
//! the frozen control protocol on an ndjson connection, which its binary
//! `plasmosomed` serves on the control socket its config names.

pub mod control;
pub mod daemon;
pub mod gatekeeper;
pub mod lifecycle;
pub mod manifest;
pub mod protocol;
pub mod reconciler;
pub mod registry;
pub mod session_log;
pub mod state;
pub mod version;

pub use control::{Controller, Handler, MAX_LINE_BYTES, serve_connection};
pub use daemon::{ConfigError, DaemonConfig, DaemonError, parse_config, run};
pub use gatekeeper::Gatekeeper;
pub use lifecycle::{PluginState, StateError};
pub use manifest::{ManifestError, PlasmidManifest};
pub use protocol::{ErrorCode, Request, Response, StatusResult, WireError};
pub use reconciler::{DesiredState, ObservedState, ReconcilePlan, Reconciler};
pub use registry::{LookupError, RegistryEntry, ToolRegistry};
pub use session_log::{SessionLog, read_events};
pub use state::{
    CellId, CellRecord, CellStatus, ControllerState, GenomeName, InstanceName, InstanceNameError,
    InstanceRecord, MockMode, PlasmidRecord,
};
pub use version::{
    Candidate, ConflictPolicy, Provision, Requirement, SelectionError, Version, VersionReq,
    select_version,
};
