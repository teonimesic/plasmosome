use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use plasmosome_guards::check_workspace;
use tempfile::TempDir;

fn guard(root: &Path) -> PathBuf {
    root.join(".githooks").join("attribution-guard")
}

fn isolated_from_the_callers_git(command: &mut Command, repository: &Path) {
    command
        .current_dir(repository)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR");
}

fn git(repository: &Path, arguments: &[&str], stdin: Option<&str>) -> Output {
    let mut command = Command::new("git");
    isolated_from_the_callers_git(&mut command, repository);
    command
        .env("GIT_AUTHOR_NAME", "A Person")
        .env("GIT_AUTHOR_EMAIL", "person@corp.locale.io")
        .env("GIT_COMMITTER_NAME", "A Person")
        .env("GIT_COMMITTER_EMAIL", "person@corp.locale.io")
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

fn git_succeeds(repository: &Path, arguments: &[&str], stdin: Option<&str>) {
    let output = git(repository, arguments, stdin);
    assert!(
        output.status.success(),
        "`git {}` failed in the scratch repository: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repository_with_commits(messages: &[&str]) -> TempDir {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let path = directory.path();
    git_succeeds(path, &["init", "--quiet"], None);
    git_succeeds(path, &["config", "commit.gpgsign", "false"], None);
    for message in messages {
        commit(path, message);
    }
    directory
}

fn commit(repository: &Path, message: &str) {
    git_succeeds(
        repository,
        &[
            "commit",
            "--allow-empty",
            "--cleanup=verbatim",
            "--file",
            "-",
        ],
        Some(message),
    );
}

fn repository_with_commit(message: &str) -> TempDir {
    repository_with_commits(&[message])
}

fn repository_with_a_merge(side: &str, merge: &str) -> TempDir {
    let repository = repository_with_commit("docs: a change both branches start from\n");
    let path = repository.path();
    git_succeeds(path, &["checkout", "--quiet", "-b", "side"], None);
    commit(path, side);
    git_succeeds(path, &["checkout", "--quiet", "-"], None);
    git_succeeds(
        path,
        &["merge", "--no-ff", "--quiet", "-m", merge, "side"],
        None,
    );
    repository
}

fn run_guard_in(root: &Path, repository: &Path, range: &str) -> Output {
    guard_command(root, repository)
        .arg(range)
        .output()
        .expect("the guard runs")
}

fn guard_command(root: &Path, repository: &Path) -> Command {
    let mut command = Command::new(guard(root));
    isolated_from_the_callers_git(&mut command, repository);
    command
}

fn run_guard_over(root: &Path, message: &str) -> Output {
    let repository = repository_with_commit(message);
    run_guard_in(root, repository.path(), "HEAD")
}

fn directory_shadowing(tool: &str, script: &str) -> TempDir {
    directory_shadowing_in(
        tempfile::tempdir().expect("a scratch directory is created"),
        tool,
        script,
    )
}

fn directory_shadowing_in(shadow: TempDir, tool: &str, script: &str) -> TempDir {
    let stub = shadow.path().join(tool);
    std::fs::write(&stub, script).expect("the stub is written");
    let mut permissions = std::fs::metadata(&stub)
        .expect("the stub is readable")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&stub, permissions).expect("the stub is made executable");
    shadow
}

fn run_guard_shadowed(root: &Path, repository: &Path, shadow: &Path) -> Output {
    let inherited = std::env::var("PATH").unwrap_or_default();
    guard_command(root, repository)
        .env("PATH", format!("{}:{inherited}", shadow.to_string_lossy()))
        .arg("HEAD")
        .output()
        .expect("the guard runs")
}

fn real_tool(tool: &str) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|directory| directory.join(tool))
        .find(|candidate| {
            std::fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
        .unwrap_or_else(|| panic!("{tool} is on PATH"))
}

fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn run_guard_with_git_refusing(root: &Path, repository: &Path, fragment: &str) -> Output {
    let shadow = tempfile::tempdir().expect("a scratch directory is created");
    let marker = shadow.path().join("the-stub-refused");
    let git = real_tool("git");
    let stub = format!(
        "#!/bin/sh\ncase \" $* \" in\n  *{}*)\n    : > {}\n    echo \"stub: refusing $*\" >&2\n    exit 128\n    ;;\nesac\nexec {} \"$@\"\n",
        shell_quoted(fragment),
        shell_quoted(marker.to_str().expect("the scratch path is UTF-8")),
        shell_quoted(git.to_str().expect("the path to git is UTF-8"))
    );
    let shadow = directory_shadowing_in(shadow, "git", &stub);
    let output = run_guard_shadowed(root, repository, shadow.path());
    assert!(
        marker.exists(),
        "setup: the git stub never saw a call holding `{fragment}`, so the guard no longer calls git that way and this test no longer reaches the branch it is for. Point the stub at the guard's new call; this alone is not a guard bug.\nguard said:\n{}",
        said(&output)
    );
    output
}

fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const OFFENCE: &str = "\" credits a model: ";

const FIX: &str = "reword the commit and correct or remove the line";

const UNTRACEABLE_CO_AUTHOR: &str =
    "docs: a change\n\nSome body prose.\n\nCo-authored-by: Probe <me@laptop.local>\n";

const MODEL_CO_AUTHOR: &str =
    "docs: a change\n\nSome body prose.\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n";

fn assert_refused_saying(output: &Output, reason: &str, what: &str) {
    let said = said(output);
    let refused = !output.status.success();
    let named = said.contains(reason);
    assert!(
        refused && named,
        "{}\nAn exit status alone proves nothing: a refusal for another reason satisfies it, and a reason printed over a successful exit still lets the commit through.\nguard said:\n{said}",
        match (refused, named) {
            (false, false) =>
                format!("the guard passed {what}; it must refuse it and say `{reason}`."),
            (false, true) => format!(
                "the guard said `{reason}` about {what}, but exited successfully, which lets it through."
            ),
            _ => format!(
                "the guard refused {what} without saying `{reason}`, so nothing shows it refused for that reason."
            ),
        }
    );
}

fn assert_refused(root: &Path, message: &str, why: &str) {
    assert_refused_saying(
        &run_guard_over(root, message),
        OFFENCE,
        &format!("a commit that {why} (message {message:?})"),
    );
}

fn assert_refused_crediting_each(root: &Path, key: &str, values: &[&str], why: &str) {
    std::thread::scope(|scope| {
        for value in values {
            scope.spawn(move || {
                assert_refused(
                    root,
                    &format!("docs: a change\n\nSome body prose.\n\n{key}: {value}\n"),
                    why,
                )
            });
        }
    });
}

fn assert_cleared(root: &Path, message: &str, why: &str) {
    let output = run_guard_over(root, message);
    assert!(
        output.status.success(),
        "the guard refused a commit that {why}. A guard that refuses what the rule permits, such as prose about attribution or a person's credit, blocks work it has no reason to block.\nmessage:\n{message}\nguard said:\n{}",
        said(&output)
    );
}

#[test]
fn refuses_a_model_trailer_in_the_closing_trailer_block() {
    check_workspace(|root| {
        assert_refused(
            root,
            MODEL_CO_AUTHOR,
            "closes with a trailer block naming a model",
        );
    });
}

#[test]
fn refuses_a_model_trailer_a_squash_merge_left_in_the_middle_of_the_body() {
    check_workspace(|root| {
        assert_refused(
            root,
            "docs: a squashed change (#2)\n\n* docs: the first commit\n\nSome body prose.\n\nCo-Authored-By: Claude (Fable 5)\n\n* docs: the second commit\n\nMore body prose.\n",
            "carries a model trailer in a paragraph the body continues past",
        );
    });
}

#[test]
fn refuses_a_model_trailer_a_squash_merge_left_before_a_human_trailer_block() {
    check_workspace(|root| {
        assert_refused(
            root,
            "docs: a squashed change (#2)\n\n* docs: the first commit\n\nCo-Authored-By: Claude (Fable 5)\n\n* docs: the second commit\n\nCo-Authored-By: A Colleague <colleague@corp.locale.io>\n",
            "credits a model mid-body and a person at the end, so only the person is visible to a parser that reads the last paragraph",
        );
    });
}

#[test]
fn refuses_a_model_trailer_between_two_peoples_trailers_in_one_block() {
    check_workspace(|root| {
        assert_refused(
            root,
            "docs: a change\n\nSome body prose.\n\nReviewed-by: A Colleague <colleague@corp.locale.io>\nCo-Authored-By: Claude <noreply@anthropic.com>\nSigned-off-by: A Colleague <colleague@corp.locale.io>\n",
            "credits a model in the middle of a trailer block whose first and last lines are people",
        );
    });
}

#[test]
fn refuses_a_model_sign_off_in_the_closing_trailer_block() {
    check_workspace(|root| {
        assert_refused(
            root,
            "docs: a change\n\nSome body prose.\n\nSigned-off-by: Claude <noreply@anthropic.com>\n",
            "closes with a sign-off naming a model",
        );
    });
}

#[test]
fn refuses_a_model_sign_off_a_squash_merge_left_mid_body() {
    check_workspace(|root| {
        assert_refused(
            root,
            "docs: a squashed change (#2)\n\n* docs: the first commit\n\nSome body prose.\n\nSigned-off-by: Claude <noreply@anthropic.com>\n\n* docs: the second commit\n\nMore body prose.\n",
            "carries a model's sign-off in a paragraph the body continues past",
        );
    });
}

#[test]
fn refuses_a_model_trailer_with_whitespace_before_its_colon() {
    check_workspace(|root| {
        assert_refused(
            root,
            "docs: a change\n\nSome body prose.\n\nCo-Authored-By : Claude <noreply@anthropic.com>\n",
            "puts a space before a model trailer's colon, which git still reads as a trailer",
        );
    });
}

#[test]
fn refuses_a_model_trailer_mid_body_when_the_message_uses_windows_line_endings() {
    check_workspace(|root| {
        assert_refused(
            root,
            "docs: a squashed change (#2)\r\n\r\n* docs: the first commit\r\n\r\nCo-Authored-By: Claude (Fable 5)\r\n\r\n* docs: the second commit\r\n\r\nMore body prose.\r\n",
            "carries a mid-body model trailer in a message written with CRLF line endings, where a paragraph break is a line holding a carriage return rather than an empty one",
        );
    });
}

#[test]
fn refuses_a_model_trailer_mid_body_when_paragraph_breaks_hold_whitespace() {
    check_workspace(|root| {
        assert_refused(
            root,
            "docs: a squashed change (#2)\n \n* docs: the first commit\n \nCo-Authored-By: Claude (Fable 5)\n \n* docs: the second commit\n",
            "carries a mid-body model trailer in a message whose paragraph breaks hold a space, which git itself reads as a break",
        );
    });
}

#[test]
fn clears_a_body_that_quotes_a_trailer_inside_a_sentence() {
    check_workspace(|root| {
        assert_cleared(
            root,
            "ci(guard): refuse a commit that credits a model as an author\n\nTwo commits carry a `Co-Authored-By: Claude (Fable 5)` line that the guard cannot see.\n",
            "mentions a trailer inside a sentence rather than writing one",
        );
    });
}

#[test]
fn clears_a_body_that_indents_a_trailer_as_an_example() {
    check_workspace(|root| {
        assert_cleared(
            root,
            "ci(guard): refuse a commit that credits a model as an author\n\nThe shape the guard now refuses:\n\n    Co-Authored-By: Claude (Fable 5)\n\nA model is a tool, and tools do not co-author.\n",
            "shows a trailer as an indented example",
        );
    });
}

#[test]
fn clears_a_paragraph_that_mixes_prose_with_a_quoted_trailer() {
    check_workspace(|root| {
        assert_cleared(
            root,
            "ci(guard): refuse a commit that credits a model as an author\n\nThe guard refuses this line:\nCo-Authored-By: Claude (Fable 5)\n",
            "puts a quoted trailer under a line of prose in the same paragraph",
        );
    });
}

#[test]
fn refuses_a_trailer_naming_a_vendor_whose_name_no_person_carries() {
    check_workspace(|root| {
        assert_refused_crediting_each(
            root,
            "Co-Authored-By",
            &[
                "Cursor Agent <cursoragent@cursor.com>",
                "Devin <devin@cognition.ai>",
                "Grok <noreply@x.ai>",
            ],
            "credits a coding agent this guard did not name before",
        );
    });
}

#[test]
fn refuses_a_trailer_with_no_address_for_each_model_word() {
    check_workspace(|root| {
        assert_refused_crediting_each(
            root,
            "Co-authored-by",
            &[
                "Claude",
                "Anthropic",
                "GPT",
                "OpenAI",
                "Copilot",
                "Codex",
                "Gemini",
                "Llama",
                "Mistral",
                "Cursor",
                "Devin (cognition.ai)",
                "Grok",
                "Agent (x.ai)",
            ],
            "credits a model by a name with no address, holding one word of the guard's vendor list and no other",
        );
    });
}

#[test]
fn refuses_a_tool_identity_under_a_neutral_name() {
    check_workspace(|root| {
        assert_refused_crediting_each(
            root,
            "Co-authored-by",
            &[
                "Probe <noreply@anthropic.com>",
                "Probe <198982749+Copilot@users.noreply.github.com>",
                "Probe <176961590+gemini-code-assist[bot]@users.noreply.github.com>",
                "Probe <noreply@x.ai>",
            ],
            "credits a model's own address under a name that holds no vendor word",
        );
    });
}

#[test]
fn clears_a_body_whose_only_co_authors_are_people() {
    check_workspace(|root| {
        assert_cleared(
            root,
            "feat: a change\n\n* feat: the first commit\n\nCo-Authored-By: A Colleague <colleague@corp.locale.io>\n\n* feat: the second commit\n\nCo-Authored-By: Another Colleague <another@corp.locale.io>\n",
            "credits two people and no model",
        );
    });
}

#[test]
fn clears_a_persons_sign_off() {
    check_workspace(|root| {
        assert_cleared(
            root,
            "docs: a change\n\nSome body prose.\n\nSigned-off-by: A Colleague <colleague@corp.locale.io>\n",
            "carries a person's sign-off, which spec 029 section 6 accepts",
        );
    });
}

#[test]
fn clears_an_assisted_by_disclosure_beside_a_persons_sign_off() {
    check_workspace(|root| {
        assert_cleared(
            root,
            "docs: a change\n\nSome body prose.\n\nAssisted-by: Claude <noreply@anthropic.com>\nSigned-off-by: A Colleague <colleague@corp.locale.io>\n",
            "discloses a model under Assisted-by, a key the guard does not read, in the trailer block that holds a person's sign-off",
        );
    });
}

#[test]
fn refuses_a_range_it_cannot_read() {
    check_workspace(|root| {
        let repository = repository_with_commit("docs: a change\n");
        assert_refused_saying(
            &run_guard_in(root, repository.path(), "no-such-ref..HEAD"),
            "cannot read the range",
            "a range it cannot read, which is a range it cannot clear",
        );
    });
}

#[test]
fn refuses_when_the_tool_it_matches_with_cannot_answer() {
    check_workspace(|root| {
        let repository = repository_with_commit(MODEL_CO_AUTHOR);
        let shadow = directory_shadowing("grep", "#!/bin/sh\nexit 2\n");
        assert_refused_saying(
            &run_guard_shadowed(root, repository.path(), shadow.path()),
            "grep exited",
            "a commit carrying a model trailer while grep fails (`grep -q` exits 1 for no match and 2 or more for an error, and reading an error as either answer is how a guard decides without having looked)",
        );
    });
}

#[test]
fn refuses_a_model_trailer_in_a_commit_behind_the_tip_of_the_range() {
    check_workspace(|root| {
        let repository = repository_with_commits(&[MODEL_CO_AUTHOR, "docs: a later change\n"]);
        assert_refused_saying(
            &run_guard_in(root, repository.path(), "HEAD"),
            OFFENCE,
            "a range whose model trailer is in a commit behind its tip",
        );
    });
}

#[test]
fn refuses_a_model_trailer_on_the_tip_of_the_range_above_a_clean_commit() {
    check_workspace(|root| {
        let repository = repository_with_commits(&["docs: an earlier change\n", MODEL_CO_AUTHOR]);
        assert_refused_saying(
            &run_guard_in(root, repository.path(), "HEAD"),
            OFFENCE,
            "a range whose model trailer is on its tip, above a clean commit the guard reads after it",
        );
    });
}

#[test]
fn refuses_a_model_trailer_in_a_merge_commits_message() {
    check_workspace(|root| {
        let repository = repository_with_a_merge(
            "docs: a change on a side branch\n",
            "Merge branch 'side'\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
        );
        assert_refused_saying(
            &run_guard_in(root, repository.path(), "HEAD"),
            OFFENCE,
            "a range whose merge commit's message credits a model",
        );
    });
}

#[test]
fn refuses_a_model_trailer_on_a_merged_side_branch() {
    check_workspace(|root| {
        let repository = repository_with_a_merge(MODEL_CO_AUTHOR, "Merge branch 'side'\n");
        assert_refused_saying(
            &run_guard_in(root, repository.path(), "HEAD"),
            OFFENCE,
            "a range whose model trailer is on a side branch that a merge brought in",
        );
    });
}

#[test]
fn refuses_a_model_trailer_after_origin_main_when_given_no_range() {
    check_workspace(|root| {
        let repository =
            repository_with_commits(&["docs: a change main already has\n", MODEL_CO_AUTHOR]);
        git_succeeds(
            repository.path(),
            &["update-ref", "refs/remotes/origin/main", "HEAD~1"],
            None,
        );
        assert_refused_saying(
            &guard_command(root, repository.path())
                .output()
                .expect("the guard runs"),
            OFFENCE,
            "a model trailer after origin/main when it runs with no range, as the root gate runs it",
        );
    });
}

#[test]
fn refuses_a_model_trailer_on_head_when_given_no_range_and_there_is_no_origin_main() {
    check_workspace(|root| {
        let repository = repository_with_commit(MODEL_CO_AUTHOR);
        assert_refused_saying(
            &guard_command(root, repository.path())
                .output()
                .expect("the guard runs"),
            OFFENCE,
            "a model trailer on HEAD when it runs with no range and there is no origin/main (this pins today's fallback to all of HEAD, which spec 029 does not state; refusing the missing origin/main as an unreadable range would also fail closed)",
        );
    });
}

#[test]
fn refuses_a_commit_whose_message_it_cannot_read() {
    check_workspace(|root| {
        let repository = repository_with_commit("docs: a change\n");
        assert_refused_saying(
            &run_guard_with_git_refusing(root, repository.path(), "--format=%B"),
            "cannot read the message of",
            "a commit while `git show` cannot read its message, which is a message it cannot clear",
        );
    });
}

#[test]
fn refuses_a_message_whose_trailers_it_cannot_parse() {
    check_workspace(|root| {
        let repository = repository_with_commit(
            "docs: a change\n\nSome body prose.\n\nCo-Authored-By: A Colleague <colleague@corp.locale.io>\n",
        );
        assert_refused_saying(
            &run_guard_with_git_refusing(root, repository.path(), "interpret-trailers"),
            "cannot parse the trailers",
            "a commit crediting a person while `git interpret-trailers` cannot parse its trailers, which are trailers it cannot clear",
        );
    });
}

#[test]
fn clears_a_body_larger_than_a_pipe_buffer_whose_first_line_is_trailer_shaped() {
    check_workspace(|root| {
        let mut message = String::from("ci(guard): a body that outgrows a pipe buffer\n\n");
        message.push_str("Co-Authored-By: A Person <person@corp.locale.io>\n");
        for _ in 0..20_000 {
            message.push_str("padding that carries the body past the pipe buffer\n");
        }
        let output = run_guard_over(root, &message);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "the guard refused a commit whose only trailer is well-formed; a guard that refuses a \
             message for its size refuses at random.\nguard stdout:\n{stdout}\nguard stderr:\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            !stdout.contains("grep exited"),
            "the guard called its own tooling unreadable on a body it can read; a guard that refuses \
             because a writer it spawned took SIGPIPE refuses at random.\nguard said:\n{stdout}",
        );
    });
}

