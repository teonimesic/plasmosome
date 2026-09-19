use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

static FIXTURE: LazyLock<PathBuf> = LazyLock::new(|| {
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/supervision_worker.c");
    let output = std::env::temp_dir().join(format!(
        "plasmosome-supervision-worker-{}",
        std::process::id()
    ));
    let status = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg(&source)
        .arg("-o")
        .arg(&output)
        .status()
        .expect("the host C compiler starts for the supervision fixture");
    assert!(status.success(), "the supervision worker fixture compiles");
    output
});

/// Takes no arguments and returns the path of the supervision worker fixture.
///
/// The fixture is compiled once per test process with the host C compiler.
/// Callers must not remove or replace the returned executable. Production
/// builds of the crate never compile the fixture; the build script compiles
/// only the product Darwin helper.
pub fn supervision_fixture() -> PathBuf {
    FIXTURE.clone()
}
