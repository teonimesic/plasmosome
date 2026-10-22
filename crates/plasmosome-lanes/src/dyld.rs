use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The leaf name libkrun v1.19.4 passes to `dlopen` inside `krun_create_ctx`. That libkrun's
/// record is `docs/dependencies/native/libkrun.toml`.
pub const LIBKRUNFW: &str = "libkrunfw.5.dylib";

/// The `LC_RPATH` entries of the pinned bundle's libkrun. That libkrun's record is
/// `docs/dependencies/native/libkrun.toml`.
pub const LIBKRUN_RPATHS: &[&str] = &["/opt/homebrew/opt/llvm/lib"];

/// The OS cryptex prefix. Before a path itself, dyld tries this prefix followed by the path, with
/// no separator between them, so a leaf name `L` becomes `/System/Volumes/Preboot/Cryptexes/OSL`.
/// Measured on macOS 26.6.2 (25G83).
pub const CRYPTEX_PREFIX: &str = "/System/Volumes/Preboot/Cryptexes/OS";

/// dyld's default fallback directories for a leaf name, for a binary built with an SDK from fall
/// 2023 or later, as `lane-vmm` is. An older binary also gets `/usr/local/lib` (dyld(1)).
pub const FALLBACK_DIRS: &[&str] = &["/usr/lib"];

/// Every path dyld tries for a `dlopen` of the leaf name `leaf`, each once, in the order dyld first
/// tries it: `cwd` joined with `leaf`, the cryptex copy of `leaf`, each entry of `rpaths` joined
/// with `leaf` followed by its cryptex copy, then each fallback directory joined with `leaf`.
///
/// `leaf` must hold no `/`. `rpaths` are the absolute `LC_RPATH` entries of the image that calls
/// `dlopen`, then those of the main executable. `lane-vmm` has none of its own. The list is for
/// an unrestricted process with no `DYLD_` variable set, which `lane-vmm` refuses first. A
/// restricted process, such as a platform binary, skips the `cwd` path.
pub fn leaf_search(cwd: &Path, leaf: &str, rpaths: &[&str]) -> Vec<PathBuf> {
    let leaf = Path::new(leaf);
    let in_rpaths = rpaths.iter().flat_map(|rpath| {
        let path = Path::new(rpath).join(leaf);
        let copy = cryptex_copy(&path);
        [path, copy]
    });
    let candidates = [cwd.join(leaf), cryptex_copy(leaf)]
        .into_iter()
        .chain(in_rpaths)
        .chain(FALLBACK_DIRS.iter().map(|dir| Path::new(dir).join(leaf)));
    let mut search = Vec::new();
    for candidate in candidates {
        if !search.contains(&candidate) {
            search.push(candidate);
        }
    }
    search
}

fn cryptex_copy(path: &Path) -> PathBuf {
    let mut copy = OsString::from(CRYPTEX_PREFIX);
    copy.push(path.as_os_str());
    PathBuf::from(copy)
}

/// Every path dyld tries for libkrun's leaf-name load of `LIBKRUNFW`, in order: the working
/// directory `cwd`, the cryptex copy of the leaf name, the pinned libkrun's `LC_RPATH` entry and
/// its cryptex copy, and `/usr/lib`. That is `leaf_search(cwd, LIBKRUNFW, LIBKRUN_RPATHS)`.
pub fn libkrunfw_search(cwd: &Path) -> Vec<PathBuf> {
    leaf_search(cwd, LIBKRUNFW, LIBKRUN_RPATHS)
}

/// The candidates that exist as any kind of file, a dangling symlink included.
pub fn planted(candidates: &[PathBuf]) -> Vec<PathBuf> {
    candidates
        .iter()
        .filter(|candidate| fs::symlink_metadata(candidate).is_ok())
        .cloned()
        .collect()
}

/// The names of the `DYLD_*` variables in `vars`, in order.
pub fn dyld_variables(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<String> {
    vars.into_iter()
        .filter(|(name, _)| name.as_bytes().starts_with(b"DYLD_"))
        .map(|(name, _)| name.to_string_lossy().to_string())
        .collect()
}