#[test]
fn tells_the_contributor_how_to_fix_an_untraceable_trailer() {
    check_workspace(|root| {
        assert_refused_saying(
            &run_guard_over(root, UNTRACEABLE_CO_AUTHOR),
            FIX,
            "a commit crediting an address on `.local`, whose contributor has to be told what to change",
        );
    });
}

fn assert_refused_when_grep_fails_on_the_address(root: &Path, arms: &str, check: &str) {
    let repository = repository_with_commit(UNTRACEABLE_CO_AUTHOR);
    let grep = real_tool("grep");
    let stub = format!(
        "#!/bin/sh\ninput=$(cat)\ncase \"$*\" in\n{arms}\nesac\nprintf '%s\\n' \"$input\" | exec {} \"$@\"\n",
        shell_quoted(grep.to_str().expect("the path to grep is UTF-8"))
    );
    let shadow = directory_shadowing("grep", &stub);
    let output = run_guard_shadowed(root, repository.path(), shadow.path());
    assert_refused_saying(
        &output,
        "grep exited 2",
        &format!("a commit crediting `me@laptop.local` while grep fails only on {check}"),
    );
    assert!(
        !said(&output).contains("is untraceable"),
        "the guard reported the address untraceable although grep could not answer {check}, so that check never asked grep.\nguard said:\n{}",
        said(&output)
    );
}

