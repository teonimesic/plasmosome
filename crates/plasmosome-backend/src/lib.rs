//! The enforcement seam: the only vocabulary allowed to cross between the
//! controller and whatever enforces capabilities (`Handle`, `Capability`,
//! `GrantKind::{Hot, GenerationBound}`, `DrainSpec`, `OsState`/`Diff`).

pub mod backend;
pub mod composite;
pub mod fake;
pub mod universe;

pub use backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, GrantKind, Handle, LedgerEntry,
    RevokePolicy,
};
pub use composite::{CompositeBackend, Leaf};
pub use fake::FakeBackend;
pub use universe::{
    Diff, OsObject, OsState, PluginId, ResidueReport, UniverseClass, UniverseOp, UniverseRemoval,
};
