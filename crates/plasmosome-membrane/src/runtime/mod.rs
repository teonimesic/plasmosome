pub mod artifacts;
pub mod confinement;
pub use plasmosome_backend::digest;
pub mod hello;
pub mod launch;
pub(crate) use plasmosome_core::runtime::strict_json;
pub use plasmosome_core::runtime::{path, recipe};
#[cfg(test)]
pub(crate) mod test_fixtures;

/// The result of removing a path the runtime created, after checking that the
/// path still names the object the runtime made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// The path still named the created object, and it is gone.
    Removed,
    /// Nothing was at the path.
    AlreadyGone,
    /// Another object now occupies the path. It was left in place.
    LeftReplacement,
    /// The removal failed with this errno; the object may remain.
    Failed { errno: i32 },
}