#[test]
fn judges_a_domain_through_the_matcher_it_checks() {
    check_workspace(|root| {
        assert_refused_when_grep_fails_on_the_address(
            root,
            "  *localdomain*) [ \"$input\" = me@laptop.local ] && exit 2 ;;",
            "the domain check (if the guard's domain pattern no longer holds `localdomain`, point the stub at the new pattern; that alone is not a guard bug)",
        )
    });
}

#[test]
fn judges_whether_there_is_an_address_through_the_matcher_it_checks() {
    check_workspace(|root| {
        assert_refused_when_grep_fails_on_the_address(
            root,
            "  *localdomain*) ;;\n  *) [ \"$input\" = me@laptop.local ] && exit 2 ;;",
            "the check that the address holds an `@`, the one check other than the domain's that the guard runs on the bare address",
        )
    });
}

#[derive(Clone, Copy)]
enum Position {
    CoAuthoredBy,
    SignedOffBy,
}

impl Position {
    fn as_written(self) -> &'static str {
        match self {
            Position::CoAuthoredBy => "Co-Authored-By",
            Position::SignedOffBy => "Signed-Off-By",
        }
    }

    fn as_printed(self) -> &'static str {
        match self {
            Position::CoAuthoredBy => "Co-authored-by",
            Position::SignedOffBy => "Signed-off-by",
        }
    }
}

