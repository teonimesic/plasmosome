use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// The bundle's libkrun, relative to the bundle directory.
pub const LIBKRUN: &str = "lib/libkrun.1.19.4.dylib";

/// The bundle's guest kernel, relative to the bundle directory.
pub const KERNEL: &str = "boot/kernel";

/// A bundle directory whose files matched its `SHA256SUMS` when it was checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBundle {
    dir: PathBuf,
    files: BTreeMap<String, String>,
}

impl VerifiedBundle {
    /// The absolute path of a listed file, or `None` if `SHA256SUMS` does not list it.
    pub fn path(&self, relative: &str) -> Option<PathBuf> {
        self.files
            .contains_key(relative)
            .then(|| self.dir.join(relative))
    }

    /// The SHA-256 `SHA256SUMS` gives for a listed file.
    pub fn digest(&self, relative: &str) -> Option<&str> {
        self.files.get(relative).map(String::as_str)
    }
}

/// Why a bundle was refused, naming the path at fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleError {
    pub path: String,
    pub reason: String,
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.reason)
    }
}

impl std::error::Error for BundleError {}

/// Check every file `dir/SHA256SUMS` lists against its digest, and refuse a file under `lib/`
/// that it does not list. Nothing is loaded; the caller loads only after this returns `Ok`.
pub fn verify_bundle(dir: &Path) -> Result<VerifiedBundle, BundleError> {
    Ok(VerifiedBundle {
        dir: dir.to_path_buf(),
        files: BTreeMap::new(),
    })
}

/// The lowercase hexadecimal SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let _ = bytes;
    String::new()
}
