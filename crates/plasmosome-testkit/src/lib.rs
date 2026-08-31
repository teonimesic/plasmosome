//! Shared test support for the kernel crates: builders for the noisy types, and
//! the backend conformance suite every `EnforcementBackend` implementation must
//! pass.
//!
//! Nothing here ships. The crate is `publish = false`, and a rule in
//! `plasmosome-freeze-checks` fails the build if any other workspace crate names
//! it outside `[dev-dependencies]`.

pub mod builders;
pub mod conformance;