#[derive(Clone, Copy)]
enum Verdict {
    Model,
    Untraceable,
    Cleared,
}

impl Verdict {
    fn refusal(self, position: Position) -> Option<&'static str> {
        match (self, position) {
            (Verdict::Model, Position::CoAuthoredBy) => {
                Some("credits a model: GitHub shows it on main as a co-author.")
            }
            (Verdict::Model, Position::SignedOffBy) => Some(
                "credits a model: a sign-off certifies where a change came from, which a tool cannot do.",
            ),
            (Verdict::Untraceable, _) => Some(
                "is untraceable: no account can be found for it, so the record on main names somebody no reader can trace.",
            ),
            (Verdict::Cleared, _) => None,
        }
    }
}

const IDENTITIES: &[(&str, Verdict)] = &[
    ("Probe <probe@local>", Verdict::Untraceable),
    ("Probe <probe@localhost>", Verdict::Untraceable),
    ("Probe <probe@invalid>", Verdict::Untraceable),
    ("Probe <probe@test>", Verdict::Untraceable),
    ("Probe <probe@example>", Verdict::Untraceable),
    ("Probe <probe@example.com>", Verdict::Untraceable),
    ("Probe <probe@example.net>", Verdict::Untraceable),
    ("Probe <probe@example.org>", Verdict::Untraceable),
    ("Probe <probe@home.arpa>", Verdict::Untraceable),
    ("Probe <probe@internal>", Verdict::Untraceable),
    ("Probe <probe@localdomain>", Verdict::Untraceable),
    ("Probe <me@laptop.local.>", Verdict::Untraceable),
    ("Probe <me@LAPTOP.LOCAL>", Verdict::Untraceable),
    ("Exec025 <exec025@localhost>", Verdict::Untraceable),
    ("Probe <probe@ci.example.com>", Verdict::Untraceable),
    ("Claude (Fable 5) <>", Verdict::Model),
    ("Claude: Fable 5", Verdict::Model),
    ("Jane Doe <>", Verdict::Untraceable),
    ("Jane Doe <jane>", Verdict::Untraceable),
    ("Jane Doe", Verdict::Untraceable),
    ("Claudette Roe <>", Verdict::Untraceable),
    ("Probe <claude>", Verdict::Untraceable),
    ("ChatGPT", Verdict::Untraceable),
    (
        "Jane Doe <jane@corp.locale.io> <jane@laptop.local>",
        Verdict::Untraceable,
    ),
    (
        "Jane Doe <jane@laptop.local> <jane@corp.locale.io>",
        Verdict::Cleared,
    ),
    ("Jane Doe <jane@corp.locale.io> <", Verdict::Untraceable),
    ("Jane Doe <jane@corp.locale.io", Verdict::Untraceable),
    ("QA <qa@ci.tests.org>", Verdict::Cleared),
    ("Jane Doe <jane@corp.locale.io>", Verdict::Cleared),
    ("Probe <probe@notlocal>", Verdict::Cleared),
    ("Probe <probe@example.community>", Verdict::Cleared),
    ("Probe <probe@te\u{17f}t>", Verdict::Cleared),
    (
        "dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
        Verdict::Cleared,
    ),
    ("dependabot[bot] <support@github.com>", Verdict::Cleared),
    ("Probe Person <probe@corp.locale.io>", Verdict::Cleared),
];

