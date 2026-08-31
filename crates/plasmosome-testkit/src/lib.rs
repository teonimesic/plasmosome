//! Test support for the kernel crates: builders for the noisy types, and the
//! conformance suite every `EnforcementBackend` implementation must pass. Reach
//! it only from `[dev-dependencies]` — a freeze rule fails the build otherwise.

pub mod builders;
pub mod conformance;
