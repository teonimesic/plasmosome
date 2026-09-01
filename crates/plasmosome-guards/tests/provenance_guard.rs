use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use plasmosome_guards::workspace_root;

fn guard() -> PathBuf {
    workspace_root().join(".githooks").join("provenance-guard")
}

fn shadow_git_with(script: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let stub = directory.path().join("git");
    std::fs::write(&stub, script).expect("the stub is written");
    let mut permissions = std::fs::metadata(&stub)
        .expect("the stub is readable")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&stub, permissions).expect("the stub is made executable");
    directory
}

#[test]
fn refuses_when_the_search_it_depends_on_cannot_run() {
    let shadow = shadow_git_with("#!/bin/sh\necho 'fatal: not a git repository' >&2\nexit 128\n");
    let inherited = std::env::var("PATH").unwrap_or_default();
    let output = Command::new(guard())
        .current_dir(workspace_root())
        .env(
            "PATH",
            format!("{}:{inherited}", shadow.path().to_string_lossy()),
        )
        .output()
        .expect("the guard runs");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "the guard reported clean while the search it depends on was failing; `git grep` exits 1 when it finds nothing and 128 when it cannot look, and reading the second as the first turns a broken search into a pass.\nguard said:\n{said}"
    );
}
