//! The enforcement seam: the only vocabulary allowed to cross between the
//! controller and whatever enforces capabilities (`Handle`, `Capability`,
//! `GrantKind::{Hot, GenerationBound}`, `DrainSpec`, `OsState`/`Diff`).
//!
//! It also holds vocabulary that guest, membrane and controller share: the
//! SHA-256 [`Digest`], spec 001's observation pages in [`paging`], spec 027's
//! pinned software trees in [`software`], and spec 028's lifetime values in
//! [`lifetime`] with the [`Clock`] they are read on.

#![doc(test(attr(forbid(unsafe_code))))]

pub mod backend;
pub mod clock;
pub mod composite;
pub mod digest;
pub mod fake;
pub mod lifetime;
pub mod paging;
pub mod recipe;
pub mod software;
pub mod universe;
mod wire;

#[cfg(test)]
mod fixtures;

pub use backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, GrantKind, Handle, LedgerEntry,
    RevokePolicy,
};
pub use clock::{Clock, ClockReading, ManualClock};
pub use composite::{CompositeBackend, Leaf};
pub use digest::{Digest, DigestError};
pub use fake::FakeBackend;
pub use lifetime::{
    BootSession, Bound, Continuous, DEFAULT_DRAIN_MS, DEFAULT_LEASE_MS, MAX_DRAIN_MS, MAX_LIMIT_MS,
    MIN_DRAIN_MS, MIN_LIMIT_MS, REFERENCE_GRACE_MS, RangeError, TimeTextError, WallMs,
    check_drain_ms, check_limit_ms,
};
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
