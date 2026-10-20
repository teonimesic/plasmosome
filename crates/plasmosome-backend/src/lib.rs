//! The enforcement seam: the only vocabulary allowed to cross between the
//! controller and whatever enforces capabilities (`Handle`, `Capability`,
//! `GrantKind::{Hot, GenerationBound}`, `DrainSpec`, `OsState`/`Diff`).
//!
//! It also holds vocabulary that guest, membrane and controller share: the
//! SHA-256 [`Digest`], spec 001's observation pages in [`paging`], and spec
//! 027's pinned software trees in [`software`].

pub mod backend;
pub mod composite;
pub mod digest;
pub mod fake;
pub mod paging;
pub mod recipe;
pub mod software;
pub mod universe;
mod wire;

#[cfg(test)]
mod fixtures;

pub use backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, GrantKind, Handle, LedgerEntry,
    RevokePolicy,
};
pub use composite::{CompositeBackend, Leaf};
pub use digest::{Digest, DigestError};
pub use fake::FakeBackend;
pub use paging::{ObservationPage, PageAssembler, PageCursor};
pub use recipe::{
    BrokerLaunch, FileAccess, MAX_SESSION_FILE_BYTES, MountRecipe, ProxyRecipe, ProxyTransport,
    RecipeError, SessionFileRecipe, UdsRecipe,
};
pub use universe::{
    CellId, CellOwner, Diff, GrantId, MockMode, OsObject, OsState, PluginId, ResidueReport,
    UniverseClass, UniverseOp, UniverseRemoval,
};
pub use wire::ObjectOnly;
