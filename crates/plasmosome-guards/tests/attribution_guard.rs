use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use plasmosome_guards::workspace_root;
use tempfile::TempDir;

fn guard() -> PathBuf {
    workspace_root().join(".githooks").join("attribution-guard")
}

fn git(repository: &Path, arguments: &[&str], stdin: Option<&str>) -> Output {
    let mut command = Command::new("git");
    command
        .current_dir(repository)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "A Person")
        .env("GIT_AUTHOR_EMAIL", "person@example.com")
        .env("GIT_COMMITTER_NAME", "A Person")
        .env("GIT_COMMITTER_EMAIL", "person@example.com")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
        .args(arguments);
    if let Some(text) = stdin {
        command.stdin(std::process::Stdio::piped());
        let mut child = command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("git runs");
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("the spawned git has a stdin pipe")
            .write_all(text.as_bytes())
            .expect("the commit message is written to git");
        return child.wait_with_output().expect("git finishes");
    }
    command.output().expect("git runs")
}

fn repository_with_commit(message: &str) -> TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let path = directory.path();
    for arguments in [
        vec!["init", "--quiet"],
        vec!["config", "commit.gpgsign", "false"],
    ] {
        let output = git(path, &arguments, None);
        assert!(
            output.status.success(),
            "`git {}` failed in the scratch repository: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = git(
        path,
        &[
            "commit",
            "--allow-empty",
            "--cleanup=verbatim",
            "--file",
            "-",
        ],
        Some(message),
    );
    assert!(
        output.status.success(),
        "the fixture commit was not created: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    directory
}

fn run_guard_in(repository: &Path, range: &str) -> Output {
    guard_command(repository)
        .arg(range)
        .output()
        .expect("the guard runs")
}

fn guard_command(repository: &Path) -> Command {
    let mut command = Command::new(guard());
    command
        .current_dir(repository)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR");
    command
}

fn run_guard_over(message: &str) -> Output {
    let repository = repository_with_commit(message);
    run_guard_in(repository.path(), "HEAD")
}

const OFFENCE: &str = "credits a model as an author";

fn assert_refused(message: &str, why: &str) {
    let output = run_guard_over(message);
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "the guard passed a commit that {why}; its purpose is to stop exactly this from reaching main.\nmessage:\n{message}\nguard said:\n{said}"
    );
    assert!(
        said.contains(OFFENCE),
        "the guard refused a commit that {why}, but not by naming the trailer it found; a guard that refuses for some other reason would satisfy an assertion on the exit status alone, so the exit status is not enough to prove it saw anything.\nmessage:\n{message}\nguard said:\n{said}"
    );
}

