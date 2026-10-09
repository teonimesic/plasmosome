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
/// The cache is `plasmosome-supervision-fixture` inside the nearest ancestor
/// holding the `CACHEDIR.TAG` file Cargo writes at the root of its target
/// directory. It therefore stays inside the user's own target directory
/// whether test executables sit in `deps` or in Cargo's newer
/// `build/<package>/<hash>/out` layout, and `cargo clean` removes it. Panics,
/// naming the path, when no ancestor is tagged; it never falls back to a
/// shared temporary directory.
pub fn fixture_cache_root(executable: &Path) -> PathBuf {
    executable
        .ancestors()
        .skip(1)
        .find(|directory| directory.join("CACHEDIR.TAG").is_file())
        .unwrap_or_else(|| {
            panic!(
                "the test executable {} is not inside a Cargo target directory tagged with CACHEDIR.TAG, so the supervision fixture has no cache there",
                executable.display()
            )
        })
        .join("plasmosome-supervision-fixture")
}

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
/// executable once with no arguments and panics unless it exits 64. The key
/// does not include the compiler, so after a compiler or SDK upgrade run
/// `cargo clean` to rebuild the fixture with it.
pub fn compile_supervision_fixture(cache_root: &Path, source: &Path) -> PathBuf {
    let key = cache_root.join(source_key(source));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&key)
        .unwrap_or_else(|error| {
            panic!(
                "the fixture cache directory {} is created, readable only by its owner: {error}",
                key.display()
            )
        });
    let executable = key.join("supervision-worker");
    if !executable.exists() {
        publish(source, &executable);
    }
    let usage = Command::new(&executable).status().unwrap_or_else(|error| {
        panic!(
            "the compiled supervision worker fixture {} runs: {error}; if it is stale, remove it or run `cargo clean`",
            executable.display()
        )
    });
    assert_eq!(
        usage.code(),
        Some(64),
        "the compiled supervision worker fixture {} refuses a call with no arguments; if it is stale, remove it or run `cargo clean`",
        executable.display()
    );
    executable
}

fn source_key(source: &Path) -> String {
    let bytes = fs::read(source).unwrap_or_else(|error| {
        panic!(
            "the supervision worker source {} is readable: {error}",
            source.display()
        )
    });
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
    assert!(
        status.success(),
        "the supervision worker fixture compiles from {}: {status}",
        source.display()
    );
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
    let fixture = supervision_fixture();
    let root = fixture
        .parent()
        .and_then(Path::parent)
        .expect("the fixture sits two directories below its cache root");
    let target = root
        .parent()
        .expect("the cache root sits in the target directory");
    assert!(
        root.ends_with("plasmosome-supervision-fixture")
            && target.join("CACHEDIR.TAG").is_file()
            && executable.starts_with(target),
        "the compiled fixture {} is cached in the Cargo target directory that holds the test executable {}, not in a shared temporary directory",
        fixture.display(),
        executable.display()
    );
}
