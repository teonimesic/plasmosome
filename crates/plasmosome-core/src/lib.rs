//! The controller brain: plasmid manifests in the frozen grammar, the tool
//! registry, the kernel-owned append-only session log, the credential
//! gatekeeper, the plasmid lifecycle FSM, capability version selection, and
//! the desired-state reconciler placeholder over named instances, cells, and
//! genomes (D1b/D1c). It also answers the frozen control protocol on an ndjson
//! connection: the request and reply envelopes, the closed error table, and
//! `plasmosome.status` built from controller state.
//!
//! This crate is the controller: VMs, shims and brokers belong to
//! `plasmosome-membrane`, and controller state crosses processes as serde
//! types carrying no shared memory. Both are design rules, held in review and
//! written down in `AGENTS.md`, not by a test.

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
