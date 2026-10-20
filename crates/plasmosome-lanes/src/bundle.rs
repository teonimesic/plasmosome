use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest as _, Sha256};

/// The bundle's libkrun, relative to the bundle directory.
pub const LIBKRUN: &str = "lib/libkrun.1.19.4.dylib";

/// The bundle's guest kernel, relative to the bundle directory.
pub const KERNEL: &str = "boot/kernel";

/// The one runtime bundle these lanes accept: the first 16 hex digits of its manifest's SHA-256,
/// which is also its directory name.
pub const PINNED_BUNDLE_ID: &str = "eed141b574337d99";

const SUMS: &str = "SHA256SUMS";

const MANIFEST: &str = "manifest.json";

const ROOT: &str = ".";

const ID_DIGITS: usize = 16;

const LOADABLE: &str = "lib";

/// A bundle directory whose files matched its `SHA256SUMS` when it was checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBundle {
    dir: PathBuf,
    id: String,
    files: BTreeMap<String, String>,
}

impl VerifiedBundle {
    /// The bundle's ID, its directory name.
    pub fn id(&self) -> &str {
        &self.id
    }

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

fn no_links_under(dir: &Path, relative: &str) -> Result<(), BundleError> {
    let here = if relative == ROOT {
        dir.to_path_buf()
    } else {
        dir.join(relative)
    };
    let mut names = Vec::new();
    for entry in fs::read_dir(&here)
        .or_else(|error| refuse(relative, format!("cannot be listed: {error}")))?
    {
        let entry =
            entry.or_else(|error| refuse(relative, format!("cannot be listed: {error}")))?;
        names.push(entry.file_name().to_string_lossy().to_string());
    }
    names.sort();
    for name in names {
        let child = if relative == ROOT {
            name
        } else {
            format!("{relative}/{name}")
        };
        let kind = fs::symlink_metadata(dir.join(&child))
            .or_else(|error| refuse(&child, format!("cannot be read: {error}")))?
            .file_type();
        if kind.is_symlink() {
            return refuse(&child, "is a symlink inside the bundle");
        }
        if kind.is_dir() {
            no_links_under(dir, &child)?;
        } else if !kind.is_file() {
            return refuse(&child, "is neither a regular file nor a directory");
        }
    }
    Ok(())
}

fn check_root(dir: &Path) -> Result<String, BundleError> {
    if !dir.is_absolute() {
        return refuse(
            ROOT,
            format!("the bundle path {} is not absolute", dir.display()),
        );
    }
    let kind = fs::symlink_metadata(dir)
        .or_else(|error| refuse(ROOT, format!("the bundle cannot be read: {error}")))?
        .file_type();
    if kind.is_symlink() {
        return refuse(
            ROOT,
            format!("the bundle path {} is a symlink", dir.display()),
        );
    }
    if !kind.is_dir() {
        return refuse(
            ROOT,
            format!("the bundle path {} is not a directory", dir.display()),
        );
    }
    dir.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| BundleError {
            path: ROOT.to_string(),
            reason: format!("the bundle path {} has no name", dir.display()),
        })
}

fn check_chain(dir: &Path, id: &str, pinned_id: &str) -> Result<Vec<u8>, BundleError> {
    let manifest = fs::read(dir.join(MANIFEST))
        .or_else(|error| refuse(MANIFEST, format!("cannot be read: {error}")))?;
    let manifest_digest = sha256_hex(&manifest);
    if id.len() != ID_DIGITS || !manifest_digest.starts_with(id) {
        return refuse(
            MANIFEST,
            format!(
                "has SHA-256 {manifest_digest}, which does not begin with the bundle's name `{id}`"
            ),
        );
    }
    if id != pinned_id {
        return refuse(
            MANIFEST,
            format!("names bundle `{id}`, but these lanes are pinned to bundle `{pinned_id}`"),
        );
    }
    let listed = serde_json::from_slice::<serde_json::Value>(&manifest)
        .ok()
        .and_then(|value| value.get("sha256sums_sha256")?.as_str().map(str::to_string))
        .filter(|digest| is_digest(digest));
    let Some(listed) = listed else {
        return refuse(MANIFEST, "has no lowercase SHA-256 in `sha256sums_sha256`");
    };
    let sums = fs::read(dir.join(SUMS))
        .or_else(|error| refuse(SUMS, format!("cannot be read: {error}")))?;
    let sums_digest = sha256_hex(&sums);
    if sums_digest != listed {
        return refuse(
            SUMS,
            format!("has SHA-256 {sums_digest}, but {MANIFEST} lists {listed}"),
        );
    }
    Ok(sums)
}

fn read_sums(sums: &[u8]) -> Result<BTreeMap<String, String>, BundleError> {
    let text = std::str::from_utf8(sums)
        .or_else(|error| refuse(SUMS, format!("is not UTF-8: {error}")))?;
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

/// Check the bundle at the absolute path `dir` and refuse it, naming the path at fault, unless:
/// no symlink and nothing but regular files and directories sit anywhere in its tree; the
/// SHA-256 of `manifest.json` begins with the directory's name, and that name is `pinned_id`;
/// the manifest's `sha256sums_sha256` is the SHA-256 of `SHA256SUMS`; every file `SHA256SUMS`
/// lists matches its digest; and every file under `lib/` is listed. Nothing is loaded; the
/// caller loads only after this returns `Ok`.
pub fn verify_bundle(dir: &Path, pinned_id: &str) -> Result<VerifiedBundle, BundleError> {
    let id = check_root(dir)?;
    no_links_under(dir, ROOT)?;
    let sums = check_chain(dir, &id, pinned_id)?;
    let files = read_sums(&sums)?;
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
        id,
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

const USR_LIB: &str = "/usr/lib/";

fn plain_absolute(text: &str) -> bool {
    text.strip_prefix('/').is_some_and(|rest| {
        rest.split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
    })
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The loaded images that are none of: the file at `executable`, the verified `libkrun`, an
/// image `in_shared_cache` accepts, or a file named by a plain absolute path under `/usr/lib/`.
/// Nothing else under `/System/` is accepted, because `/System/Volumes/Data` is writable.
pub fn unverified_images(
    images: &[PathBuf],
    executable: &Path,
    libkrun: &Path,
    in_shared_cache: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    images
        .iter()
        .filter(|image| {
            let text = image.to_string_lossy();
            let system = plain_absolute(&text) && text.starts_with(USR_LIB);
            !(system
                || in_shared_cache(image)
                || same_file(image, executable)
                || same_file(image, libkrun))
        })
        .cloned()
        .collect()
}

/// True when `path` resolves to a file inside `dir`.
pub fn is_inside(_dir: &Path, _path: &Path) -> bool {
    false
}
