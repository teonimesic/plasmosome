//! The enforcement seam: the only vocabulary allowed to cross between the
//! controller and whatever enforces capabilities (`Handle`, `Capability`,
//! `GrantKind::{Hot, GenerationBound}`, `DrainSpec`, `OsState`/`Diff`).
//!
//! Contract (86 §4 rule 1): nothing in this crate or anything that depends on
//! it may name a VMM, netstack, or broker process — enforced by
//! `plasmosome-freeze-checks` walking `cargo tree`. Contract (86 §4 rule 2):
//! seam state crosses processes only as serde data, so every wire type derives
//! `Serialize`/`Deserialize` and none of them holds shared memory.

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
