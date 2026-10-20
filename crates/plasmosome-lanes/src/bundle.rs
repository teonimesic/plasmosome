use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest as _, Sha256};

/// The bundle's libkrun, relative to the bundle directory.
pub const LIBKRUN: &str = "lib/libkrun.1.19.4.dylib";

/// The bundle's guest kernel, relative to the bundle directory.
pub const KERNEL: &str = "boot/kernel";

const SUMS: &str = "SHA256SUMS";

const LOADABLE: &str = "lib";

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

fn refuse<T>(path: &str, reason: impl Into<String>) -> Result<T, BundleError> {
    Err(BundleError {
        path: path.to_string(),
        reason: reason.into(),
    })
}

fn is_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn inside(relative: &str) -> bool {
    let path = Path::new(relative);
    !relative.is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn read_sums(dir: &Path) -> Result<BTreeMap<String, String>, BundleError> {
    let text = fs::read_to_string(dir.join(SUMS))
        .or_else(|error| refuse(SUMS, format!("cannot be read: {error}")))?;
    let mut files = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let Some((digest, relative)) = line.split_once("  ") else {
            return refuse(
                SUMS,
                format!("line {} is not `<sha256>  <path>`", index + 1),
            );
        };
        if !is_digest(digest) {
            return refuse(
                SUMS,
                format!("line {} has no lowercase SHA-256 digest", index + 1),
            );
        }
        if !inside(relative) {
            return refuse(
                relative,
                "is listed in SHA256SUMS but lies outside the bundle",
            );
        }
        if files
            .insert(relative.to_string(), digest.to_string())
            .is_some()
        {
            return refuse(relative, "is listed in SHA256SUMS more than once");
        }
    }
    Ok(files)
}

fn check_file(dir: &Path, relative: &str, digest: &str) -> Result<(), BundleError> {
    let path = dir.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .or_else(|error| refuse(relative, format!("is listed but cannot be read: {error}")))?;
    if !metadata.file_type().is_file() {
        return refuse(relative, "is listed but is not a regular file");
    }
    let bytes = fs::read(&path)
        .or_else(|error| refuse(relative, format!("is listed but cannot be read: {error}")))?;
    let actual = sha256_hex(&bytes);
    if actual != digest {
        return refuse(
            relative,
            format!("has SHA-256 {actual}, but SHA256SUMS lists {digest}"),
        );
    }
    Ok(())
}

fn unlisted_under(
    dir: &Path,
    relative: &str,
    files: &BTreeMap<String, String>,
) -> Result<(), BundleError> {
    let entries = match fs::read_dir(dir.join(relative)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return refuse(relative, format!("cannot be listed: {error}")),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry =
            entry.or_else(|error| refuse(relative, format!("cannot be listed: {error}")))?;
        names.push(entry.file_name().to_string_lossy().to_string());
    }
    names.sort();
    for name in names {
        let child = format!("{relative}/{name}");
        let kind = fs::symlink_metadata(dir.join(&child))
            .or_else(|error| refuse(&child, format!("cannot be read: {error}")))?
            .file_type();
        if kind.is_dir() {
            unlisted_under(dir, &child, files)?;
        } else if !files.contains_key(&child) {
            return refuse(&child, "lies under lib/ and is not listed in SHA256SUMS");
        }
    }
    Ok(())
}

/// Check every file `dir/SHA256SUMS` lists against its digest, and refuse a file under `lib/`
/// that it does not list. Nothing is loaded; the caller loads only after this returns `Ok`.
pub fn verify_bundle(dir: &Path) -> Result<VerifiedBundle, BundleError> {
    let files = read_sums(dir)?;
    for required in [LIBKRUN, KERNEL] {
        if !files.contains_key(required) {
            return refuse(required, "is not listed in SHA256SUMS");
        }
    }
    for (relative, digest) in &files {
        check_file(dir, relative, digest)?;
    }
    unlisted_under(dir, LOADABLE, &files)?;
    Ok(VerifiedBundle {
        dir: dir.to_path_buf(),
        files,
    })
}

/// The lowercase hexadecimal SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
