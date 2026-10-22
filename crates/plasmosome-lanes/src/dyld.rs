use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The leaf name libkrun v1.19.4 passes to `dlopen` inside `krun_create_ctx`. That libkrun's
/// record is `docs/dependencies/native/libkrun.toml`.
pub const LIBKRUNFW: &str = "libkrunfw.5.dylib";

/// The `LC_RPATH` entries of the pinned bundle's libkrun, each written as an absolute path in the
/// load command, as `leaf_search` needs. That libkrun's record is
/// `docs/dependencies/native/libkrun.toml`.
pub const LIBKRUN_RPATHS: &[&str] = &["/opt/homebrew/opt/llvm/lib"];

const CRYPTEX_PREFIX: &str = "/System/Volumes/Preboot/Cryptexes/OS";

/// dyld's default fallback directories for a leaf name, for a binary linked against an SDK from
/// fall 2023 or later, as `lane-vmm` is. A binary linked against an older SDK also gets
/// `/usr/local/lib` (dyld(1)).
pub const FALLBACK_DIRS: &[&str] = &["/usr/lib"];

/// Every path dyld tries for a `dlopen` of the leaf name `leaf`, each once, in the order dyld first
/// tries it: `cwd` joined with `leaf`, the cryptex copy of `leaf`, each entry of `rpaths` joined
/// with `leaf` followed by its cryptex copy, then each fallback directory joined with `leaf`.
///
/// A cryptex copy is `/System/Volumes/Preboot/Cryptexes/OS` followed by the path, with no
/// separator between them, so `L` becomes `/System/Volumes/Preboot/Cryptexes/OSL`. dyld tries it
/// right after the leaf name and after each rpath entry written as an absolute path. It tries none
/// after the `/usr/lib` fallback, after an `@executable_path` entry it expanded, or after a
/// relative entry. Measured on macOS 26.6.2 (25G83); an `@loader_path` entry was not measured.
///
/// `leaf` must hold no `/`. `rpaths` are the `LC_RPATH` entries written as absolute paths in the
/// load commands of the image that calls `dlopen`, then in those of the main executable. Pass no
/// entry that starts with `@` and no relative one, expanded or not: dyld gave the ones measured no
/// cryptex copy, and this list would add one. `lane-vmm` has no entry of its own. The list is for
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
