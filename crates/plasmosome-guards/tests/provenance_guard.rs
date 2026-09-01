use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn detached_from_any_inherited_repository(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
}

fn git_in(repository: &Path, arguments: &[&str]) -> Output {
    let mut command = Command::new("git");
    detached_from_any_inherited_repository(&mut command)
        .current_dir(repository)
        .args(arguments)
        .output()
        .expect("git runs")
}

fn scratch_repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let output = git_in(directory.path(), &["init", "--quiet"]);
    assert!(
        output.status.success(),
        "the scratch repository was not created: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    directory
}

fn track_everything_in(repository: &Path) {
    let output = git_in(repository, &["add", "-A"]);
    assert!(
        output.status.success(),
        "the scratch file was not tracked, and the search this guard runs reads tracked files only, so an untracked plant would read as a clean tree: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_guard_in(repository: &Path) -> Output {
    let mut command = Command::new(guard());
    detached_from_any_inherited_repository(&mut command)
        .current_dir(repository)
        .output()
        .expect("the guard runs")
}

fn first_forbidden_term() -> String {
    let script = std::fs::read_to_string(guard()).expect("the guard script is readable");
    let declared = script
        .lines()
        .find_map(|line| line.trim().strip_prefix("forbidden=("))
        .expect("the guard declares the terms it refuses in a shell array named `forbidden`");
    declared
        .split(')')
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .next()
        .expect("the guard refuses at least one term")
        .to_string()
}

#[test]
fn refuses_a_tree_carrying_a_term_it_forbids() {
    let repository = scratch_repository();
    let notes = repository.path().join("notes.md");

    std::fs::write(&notes, "a tree with nothing in it to refuse\n").expect("the file is written");
    track_everything_in(repository.path());
    let clean = run_guard_in(repository.path());
    assert!(
        clean.status.success(),
        "the guard refused a scratch tree carrying no forbidden term, so the refusal asserted below would not distinguish a working guard from one that refuses everything; it said:\n{}{}",
        String::from_utf8_lossy(&clean.stdout),
        String::from_utf8_lossy(&clean.stderr)
    );

    let term = first_forbidden_term();
    std::fs::write(&notes, format!("a tree that names {term} in passing\n"))
        .expect("the file is written");
    track_everything_in(repository.path());
    let planted = run_guard_in(repository.path());
    assert!(
        !planted.status.success(),
        "the guard cleared a scratch tree carrying a term it forbids; this is the check that keeps the private research corpus out of a public repository, and one that cannot fail on the violation it names proves nothing about the tree it clears"
    );
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
