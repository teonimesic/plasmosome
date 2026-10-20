use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

const FAKE_COMPILER_ARGUMENTS: [&str; 4] = ["-std=c11", "-Wall", "-Wextra", "-Werror"];

const FAKE_MODES: [&str; 1] = ["record"];

static FAKE_DIRECTORY: LazyLock<PathBuf> = LazyLock::new(|| {
    let executable = std::env::current_exe().expect("the running test executable has a path");
    let root = executable
        .ancestors()
        .skip(1)
        .find(|directory| directory.join("CACHEDIR.TAG").is_file())
        .unwrap_or_else(|| {
            panic!(
                "the test executable {} is not inside a Cargo target directory tagged with CACHEDIR.TAG, so the runtime fake has no cache there",
                executable.display()
            )
        })
        .join("plasmosome-runtime-fake");
    compile_runtime_fake(&root)
});

/// Takes the name of a mode of `tests/support/runtime_fake.c` and returns the
/// path of the runtime fake under that name, since the fake picks its mode
/// from the base name of `argv[0]`.
///
/// The fake is compiled with the host C compiler once per source into
/// `plasmosome-runtime-fake/<key>/runtime-fake` inside the Cargo target
/// directory, as `supervision_fixture` is, and each mode is a hard link to
/// it. Each test process runs it once with no mode before the first path is
/// returned, so macOS spends the first-exec cost of a freshly linked binary
/// where no test deadline is running. Panics on a mode the fake lacks.
/// Callers must not remove or replace the returned file.
pub fn runtime_fake(mode: &str) -> PathBuf {
    assert!(
        FAKE_MODES.contains(&mode),
        "{mode} is a mode of runtime_fake.c"
    );
    FAKE_DIRECTORY.join(mode)
}

fn compile_runtime_fake(root: &Path) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/runtime_fake.c");
    let key = root.join(fake_key(&source));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&key)
        .unwrap_or_else(|error| {
            panic!(
                "the runtime fake cache directory {} is created, readable only by its owner: {error}",
                key.display()
            )
        });
    let fake = key.join("runtime-fake");
    if !fake.exists() {
        publish_fake(&source, &fake);
    }
    for mode in FAKE_MODES {
        if let Err(error) = fs::hard_link(&fake, key.join(mode))
            && error.kind() != ErrorKind::AlreadyExists
        {
            panic!("the runtime fake is linked as mode {mode}: {error}");
        }
    }
    let usage = Command::new(&fake).status().unwrap_or_else(|error| {
        panic!(
            "the compiled runtime fake {} runs: {error}; if it is stale, run `cargo clean`",
            fake.display()
        )
    });
    assert_eq!(
        usage.code(),
        Some(64),
        "the compiled runtime fake {} refuses a name that is no mode; if it is stale, run `cargo clean`",
        fake.display()
    );
    key
}

fn fake_key(source: &Path) -> String {
    let bytes = fs::read(source).unwrap_or_else(|error| {
        panic!(
            "the runtime fake source {} is readable: {error}",
            source.display()
        )
    });
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    FAKE_COMPILER_ARGUMENTS.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn publish_fake(source: &Path, fake: &Path) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let unique = fake.with_extension(format!(
        "{}-{}.partial",
        std::process::id(),
        CALLS.fetch_add(1, Ordering::Relaxed)
    ));
    let compiled = Command::new("cc")
        .args(FAKE_COMPILER_ARGUMENTS)
        .arg(source)
        .arg("-o")
        .arg(&unique)
        .status();
    let published = match &compiled {
        Ok(status) if status.success() => fs::hard_link(&unique, fake),
        _ => Ok(()),
    };
    if let Err(error) = fs::remove_file(&unique)
        && error.kind() != ErrorKind::NotFound
    {
        panic!(
            "the runtime fake's temporary name {} is removed: {error}",
            unique.display()
        );
    }
    let status = compiled.expect("the host C compiler starts for the runtime fake");
    assert!(
        status.success(),
        "the runtime fake compiles from {}: {status}",
        source.display()
    );
    if let Err(error) = published
        && error.kind() != ErrorKind::AlreadyExists
    {
        panic!(
            "the compiled runtime fake is published at {}: {error}",
            fake.display()
        );
    }
}
