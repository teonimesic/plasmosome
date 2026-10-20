use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The leaf name libkrun v1.19.4 passes to `dlopen` inside `krun_create_ctx`.
pub const LIBKRUNFW: &str = "libkrunfw.5.dylib";

/// The `LC_RPATH` entries of the pinned bundle's libkrun.
pub const LIBKRUN_RPATHS: &[&str] = &["/opt/homebrew/opt/llvm/lib"];

/// dyld's default fallback directories for a leaf name.
pub const FALLBACK_DIRS: &[&str] = &["/usr/local/lib", "/usr/lib"];

/// Every path dyld may try for libkrun's leaf-name load of `LIBKRUNFW`, in order: the working
/// directory `cwd`, the pinned libkrun's `LC_RPATH` entries, and the default fallbacks.
pub fn libkrunfw_search(cwd: &Path) -> Vec<PathBuf> {
    std::iter::once(cwd)
        .chain(LIBKRUN_RPATHS.iter().map(Path::new))
        .chain(FALLBACK_DIRS.iter().map(Path::new))
        .map(|dir| dir.join(LIBKRUNFW))
        .collect()
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
