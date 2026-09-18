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

/// Returns the path of the supervision worker fixture, compiling it with the
/// host C compiler once per test process. Tests run this; production builds
/// of the crate never need a C toolchain for the fixture, because the build
/// script compiles only the product Darwin helper.
pub fn supervision_fixture() -> PathBuf {
    FIXTURE.clone()
}
