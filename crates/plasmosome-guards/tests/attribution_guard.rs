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

const QUOTED_TRAILERS: [&str; 2] = [
    "Co-Authored-By: Claude (Fable 5)",
    "Co-Authored-By: Probe <noreply@anthropic.com>",
];

#[test]
fn clears_a_body_that_quotes_a_trailer_inside_a_sentence() {
    check_workspace(|root| {
        for quoted in QUOTED_TRAILERS {
            assert_cleared(
                root,
                &format!(
                    "ci(guard): refuse a commit that credits a model as an author\n\nTwo commits carry a `{quoted}` line that the guard cannot see.\n"
                ),
                "mentions a trailer inside a sentence rather than writing one",
            );
        }
    });
}

#[test]
fn clears_a_body_that_indents_a_trailer_as_an_example() {
    check_workspace(|root| {
        for quoted in QUOTED_TRAILERS {
            assert_cleared(
                root,
                &format!(
                    "ci(guard): refuse a commit that credits a model as an author\n\nThe shape the guard now refuses:\n\n    {quoted}\n\nA model is a tool, and tools do not co-author.\n"
                ),
                "shows a trailer as an indented example",
            );
        }
    });
}

#[test]
fn clears_a_paragraph_that_mixes_prose_with_a_quoted_trailer() {
    check_workspace(|root| {
        for quoted in QUOTED_TRAILERS {
            assert_cleared(
                root,
                &format!(
                    "ci(guard): refuse a commit that credits a model as an author\n\nThe guard refuses this line:\n{quoted}\n"
                ),
                "puts a quoted trailer under a line of prose in the same paragraph",
            );
        }
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
fn refuses_a_model_tools_suggestion_taken_with_githubs_co_author_line() {
    check_workspace(|root| {
        assert_refused(
            root,
            "docs: a change\n\nSome body prose.\n\nCo-authored-by: coderabbitai[bot] <136622811+coderabbitai[bot]@users.noreply.github.com>\n",
            "closes with the line GitHub's suggestion button adds for CodeRabbit, a model tool behind a bot account, which the tool list decides before any bot is cleared",
        );
    });
}

#[test]
fn clears_dependabots_sign_off_and_co_author_in_one_closing_block() {
    check_workspace(|root| {
        assert_cleared(
            root,
            "chore(deps): bump actions/checkout (#22)\n\nBumps the actions group with 1 update in the / directory: [actions/checkout](https://github.com/actions/checkout).\n\n\nUpdates `actions/checkout` from 5.1.0 to 7.0.1\n- [Release notes](https://github.com/actions/checkout/releases)\n\n---\nupdated-dependencies:\n- dependency-name: actions/checkout\n  dependency-version: 7.0.1\n  dependency-type: direct:production\n...\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>\n",
            "closes as #22's squash did, with Dependabot's sign-off and GitHub's co-author line for it in one block. Dependabot is a bot that is not a model tool, and spec 029 section 5 clears it until the owner answers Q1",
        );
    });
}

#[test]
fn names_the_canonical_position_however_the_key_is_spelled() {
    check_workspace(|root| {
        let value = "Probe <noreply@anthropic.com>";
        let probes: Vec<Probe> = [
            ("CO-AUTHORED-BY : ", Position::CoAuthoredBy),
            ("signed-off-by:", Position::SignedOffBy),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (key, position))| {
            let line = format!("{key}{value}");
            Probe {
                label: format!("{line:?}"),
                message: format!("probe {index}\n\n{line}\n"),
                refusal: Verdict::Model
                    .refusal(position)
                    .map(|refusal| format!("{} \"{value}\" {refusal}", position.as_printed())),
            }
        })
        .collect();
        assert_each_probe_judged(
            root,
            "Git reads a trailer key in any case, with whitespace before its colon or none after it, and the refusal names the position as spec 029 spells it, so",
            &probes,
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

fn assert_refused_when_grep_fails_on_the_address(root: &Path, arms: &str, check: &str) -> Output {
    assert_refused_when_grep_fails_judging(
        root,
        UNTRACEABLE_CO_AUTHOR,
        ("me@laptop.local", "is untraceable"),
        arms,
        check,
    )
}

fn assert_refused_when_grep_fails_judging(
    root: &Path,
    message: &str,
    (address, verdict): (&str, &str),
    arms: &str,
    check: &str,
) -> Output {
    let repository = repository_with_commit(message);
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
        &format!("a commit crediting `{address}` while grep fails only on {check}"),
    );
    assert!(
        !said(&output).contains(verdict),
        "the guard said `{verdict}` about `{address}` although grep could not answer {check}, so that check never asked grep.\nguard said:\n{}",
        said(&output)
    );
    let unreadable: Vec<String> = output
        .stdout
        .split(|&byte| byte == b'\n')
        .filter(|line| line.windows(11).any(|word| word == b"grep exited"))
        .filter(|line| !line.iter().all(|byte| (b' '..=b'~').contains(byte)))
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .collect();
    assert!(
        unreadable.is_empty(),
        "the line naming grep's failure holds a byte outside printable ASCII while grep fails only on {check}. A carriage return there sends a terminal back to the line's start, and an invisible space hides a byte, so the reader cannot see the pattern that failed: {unreadable:?}"
    );
    output
}

#[test]
fn judges_a_domain_through_the_matcher_it_checks() {
    check_workspace(|root| {
        assert_refused_when_grep_fails_on_the_address(
            root,
            "  *localdomain*) [ \"$input\" = me@laptop.local ] && exit 2 ;;",
            "the domain check (if the guard's domain pattern no longer holds `localdomain`, point the stub at the new pattern; that alone is not a guard bug)",
        );
    });
}

#[test]
fn judges_whether_there_is_an_address_through_the_matcher_it_checks() {
    check_workspace(|root| {
        assert_refused_when_grep_fails_on_the_address(
            root,
            r#"  '-qiE @') [ "$input" = me@laptop.local ] && exit 2 ;;"#,
            "the check that the address holds an `@`, which grep runs with the pattern `@` alone (if the guard asks that question another way, point the stub at it; that alone is not a guard bug)",
        );
    });
}

#[test]
fn judges_a_malformed_address_through_the_matcher_it_checks() {
    check_workspace(|root| {
        let output = assert_refused_when_grep_fails_on_the_address(
            root,
            r#"  *'\.\.'*) [ "$input" = me@laptop.local ] && exit 2 ;;"#,
            "the check that the address is malformed (if the guard's malformed pattern no longer holds two escaped dots in a row, point the stub at a fragment only that pattern holds; that alone is not a guard bug)",
        );
        let escaped = r"|[ \x09\x0d\x0b\x0c]|\xc2[\x85\xa0]|\xe1\x9a\x80|";
        assert!(
            said(&output).contains(escaped),
            "the line naming grep's failure must write each byte of the malformed pattern outside printable ASCII as a backslash, an `x` and two hex digits, so it shows `{escaped}`.\nguard said:\n{}",
            said(&output)
        );
    });
}

#[test]
fn judges_a_tool_address_through_the_matcher_it_checks() {
    check_workspace(|root| {
        assert_refused_when_grep_fails_judging(
            root,
            "docs: a change\n\nSome body prose.\n\nCo-authored-by: Probe <noreply@anthropic.com>\n",
            ("noreply@anthropic.com", "credits a model"),
            r#"  *'noreply@anthropic\.com'*) [ "$input" = noreply@anthropic.com ] && exit 2 ;;"#,
            "the tool address list, the one pattern holding `noreply@anthropic\\.com` (the `@` and malformed checks before it run the real grep; if the list is matched another way, point the stub at it; that alone is not a guard bug)",
        );
    });
}

#[test]
fn judges_a_tool_login_through_the_matcher_it_checks() {
    check_workspace(|root| {
        assert_refused_when_grep_fails_judging(
            root,
            "docs: a change\n\nSome body prose.\n\nCo-authored-by: Probe <81847+claude@users.noreply.github.com>\n",
            ("81847+claude@users.noreply.github.com", "credits a model"),
            r#"  *'users\.noreply\.github\.com'*) [ "$input" = 81847+claude@users.noreply.github.com ] && exit 2 ;;"#,
            "the tool login list, the one pattern holding `users\\.noreply\\.github\\.com` (if the logins are matched another way, point the stub at it; that alone is not a guard bug)",
        );
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

const CO_AUTHOR_MODEL: &str = "credits a model: the squash keeps the line, and GitHub shows a co-author for it when it can resolve the address to an account.";

impl Verdict {
    fn refusal(self, position: Position) -> Option<&'static str> {
        match (self, position) {
            (Verdict::Model, Position::CoAuthoredBy) => Some(CO_AUTHOR_MODEL),
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
    ("Probe <me@host.ci.internal>", Verdict::Untraceable),
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
    (
        "Probe <me@laptop.local>me@corp.locale.io>",
        Verdict::Untraceable,
    ),
    ("Jane Doe jane@corp.locale.io>", Verdict::Untraceable),
    (
        "Probe <me@corp.locale.io@laptop.local>",
        Verdict::Untraceable,
    ),
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
    ("Probe <noreply@anthropic.com>", Verdict::Model),
    ("Probe <cursoragent@cursor.com>", Verdict::Model),
    ("Probe <codex@openai.com>", Verdict::Model),
    ("Probe <devin@cognition.ai>", Verdict::Model),
    ("Probe <noreply@x.ai>", Verdict::Model),
    (
        "Probe <81847+claude@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <209825114+claude[bot]@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <198982749+Copilot@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <175728472+Copilot@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <136622811+coderabbitai[bot]@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <161369871+google-labs-jules[bot]@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <176961590+gemini-code-assist[bot]@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <158243242+devin-ai-integration[bot]@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <199175422+chatgpt-codex-connector[bot]@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <215057067+openai-codex[bot]@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <206951365+cursor[bot]@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <199161495+cursoragent@users.noreply.github.com>",
        Verdict::Model,
    ),
    ("Claude <NOREPLY@ANTHROPIC.COM>", Verdict::Model),
    (
        "Probe <claude[bot]@users.noreply.github.com>",
        Verdict::Model,
    ),
    (
        "Probe <209825114+CLAUDE[BOT]@USERS.NOREPLY.GITHUB.COM>",
        Verdict::Model,
    ),
    ("Jane Doe <jane@anthropic.com>", Verdict::Cleared),
    ("Sam Roe <sam@cursor.com>", Verdict::Cleared),
    (
        "Claude Monet <claude.monet@corp.locale.io>",
        Verdict::Cleared,
    ),
    ("Alex Aitken <alex.aitken@corp.locale.io>", Verdict::Cleared),
    ("Ana Gemini <ana@corp.locale.io>", Verdict::Cleared),
    (
        "Jane Doe <4242+janedoe@users.noreply.github.com>",
        Verdict::Cleared,
    ),
    ("Kevin Devin <kevin.devin@cognition.ai>", Verdict::Cleared),
    (
        "Claude Monet <81848+claudemonet@users.noreply.github.com>",
        Verdict::Cleared,
    ),
    ("Gemini <probe@corp.locale.io>", Verdict::Cleared),
    ("Copilot <probe@corp.locale.io>", Verdict::Cleared),
    ("Probe <noreply@anthropic.com.evil.io>", Verdict::Cleared),
    (
        "Probe <81847+claude@users.noreply.github.com.evil.io>",
        Verdict::Cleared,
    ),
    ("Probe <xnoreply@anthropic.com>", Verdict::Cleared),
    ("Probe <noreply@anthropicxcom>", Verdict::Cleared),
    ("Probe <noreply@anthropic.com.>", Verdict::Cleared),
    ("Probe <claudet@users.noreply.github.com>", Verdict::Cleared),
    (
        "Jane Doe <4242+janedoe-claude@users.noreply.github.com>",
        Verdict::Cleared,
    ),
    (
        "Probe <81847+claude@users.noreply.githubxcom>",
        Verdict::Cleared,
    ),
    (
        "Jane <jane@corp.locale.io> <noreply@anthropic.com>",
        Verdict::Model,
    ),
    (
        "Claude <noreply@anthropic.com> <jane@corp.locale.io>",
        Verdict::Cleared,
    ),
];

fn identities() -> Vec<(String, Verdict)> {
    let letter = |point: u32| {
        char::from_u32(point)
            .unwrap_or_else(|| panic!("setup: {point:#x} is a Unicode scalar value"))
    };
    let mut rows = owned(IDENTITIES);
    rows.push((
        format!("Claude{}o Roe <>", letter(0xf1)),
        Verdict::Untraceable,
    ));
    rows.push((
        format!("Jos{}gemini Roe <>", letter(0xe9)),
        Verdict::Untraceable,
    ));
    rows
}

const PROBES_PER_REPOSITORY: usize = 8;

struct Probe {
    label: String,
    message: String,
    refusal: Option<String>,
}

struct ProbeRun {
    shas: Vec<String>,
    said: String,
    refused: bool,
}

fn probe_repository(messages: &[&str]) -> (TempDir, Vec<String>) {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let path = directory.path();
    git_succeeds(path, &["init", "--quiet"], None);
    let mut stream = String::new();
    for (index, message) in messages.iter().enumerate() {
        stream.push_str(&format!(
            "commit refs/heads/probe\nmark :{}\ncommitter A Person <person@corp.locale.io> 1700000000 +0000\ndata {}\n{message}\n",
            index + 1,
            message.len()
        ));
    }
    let marks = path.join("probe-marks");
    let export = format!("--export-marks={}", marks.display());
    git_succeeds(path, &["fast-import", "--quiet", &export], Some(&stream));
    let marks = std::fs::read_to_string(&marks).expect("setup: fast-import wrote its marks");
    let mut shas = vec![String::new(); messages.len()];
    for line in marks.lines() {
        let (mark, sha) = line
            .split_once(' ')
            .expect("setup: a mark line holds a mark and a SHA");
        let index: usize = mark
            .strip_prefix(':')
            .and_then(|mark| mark.parse().ok())
            .expect("setup: fast-import numbers its marks");
        shas[index - 1] = sha[..9].to_owned();
    }
    (directory, shas)
}

fn run_guard_over_probes(root: &Path, probes: &[Probe]) -> ProbeRun {
    let messages: Vec<&str> = probes.iter().map(|probe| probe.message.as_str()).collect();
    let (repository, shas) = probe_repository(&messages);
    let output = guard_command(root, repository.path())
        .env_remove("LC_ALL")
        .env_remove("LC_CTYPE")
        .env("LANG", "C.UTF-8")
        .arg("probe")
        .output()
        .expect("the guard runs");
    ProbeRun {
        shas,
        said: said(&output),
        refused: !output.status.success(),
    }
}

fn assert_each_probe_judged(root: &Path, what: &str, probes: &[Probe]) {
    let runs: Vec<ProbeRun> = std::thread::scope(|scope| {
        let handles: Vec<_> = probes
            .chunks(PROBES_PER_REPOSITORY)
            .map(|chunk| scope.spawn(move || run_guard_over_probes(root, chunk)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("a probe run finishes"))
            .collect()
    });
    let mut misjudged = Vec::new();
    for (run, chunk) in runs.iter().zip(probes.chunks(PROBES_PER_REPOSITORY)) {
        for (probe, sha) in chunk.iter().zip(&run.shas) {
            let naming = format!("attribution-guard: {sha} ");
            let lines: Vec<&str> = run
                .said
                .lines()
                .filter(|line| line.starts_with(&naming))
                .collect();
            let label = &probe.label;
            match &probe.refusal {
                Some(refusal) => {
                    let expected = format!("{naming}{refusal}");
                    if lines != [expected.as_str()] {
                        misjudged.push(format!("{label} must be refused by exactly one line, `{expected}`; the lines naming its commit are {lines:?}"));
                    }
                }
                None => {
                    if !lines.is_empty() {
                        misjudged.push(format!(
                            "{label} must be cleared, yet the guard named its commit: {lines:?}"
                        ));
                    }
                }
            }
        }
        let holds_a_refusal = chunk.iter().any(|probe| probe.refusal.is_some());
        if run.refused != holds_a_refusal {
            let labels: Vec<&str> = chunk.iter().map(|probe| probe.label.as_str()).collect();
            misjudged.push(if holds_a_refusal {
                format!("the guard exited successfully over {labels:?}, which lets every refusal among them through")
            } else {
                format!("the guard failed over {labels:?}, none of which it may refuse")
            });
        }
    }
    let said: Vec<&str> = runs.iter().map(|run| run.said.as_str()).collect();
    assert!(
        misjudged.is_empty(),
        "{what}, these probes got the wrong verdict:\n{}\nguard said:\n{}",
        misjudged.join("\n"),
        said.join("\n")
    );
}

fn rule_2_whitespace() -> Vec<char> {
    let characters: Vec<char> = [0x20, 0x09, 0x0d, 0x0b, 0x0c, 0x85, 0xa0, 0x1680]
        .into_iter()
        .chain(0x2000..=0x200a)
        .chain([0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff])
        .map(|point| {
            char::from_u32(point)
                .unwrap_or_else(|| panic!("setup: {point:#x} is a Unicode scalar value"))
        })
        .collect();
    assert_eq!(
        characters.len(),
        25,
        "setup: spec 029 section 4 rule 2 lists 25 whitespace characters"
    );
    characters
}

fn owned(rows: &[(&str, Verdict)]) -> Vec<(String, Verdict)> {
    rows.iter()
        .map(|&(identity, verdict)| (identity.to_owned(), verdict))
        .collect()
}

const RULE_2_ROWS_BEYOND_ACCEPTANCE_1: &[(&str, Verdict)] = &[
    ("Probe <claude@>", Verdict::Untraceable),
    ("Probe <noreply@anthropic.com >", Verdict::Untraceable),
    ("Claudette Roe <@laptop.local>", Verdict::Untraceable),
    ("Probe <probe@@corp.locale.io>", Verdict::Cleared),
    ("Probe <@@corp.locale.io>", Verdict::Cleared),
    ("Probe <probe@corp@.locale.io>", Verdict::Untraceable),
    ("Jane Doe <jane..doe@corp.locale.io>", Verdict::Cleared),
    ("Probe <.probe.@corp.locale.io>", Verdict::Cleared),
];

fn rule_2_rows() -> Vec<(String, Verdict)> {
    let mut rows = malformed_addresses();
    rows.extend(owned(RULE_2_ROWS_BEYOND_ACCEPTANCE_1));
    rows
}

fn malformed_addresses() -> Vec<(String, Verdict)> {
    let no_break_space = char::from_u32(0xa0).expect("setup: U+00A0 is a Unicode scalar value");
    let mut rows = owned(&[
        ("Claude <@>", Verdict::Model),
        ("Claude <noreply@anthropic.com >", Verdict::Model),
    ]);
    rows.push((
        format!("Claude <noreply@anthropic.com{no_break_space}>"),
        Verdict::Model,
    ));
    rows.extend(owned(&[
        ("Claude <@laptop.local>", Verdict::Model),
        ("Probe <probe@>", Verdict::Untraceable),
        ("Probe <probe@.>", Verdict::Untraceable),
        ("Probe <@>", Verdict::Untraceable),
        ("Probe <@corp.locale.io>", Verdict::Untraceable),
        ("Probe < probe@corp.locale.io>", Verdict::Untraceable),
        ("Probe <probe @corp.locale.io>", Verdict::Untraceable),
        ("Jane Doe <jane doe@corp.locale.io>", Verdict::Untraceable),
        ("Probe <probe@corp .locale.io>", Verdict::Untraceable),
    ]));
    rows.extend(rule_2_whitespace().into_iter().map(|blank| {
        (
            format!("Probe <probe@corp.locale.io{blank}>"),
            Verdict::Untraceable,
        )
    }));
    rows.extend(owned(&[
        ("Probe <probe@corp.locale.io..>", Verdict::Untraceable),
        ("Probe <probe@.corp.locale.io>", Verdict::Untraceable),
        ("Probe <probe@corp..locale.io>", Verdict::Untraceable),
        ("Probe <probe@corp.locale.io.>", Verdict::Cleared),
        ("Jane Doe <jane.doe@corp.locale.io>", Verdict::Cleared),
    ]));
    assert_eq!(
        rows.len(),
        42,
        "setup: spec 029 Acceptance 1 lists 42 malformed address rows"
    );
    rows
}

fn assert_each_identity_judged_in<Identity: AsRef<str>>(
    root: &Path,
    position: Position,
    identities: &[(Identity, Verdict)],
) {
    let probes: Vec<Probe> = identities
        .iter()
        .enumerate()
        .map(|(index, (identity, verdict))| {
            let identity = identity.as_ref();
            Probe {
                label: format!("{identity:?}"),
                message: format!("probe {index}\n\n{}: {identity}\n", position.as_written()),
                refusal: verdict
                    .refusal(position)
                    .map(|refusal| format!("{} \"{identity}\" {refusal}", position.as_printed())),
            }
        })
        .collect();
    assert_each_probe_judged(root, &format!("as {}", position.as_written()), &probes);
}

#[test]
fn judges_each_identity_as_a_co_author() {
    check_workspace(|root| {
        assert_each_identity_judged_in(root, Position::CoAuthoredBy, &identities())
    });
}

#[test]
fn judges_each_identity_as_a_sign_off() {
    check_workspace(|root| {
        assert_each_identity_judged_in(root, Position::SignedOffBy, &identities())
    });
}

#[test]
fn judges_each_malformed_address_as_a_co_author() {
    check_workspace(|root| {
        assert_each_identity_judged_in(root, Position::CoAuthoredBy, &rule_2_rows())
    });
}

#[test]
fn judges_each_malformed_address_as_a_sign_off() {
    check_workspace(|root| {
        assert_each_identity_judged_in(root, Position::SignedOffBy, &rule_2_rows())
    });
}

#[test]
fn splits_a_paragraph_only_at_a_line_git_reads_as_blank() {
    check_workspace(|root| {
        let model = format!(
            "Co-authored-by \"Gemini <176961590+gemini-code-assist[bot]@users.noreply.github.com>\" {CO_AUTHOR_MODEL}"
        );
        let lines: &[(&str, &str, bool)] = &[
            ("U+3000, an ideographic space", "\u{3000}", true),
            ("U+2003, an em space", "\u{2003}", true),
            ("U+00A0, a no-break space", "\u{a0}", true),
            ("a vertical tab", "\u{b}", true),
            ("a form feed", "\u{c}", true),
            ("a space", " ", false),
            ("a tab", "\t", false),
            ("a carriage return", "\r", false),
        ];
        let probes: Vec<Probe> = lines
            .iter()
            .enumerate()
            .map(|(index, (name, line, git_reads_text))| Probe {
                label: format!("a line holding only {name}"),
                message: format!("probe {index}\n\nProse A\nProse B\nProse C\nCo-authored-by: Gemini <176961590+gemini-code-assist[bot]@users.noreply.github.com>\n{line}\nSigned-off-by: A Colleague <colleague@corp.locale.io>\n"),
                refusal: git_reads_text.then(|| model.clone()),
            })
            .collect();
        assert_each_probe_judged(
            root,
            "Git ends a paragraph only at a line holding nothing but spaces, tabs and carriage returns. A model's co-author line above a person's sign-off is a trailer unless such a line comes between them, so",
            &probes,
        );
    });
}

#[test]
fn reads_a_paragraph_both_ways_when_a_line_is_blank_only_to_another_reader() {
    check_workspace(|root| {
        let model =
            "Co-authored-by: Gemini <176961590+gemini-code-assist[bot]@users.noreply.github.com>";
        let refused: &str = &format!(
            "Co-authored-by \"Gemini <176961590+gemini-code-assist[bot]@users.noreply.github.com>\" {CO_AUTHOR_MODEL}"
        );
        let person = "Co-authored-by: A Colleague <colleague@corp.locale.io>";
        let sign_off = "Signed-off-by: A Colleague <colleague@corp.locale.io>";
        let untraceable = "Co-authored-by: Probe <me@laptop.local>";
        let untraceable_refused = "Co-authored-by \"Probe <me@laptop.local>\" is untraceable: no account can be found for it, so the record on main names somebody no reader can trace.";
        let prose = "Some prose that follows.";
        let ambiguous: &[(&str, &str)] = &[
            ("U+3000, an ideographic space", "\u{3000}"),
            ("U+2003, an em space", "\u{2003}"),
            ("U+00A0, a no-break space", "\u{a0}"),
            ("a vertical tab", "\u{b}"),
            ("a form feed", "\u{c}"),
        ];
        let mut rows: Vec<(String, String, Option<&str>)> = Vec::new();
        for (name, line) in ambiguous {
            rows.push((
                format!("a model co-author, a line holding only {name}, then prose"),
                format!("{model}\n{line}\n{prose}\n"),
                Some(refused),
            ));
            rows.push((
                format!("a person's co-author, a line holding only {name}, then prose"),
                format!("{person}\n{line}\n{prose}\n"),
                None,
            ));
            rows.push((
                format!("a model co-author, a line holding only {name}, then a person's sign-off"),
                format!("{model}\n{line}\n{sign_off}\n"),
                Some(refused),
            ));
            rows.push((
                format!("prose, a line holding only {name}, then a model co-author"),
                format!("{prose}\n{line}\n{model}\n"),
                Some(refused),
            ));
        }
        rows.push((
            "an untraceable co-author, a line holding only U+3000, then prose".into(),
            format!("{untraceable}\n\u{3000}\n{prose}\n"),
            Some(untraceable_refused),
        ));
        rows.push((
            "a model co-author, a line of a space, U+3000 and a tab, then prose".into(),
            format!("{model}\n \u{3000}\t\n{prose}\n"),
            Some(refused),
        ));
        rows.push((
            "a model co-author, a line of U+00A0 and a letter, then prose".into(),
            format!("{model}\n\u{a0}x\n{prose}\n"),
            None,
        ));
        rows.push((
            "a model co-author, a line holding only U+30C0, then prose".into(),
            format!("{model}\n\u{30c0}\n{prose}\n"),
            None,
        ));
        for (name, line) in [
            ("a space", " "),
            ("a tab", "\t"),
            ("a carriage return", "\r"),
            ("a carriage return, a space and a carriage return", "\r \r"),
        ] {
            rows.push((
                format!("a model co-author, a line holding only {name}, then prose"),
                format!("{model}\n{line}\n{prose}\n"),
                Some(refused),
            ));
        }
        let probes: Vec<Probe> = rows
            .into_iter()
            .enumerate()
            .map(|(index, (label, body, refusal))| Probe {
                label,
                message: format!("probe {index}\n\n{body}"),
                refusal: refusal.map(str::to_owned),
            })
            .collect();
        assert_each_probe_judged(
            root,
            "A line git reads as text but another reader may call blank splits the paragraph for that reader, so the guard must refuse what either reading refuses, and only once; so",
            &probes,
        );
    });
}

#[test]
fn clears_a_look_alike_domain_when_the_caller_sets_lc_all() {
    check_workspace(|root| {
        let repository =
            repository_with_commit("docs: a change\n\nCo-authored-by: Probe <probe@te\u{17f}t>\n");
        let output = guard_command(root, repository.path())
            .env("LC_ALL", "C.UTF-8")
            .arg("HEAD")
            .output()
            .expect("the guard runs");
        assert!(
            output.status.success(),
            "the guard refused `probe@te\u{17f}t`, a look-alike of a listed domain, when the caller set LC_ALL=C.UTF-8. A guard that sets only LC_CTYPE or LANG leaves grep in the caller's LC_ALL, where GNU grep folds the long s onto `s`.\nguard said:\n{}",
            said(&output)
        );
    });
}

#[test]
fn prints_both_offences_when_a_line_git_reads_as_blank_parts_two_model_co_authors() {
    check_workspace(|root| {
        let harm = CO_AUTHOR_MODEL;
        std::thread::scope(|scope| {
            for (name, line) in [
                ("a space", " "),
                ("a tab", "\t"),
                ("a carriage return", "\r"),
                ("a carriage return, a space and a carriage return", "\r \r"),
            ] {
                scope.spawn(move || {
                    let repository = repository_with_commit(&format!(
                        "docs: a change\n\nCo-authored-by: Gemini <176961590+gemini-code-assist[bot]@users.noreply.github.com>\n{line}\nCo-authored-by: Copilot <198982749+Copilot@users.noreply.github.com>\n"
                    ));
                    let head = git(repository.path(), &["rev-parse", "HEAD"], None);
                    let sha = String::from_utf8_lossy(&head.stdout)[..9].to_owned();
                    let output = run_guard_in(root, repository.path(), "HEAD");
                    let said = said(&output);
                    let naming = format!("attribution-guard: {sha} ");
                    let lines: Vec<&str> = said.lines().filter(|text| text.starts_with(&naming)).collect();
                    let expected = [
                        format!("{naming}Co-authored-by \"Gemini <176961590+gemini-code-assist[bot]@users.noreply.github.com>\" {harm}"),
                        format!("{naming}Co-authored-by \"Copilot <198982749+Copilot@users.noreply.github.com>\" {harm}"),
                    ];
                    assert!(
                        !output.status.success() && lines == expected,
                        "a model co-author, a line holding only {name}, then a second model co-author must be refused by both lines, in this order: {expected:?}. Git reads the line as blank, so each co-author is a trailer of its own paragraph. A guard that keeps the two paragraphs as one hands git only the last, and the first offence stays hidden until the second is fixed.\nthe lines naming its commit are {lines:?}\nguard said:\n{said}"
                    );
                });
            }
        });
    });
}
