use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

static FIXTURE: LazyLock<PathBuf> = LazyLock::new(|| {
    let unique = format!(
        "plasmosome-supervision-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the system clock is past the epoch")
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(&unique);
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/supervision_worker.c");

    fs::create_dir(&dir).expect("the fixture temporary directory is created");
    fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .expect("the fixture temporary directory is locked to the owner");
    let output = dir.join("supervision-worker");
    let status = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg(&source)
        .arg("-o")
        .arg(&output)
        .status()
        .expect("the host C compiler starts for the supervision fixture");
    assert!(status.success(), "the supervision worker fixture compiles");
    let usage = Command::new(&output)
        .status()
        .expect("the compiled supervision worker fixture runs");
    assert_eq!(
        usage.code(),
        Some(64),
        "the compiled supervision worker fixture refuses a call with no arguments"
    );
    output
});

/// Takes no arguments and returns the path of the supervision worker fixture.
///
/// The fixture is compiled once per test process with the host C compiler,
/// then run once with no arguments before its path is returned. macOS charges
/// the first exec of a freshly linked binary up to seconds under load, so the
/// warm-up spends that cost here, where no test deadline is running.
/// Callers must not remove or replace the returned executable. Production
/// builds of the crate never compile the fixture; the build script compiles
/// only the product Darwin helper.
pub fn supervision_fixture() -> PathBuf {
    FIXTURE.clone()
}
