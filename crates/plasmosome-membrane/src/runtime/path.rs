use std::ffi::{CStr, CString};
use std::path::Path;

/// An absolute path in canonical form: it starts with `/`, is not `/` itself,
/// holds no NUL byte, has no trailing `/`, no empty component and no `.` or
/// `..` component, and is shorter than `PATH_MAX` bytes.
///
/// Canonical form is about spelling only. Parsing touches no filesystem, so a
/// `RecipePath` may still name a symlink or nothing at all; a caller that opens
/// it must walk its components without following symlinks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecipePath {
    text: String,
    c: CString,
}

/// Why a text is not a canonical absolute path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathFault {
    /// The text is empty.
    Empty,
    /// The text does not start with `/`.
    NotAbsolute,
    /// The text is `/`, which names no file a recipe may select.
    Root,
    /// The text holds a NUL byte.
    Nul,
    /// The text ends with `/`.
    TrailingSlash,
    /// The text holds `//`.
    EmptyComponent,
    /// A component is `.`.
    DotComponent,
    /// A component is `..`.
    DotDotComponent,
    /// The text is `bytes` bytes long; at most `max` are allowed.
    TooLong { bytes: usize, max: usize },
}

impl RecipePath {
    /// Accepts `text` only in canonical form, checking the rules in the order
    /// `PathFault` lists them and reporting the first one broken.
    pub fn parse(_text: &str) -> Result<RecipePath, PathFault> {
        todo!()
    }

    /// The path as a `Path`.
    pub fn as_path(&self) -> &Path {
        todo!()
    }

    /// The path's bytes followed by one terminating NUL, for a system call.
    pub fn as_c_str(&self) -> &CStr {
        todo!()
    }

    /// The components after the leading `/`, in order; never empty, `.` or `..`.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        let parts: std::str::Split<'_, char> = todo!();
        parts
    }

    /// The path without its last component, or `None` when that would be `/`.
    pub fn parent(&self) -> Option<RecipePath> {
        todo!()
    }

    /// The last component.
    pub fn file_name(&self) -> &str {
        todo!()
    }
}

impl std::fmt::Display for PathFault {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for PathFault {}

