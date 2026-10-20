use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The leaf name libkrun v1.19.4 passes to `dlopen` inside `krun_create_ctx`.
pub const LIBKRUNFW: &str = "libkrunfw.5.dylib";

/// Every path dyld may try for libkrun's leaf-name load of `LIBKRUNFW`.
pub fn libkrunfw_search(_cwd: &Path) -> Vec<PathBuf> {
    Vec::new()
}

/// The candidates that exist, as any kind of file.
pub fn planted(_candidates: &[PathBuf]) -> Vec<PathBuf> {
    Vec::new()
}

/// The names of the `DYLD_*` variables in `vars`.
pub fn dyld_variables(_vars: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<String> {
    Vec::new()
}
