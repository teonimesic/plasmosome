//! The controller brain: plasmid manifests in the frozen grammar, the tool
//! registry, the kernel-owned append-only session log, the credential
//! gatekeeper, the plasmid lifecycle FSM, capability version selection, and
//! the desired-state reconciler placeholder over named instances, cells, and
//! genomes (D1b/D1c).
//!
//! Contract (86 §4 rule 1): this crate must build and test with no dependency
//! path to any VMM, netstack, or broker-process crate — `plasmosome-core` is
//! the controller; VMs, shims, and brokers belong to `plasmosome-membrane`.
//! `plasmosome-freeze-checks` walks `cargo tree -p plasmosome-core` to hold
//! that line. Contract (86 §4 rule 2): controller state crosses processes only
//! as serde types (`state`, `reconciler`, and `manifest` hold no `Arc`/`Rc`
//! and no locks), and desired state is generation-numbered so a replayed
//! reconciler converges instead of re-firing.

pub mod gatekeeper;
pub mod lifecycle;
pub mod manifest;
pub mod reconciler;
pub mod registry;
pub mod session_log;
pub mod state;
pub mod version;

pub use gatekeeper::Gatekeeper;
pub use lifecycle::{PluginState, StateError};
pub use manifest::{ManifestError, PlasmidManifest};
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
