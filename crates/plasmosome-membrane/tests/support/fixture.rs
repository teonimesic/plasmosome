use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

const COMPILER_ARGUMENTS: [&str; 4] = ["-std=c11", "-Wall", "-Wextra", "-Werror"];

static FIXTURE: LazyLock<PathBuf> = LazyLock::new(|| {
    let executable = std::env::current_exe().expect("the running test executable has a path");
    compile_supervision_fixture(
        &fixture_cache_root(&executable),
        &supervision_worker_source(),
    )
});

/// Takes no arguments and returns the path of the supervision worker fixture.
///
/// The fixture is compiled with the host C compiler once per source into the
/// target directory and reused by every later test process; see
/// `compile_supervision_fixture`. Each test process runs it once with no
/// arguments before its path is returned. macOS charges the first exec of a
/// freshly linked binary up to seconds under load, so the warm-up spends that
/// cost here, where no test deadline is running.
/// Callers must not remove or replace the returned executable. Production
/// builds of the crate never compile the fixture; the build script compiles
/// only the product Darwin helper.
pub fn supervision_fixture() -> PathBuf {
    FIXTURE.clone()
}

/// Takes the path of a running test executable and returns the directory the
/// compiled fixture is cached in.
///
/// Cargo places unit and integration test executables in
/// `<target>/<profile>/deps`, so the cache is the sibling directory
/// `<target>/<profile>/plasmosome-supervision-fixture`: it stays inside the
/// user's own target directory, and `cargo clean` removes it. Panics, naming
/// the path, when the executable is not in a `deps` directory; it never falls
/// back to a shared temporary directory.
pub fn fixture_cache_root(executable: &Path) -> PathBuf {
    executable
        .parent()
        .filter(|directory| directory.ends_with("deps"))
        .and_then(Path::parent)
        .unwrap_or_else(|| {
            panic!(
                "the test executable {} is not in a Cargo deps directory, so the supervision fixture has no cache inside the target directory",
                executable.display()
            )
        })
        .join("plasmosome-supervision-fixture")
}

/// Returns the path of the C source the supervision worker is compiled from.
pub fn supervision_worker_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/supervision_worker.c")
}

/// Takes a cache root and a C source, and returns the path of the supervision
/// worker compiled from that source.
///
/// The executable is `<cache_root>/<key>/supervision-worker`, where the key
/// hashes the source's bytes and the compiler arguments, and directories it
/// creates are readable only by their owner. An executable already published
/// there is reused. A missing one is compiled under a name unique to this
/// call and published with a hard link, which fails instead of replacing a
/// file when another caller, in this process or another, published first; a
/// published executable is therefore never replaced. Every call then runs the
/// executable once with no arguments and panics unless it exits 64.
pub fn compile_supervision_fixture(cache_root: &Path, source: &Path) -> PathBuf {
    let key = cache_root.join(source_key(source));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&key)
        .expect("the fixture cache directory is created, readable only by its owner");
    let executable = key.join("supervision-worker");
    if !executable.exists() {
        publish(source, &executable);
    }
    let usage = Command::new(&executable)
        .status()
        .expect("the compiled supervision worker fixture runs");
    assert_eq!(
        usage.code(),
        Some(64),
        "the compiled supervision worker fixture refuses a call with no arguments"
    );
    executable
}

fn source_key(source: &Path) -> String {
    let bytes = fs::read(source).expect("the supervision worker source is readable");
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    COMPILER_ARGUMENTS.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn publish(source: &Path, executable: &Path) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let unique = executable.with_extension(format!(
        "{}-{}.partial",
        std::process::id(),
        CALLS.fetch_add(1, Ordering::Relaxed)
    ));
    let compiled = Command::new("cc")
        .args(COMPILER_ARGUMENTS)
        .arg(source)
        .arg("-o")
        .arg(&unique)
        .status();
    let published = match &compiled {
        Ok(status) if status.success() => fs::hard_link(&unique, executable),
        _ => Ok(()),
    };
    if let Err(error) = fs::remove_file(&unique)
        && error.kind() != ErrorKind::NotFound
    {
        panic!(
            "the fixture's temporary name {} is removed: {error}",
            unique.display()
        );
    }
    let status = compiled.expect("the host C compiler starts for the supervision fixture");
    assert!(status.success(), "the supervision worker fixture compiles");
    if let Err(error) = published
        && error.kind() != ErrorKind::AlreadyExists
    {
        panic!(
            "the compiled fixture is published at {}: {error}",
            executable.display()
        );
    }
}

#[test]
fn the_supervision_fixture_is_cached_inside_the_target_directory() {
    let executable = std::env::current_exe().expect("the running test executable has a path");
    let profile = executable
        .parent()
        .and_then(Path::parent)
        .expect("the test executable sits two directories below the target directory");
    let fixture = supervision_fixture();
    assert_eq!(
        fixture.parent().and_then(Path::parent),
        Some(profile.join("plasmosome-supervision-fixture").as_path()),
        "the compiled fixture is cached beside deps in the target directory, not in a shared temporary directory"
    );
}
