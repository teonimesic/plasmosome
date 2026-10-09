pub mod digest;
pub mod path;
pub mod recipe;
pub(crate) mod strict_json;

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
