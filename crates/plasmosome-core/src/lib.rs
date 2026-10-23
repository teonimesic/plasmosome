//! The controller: plasmid manifests in the frozen grammar, the tool registry,
//! the kernel-owned append-only session log, the credential gatekeeper, the
//! plasmid lifecycle FSM, and the desired-state reconciler over named
//! instances, cells and genomes. A caller reaches it over the frozen control
//! protocol on an ndjson connection, which its binary `plasmosomed` serves on
//! the control socket its config names.

/// The controller's check of one spec 001 §4.2 Artifact before it allocates a cell (spec 023 §1).
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod artifact;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod clock;
pub mod closure;
pub mod control;
pub mod daemon;
pub mod gatekeeper;
pub mod instance;
pub mod kill_budget;
pub mod lifecycle;
pub mod manifest;
/// The no-follow walk: opens a directory or a file one component at a time from "/", never
/// following a symlink, judges every directory it passes by spec 001 §4.1's ancestor rule, and
/// hands the object it reaches to the caller's judge.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod nofollow;
/// Spec 022's operator declarations: the trusted files the controller reads them from.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod operator;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod private_socket;
pub mod protocol;
pub mod reconciler;
pub mod registry;
/// The hardware runtime's launch input, shared by the controller and the
/// cell supervisor: the strictly parsed `RuntimeRecipe` and spec 023's
/// template, canonical paths, cell IDs, the supervisor's config record and a
/// JSON reader that refuses repeated keys.
pub mod runtime;
pub mod session_log;
pub mod state;
pub mod supervisor_check;
/// Spec 023 §4's start: the controller runs one cell's `membraned` detached, with only
/// descriptors 0 to 4, and writes it the config record.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod supervisor_start;
mod toml_depth;
pub mod version;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use clock::{ClockError, SystemClock, boot_session};
pub use control::{Controller, Handler, MAX_LINE_BYTES, serve_connection};
pub use daemon::{
    CellRuntime, CellRuntimeError, ConfigError, DaemonConfig, DaemonError, parse_config, run,
};
pub use gatekeeper::Gatekeeper;
pub use instance::{
    AllocateError, CellDir, CellDirError, DiscoveredEntry, Discovery, DiscoveryError, EntryClass,
    FileIdentity, InstanceRoot, InstanceRootError, JournalAppend, JournalOpen, JournalRefusal,
    LaunchedMarker, LockError, LockProbe, LockTest, MarkerFault, MarkerRead, NotACell, RetireError,
    SupervisorLock, WriterLock,
};
pub use kill_budget::{
    GuestStop, Incomplete, KillBudget, Share, Shortfall, Withdrawal, WorkloadStop,
};
pub use lifecycle::{PluginState, StateError};
pub use manifest::{MAX_MANIFEST_BYTES, MAX_MANIFEST_DEPTH, ManifestError, PlasmidManifest};
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use private_socket::{
    Accepted, DirIdentity, PrivateDir, PrivateListener, PrivateSocketError, SocketEntry,
    check_peer_uid, check_private_path, effective_uid, peer_uid,
};
pub use protocol::{ErrorCode, Request, Response, StatusResult, WireError};
pub use reconciler::{DesiredState, ObservedState, ReconcilePlan, Reconciler};
pub use registry::{LookupError, RegisterError, RegistryEntry, ToolRegistry};
pub use session_log::{
    LogFault, LogFile, LogStep, LogStore, OsLogStore, SessionLog, SessionLogError, read_events,
};
pub use state::{
    CellPathError, CellRecord, CellStatus, ControllerState, GenomeName, InstanceName,
    InstanceNameError, InstanceRecord, PlasmidRecord, cell_ledger_path,
    cell_supervisor_socket_path, validate_cell_id,
};
pub use version::{
    Candidate, ConflictPolicy, Provision, Requirement, SelectionError, Version, VersionReq,
    select_version,
};