fn assert_cleared(message: &str, why: &str) {
    let output = run_guard_over(message);
    assert!(
        output.status.success(),
        "the guard refused a commit that {why}; a guard that flags prose about attribution makes writing about the rule impossible.\nmessage:\n{message}\nguard said:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn refuses_a_model_trailer_in_the_closing_trailer_block() {
    assert_refused(
        "docs: a change\n\nSome body prose.\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
        "closes with a trailer block naming a model",
    );
}

#[test]
fn refuses_a_model_trailer_a_squash_merge_left_in_the_middle_of_the_body() {
    assert_refused(
        "docs: a squashed change (#2)\n\n* docs: the first commit\n\nSome body prose.\n\nCo-Authored-By: Claude (Fable 5)\n\n* docs: the second commit\n\nMore body prose.\n",
        "carries a model trailer in a paragraph the body continues past",
    );
}

#[test]
fn refuses_a_model_trailer_a_squash_merge_left_before_a_human_trailer_block() {
    assert_refused(
        "docs: a squashed change (#2)\n\n* docs: the first commit\n\nCo-Authored-By: Claude (Fable 5)\n\n* docs: the second commit\n\nCo-Authored-By: A Colleague <colleague@example.com>\n",
        "credits a model mid-body and a person at the end, so only the person is visible to a parser that reads the last paragraph",
    );
}

#[test]
fn refuses_a_model_trailer_mid_body_when_the_message_uses_windows_line_endings() {
    assert_refused(
        "docs: a squashed change (#2)\r\n\r\n* docs: the first commit\r\n\r\nCo-Authored-By: Claude (Fable 5)\r\n\r\n* docs: the second commit\r\n\r\nMore body prose.\r\n",
        "carries a mid-body model trailer in a message written with CRLF line endings, where a paragraph break is a line holding a carriage return rather than an empty one",
    );
}

#[test]
fn refuses_a_model_trailer_mid_body_when_paragraph_breaks_hold_whitespace() {
    assert_refused(
        "docs: a squashed change (#2)\n \n* docs: the first commit\n \nCo-Authored-By: Claude (Fable 5)\n \n* docs: the second commit\n",
        "carries a mid-body model trailer in a message whose paragraph breaks hold a space, which git itself reads as a break",
    );
}

#[test]
fn clears_a_body_that_quotes_a_trailer_inside_a_sentence() {
    assert_cleared(
        "ci(guard): refuse a commit that credits a model as an author\n\nTwo commits carry a `Co-Authored-By: Claude (Fable 5)` line that the guard cannot see.\n",
        "mentions a trailer inside a sentence rather than writing one",
    );
}

#[test]
fn clears_a_body_that_indents_a_trailer_as_an_example() {
    assert_cleared(
        "ci(guard): refuse a commit that credits a model as an author\n\nThe shape the guard now refuses:\n\n    Co-Authored-By: Claude (Fable 5)\n\nA model is a tool, and tools do not co-author.\n",
        "shows a trailer as an indented example",
    );
}

#[test]
fn clears_a_paragraph_that_mixes_prose_with_a_quoted_trailer() {
    assert_cleared(
        "ci(guard): refuse a commit that credits a model as an author\n\nThe guard refuses this line:\nCo-Authored-By: Claude (Fable 5)\n",
        "puts a quoted trailer under a line of prose in the same paragraph",
    );
}

#[test]
fn refuses_a_trailer_naming_a_vendor_whose_name_no_person_carries() {
    for value in [
        "Cursor Agent <cursoragent@cursor.com>",
        "Devin <devin@cognition.ai>",
        "Grok <noreply@x.ai>",
    ] {
        assert_refused(
            &format!("docs: a change\n\nSome body prose.\n\nCo-Authored-By: {value}\n"),
            "credits a coding agent this guard did not name before",
        );
    }
}

#[test]
fn clears_a_body_whose_only_co_authors_are_people() {
    assert_cleared(
        "feat: a change\n\n* feat: the first commit\n\nCo-Authored-By: A Colleague <colleague@example.com>\n\n* feat: the second commit\n\nCo-Authored-By: Another Colleague <another@example.com>\n",
        "credits two people and no model",
    );
}

#[test]
fn refuses_a_range_it_cannot_read() {
    let repository = repository_with_commit("docs: a change\n");
    let output = run_guard_in(repository.path(), "no-such-ref..HEAD");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "the guard cleared a range it could not read; a range it cannot read is a range it cannot clear.\nguard said:\n{said}"
    );
    assert!(
        said.contains("cannot read the range"),
        "the guard refused an unreadable range without saying that is why, so its output does not tell a caller what to fix.\nguard said:\n{said}"
    );
}

#[test]
fn refuses_when_the_tool_it_matches_with_cannot_answer() {
    let repository = repository_with_commit(
        "docs: a change\n\nSome body prose.\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
    );
    let shadow = tempfile::tempdir().expect("a scratch directory is created");
    let stub = shadow.path().join("grep");
    std::fs::write(&stub, "#!/bin/sh\nexit 2\n").expect("the stub is written");
    let mut permissions = std::fs::metadata(&stub)
        .expect("the stub is readable")
        .permissions();
    use std::os::unix::fs::PermissionsExt;
    permissions.set_mode(0o755);
    std::fs::set_permissions(&stub, permissions).expect("the stub is made executable");

    let inherited = std::env::var("PATH").unwrap_or_default();
    let output = guard_command(repository.path())
        .env(
            "PATH",
            format!("{}:{inherited}", shadow.path().to_string_lossy()),
        )
        .arg("HEAD")
        .output()
        .expect("the guard runs");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "the guard cleared a commit carrying a model trailer while the tool it matches with was failing; `grep -q` exits 1 for no match and 2 or more for an error, and reading an error as no match is how a guard reports clean because its own tooling broke.\nguard said:\n{said}"
    );
}