fn probe_shas(repository: &Path) -> Vec<String> {
    let output = git(repository, &["log", "--format=%H %s"], None);
    assert!(
        output.status.success(),
        "setup: `git log` failed in the scratch repository: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut shas = vec![String::new(); IDENTITIES.len()];
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let (sha, subject) = line
            .split_once(' ')
            .expect("setup: `git log` prints a SHA and a subject");
        let index: usize = subject
            .strip_prefix("probe ")
            .and_then(|index| index.parse().ok())
            .expect("setup: every scratch commit is a probe");
        shas[index] = sha[..9].to_owned();
    }
    shas
}

fn assert_each_identity_judged_in(root: &Path, position: Position) {
    let messages: Vec<String> = IDENTITIES
        .iter()
        .enumerate()
        .map(|(index, (identity, _))| {
            format!("probe {index}\n\n{}: {identity}\n", position.as_written())
        })
        .collect();
    let repository =
        repository_with_commits(&messages.iter().map(String::as_str).collect::<Vec<_>>());
    let output = guard_command(root, repository.path())
        .env("LC_ALL", "C.UTF-8")
        .arg("HEAD")
        .output()
        .expect("the guard runs");
    let said = said(&output);
    let shas = probe_shas(repository.path());
    let misjudged: Vec<String> = IDENTITIES
        .iter()
        .zip(&shas)
        .filter_map(|((identity, verdict), sha)| {
            let naming = format!("attribution-guard: {sha} ");
            let lines: Vec<&str> = said
                .lines()
                .filter(|line| line.starts_with(&naming))
                .collect();
            match verdict.refusal(position) {
                Some(refusal) => {
                    let expected =
                        format!("{naming}{} \"{identity}\" {refusal}", position.as_printed());
                    (lines != [expected.as_str()]).then(|| {
                        format!("{identity:?} must be refused by exactly one line, `{expected}`; the lines naming its commit are {lines:?}")
                    })
                }
                None => (!lines.is_empty()).then(|| {
                    format!("{identity:?} must be cleared, yet the guard named its commit: {lines:?}")
                }),
            }
        })
        .collect();
    assert!(
        misjudged.is_empty(),
        "as {}, these identities got the wrong verdict:\n{}\nguard said:\n{said}",
        position.as_written(),
        misjudged.join("\n")
    );
    assert!(
        !output.status.success(),
        "the guard printed every refusal above but exited successfully, which lets them all through.\nguard said:\n{said}"
    );
}

#[test]
fn judges_each_identity_as_a_co_author() {
    check_workspace(|root| assert_each_identity_judged_in(root, Position::CoAuthoredBy));
}

#[test]
fn judges_each_identity_as_a_sign_off() {
    check_workspace(|root| assert_each_identity_judged_in(root, Position::SignedOffBy));
}
