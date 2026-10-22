use std::any::Any;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const TAG_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

/// Takes the path of a running test executable, the name of a cache
/// directory, what the cache holds, and the run-time `CARGO_TARGET_DIR` if one
/// is set, and returns `cache` inside the nearest ancestor of the executable
/// that holds the `CACHEDIR.TAG` file Cargo writes.
///
/// With no tagged ancestor it returns the message every test that needs the
/// cache fails with. The message names the executable, says why the tag can be
/// missing and gives the command that writes it, into `declared_target` when
/// that directory holds the executable. There is no fallback directory.
pub fn cache_root(
    executable: &Path,
    cache: &str,
    what: &str,
    declared_target: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(target) = executable
        .ancestors()
        .skip(1)
        .find(|directory| directory.join("CACHEDIR.TAG").is_file())
    {
        return Ok(target.join(cache));
    }
    let fix = match declared_target.filter(|target| holds(target, executable)) {
        Some(target) => format!(
            "write the tag into CARGO_TARGET_DIR: echo '{TAG_SIGNATURE}' > {}",
            shell_quoted(&target.join("CACHEDIR.TAG"))
        ),
        None => format!(
            "write the tag into the Cargo target directory that holds the test executable: echo '{TAG_SIGNATURE}' > <that directory>/CACHEDIR.TAG"
        ),
    };
    Err(format!(
        "the test executable {} has no ancestor holding CACHEDIR.TAG, so {what} has no cache inside a Cargo target directory. Cargo writes that tag only when it creates the target directory itself; a directory made by hand or by cargo llvm-cov has none. Let Cargo create the directory, or {fix}",
        executable.display()
    ))
}

/// Takes nothing and returns `CARGO_TARGET_DIR` as this test process sees
/// it, or `None` when it is unset.
pub fn declared_target() -> Option<PathBuf> {
    std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from)
}

/// Takes the function that builds a fixture and returns the fixture's path,
/// or the message the function panicked with.
///
/// A fixture kept in a `LazyLock` is built through this, so a failed build is
/// kept as a value and the lock is never poisoned.
pub fn set_up(build: fn() -> PathBuf) -> Result<PathBuf, String> {
    std::panic::catch_unwind(build).map_err(panic_text)
}

/// Takes a fixture built through `set_up` and returns its path, or panics
/// with the message its build failed with. Every call panics with that same
/// message.
pub fn prepared(fixture: &LazyLock<Result<PathBuf, String>>) -> PathBuf {
    match &**fixture {
        Ok(path) => path.clone(),
        Err(message) => panic!("{message}"),
    }
}

fn holds(target: &Path, executable: &Path) -> bool {
    let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    !target.as_os_str().is_empty()
        && (executable.starts_with(target) || canonical(executable).starts_with(canonical(target)))
}

fn shell_quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

fn panic_text(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => payload.downcast_ref::<&str>().map_or_else(
            || "the fixture's setup panicked with a payload that is not text".to_owned(),
            |message| (*message).to_owned(),
        ),
    }
}
