//! The enforcement seam: the only vocabulary allowed to cross between the
//! controller and whatever enforces capabilities (`Handle`, `Capability`,
//! `GrantKind::{Hot, GenerationBound}`, `DrainSpec`, `OsState`/`Diff`).

pub mod backend;
pub mod composite;
pub mod fake;
pub mod recipe;
pub mod universe;
pub mod wire;

pub use backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, GrantKind, Handle, LedgerEntry,
    RevokePolicy,
};
pub use composite::{CompositeBackend, Leaf};
pub use fake::FakeBackend;
pub use recipe::{
    BrokerLaunch, FileAccess, MAX_SESSION_FILE_BYTES, MountRecipe, ProxyRecipe, ProxyTransport,
    RecipeError, SessionFileRecipe, UdsRecipe,
};
pub use universe::{
    CellId, CellOwner, Diff, GrantId, MockMode, OsObject, OsState, PluginId, ResidueReport,
    UniverseClass, UniverseOp, UniverseRemoval,
};
pub use wire::ObjectOnly;
