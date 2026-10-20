use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use plasmosome_guards::check_workspace;
use tempfile::TempDir;

const HOOKS: [&str; 3] = ["pre-push", "attribution-guard", "provenance-guard"];

const OFFENCE: &str = "credits a model as an author";

const CLEARED: &str = "attribution-guard: clean";

const CREDITS_A_MODEL: &str =
    "docs: a change\n\nSome body prose.\n\nCo-authored-by: Claude <noreply@anthropic.com>\n";

const CREDITS_A_PERSON: &str = "docs: a change\n\nSome body prose.\n\nCo-authored-by: Probe Colleague <colleague@corp.locale.io>\n";

fn git(directory: &Path, arguments: &[&str]) -> Output {
    Command::new("git")
        .current_dir(directory)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Probe Person")
        .env("GIT_AUTHOR_EMAIL", "probe@corp.locale.io")
        .env("GIT_COMMITTER_NAME", "Probe Person")
        .env("GIT_COMMITTER_EMAIL", "probe@corp.locale.io")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_TEMPLATE_DIR")
        .args(arguments)
        .output()
        .expect("git runs")
}

fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn set_up(directory: &Path, arguments: &[&str]) -> String {
    let output = git(directory, arguments);
    assert!(
        output.status.success(),
        "setup: `git {}` failed in the scratch repository, so this test never reached the hook: {}",
        arguments.join(" "),
        said(&output)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn link_checkout_hooks(root: &Path, hooks: &Path) {
    fs::create_dir(hooks).expect("setup: the scratch hooks directory is created");
    for name in HOOKS {
        symlink(root.join(".githooks").join(name), hooks.join(name))
            .expect("setup: the checkout's hook is linked into the scratch repository");
    }
}

struct Scratch {
    _directory: TempDir,
    work: PathBuf,
    remote: String,
}

impl Scratch {
    fn new(root: &Path) -> Self {
        let directory = tempfile::tempdir().expect("setup: a scratch directory is created");
        let work = directory.path().join("work");
        let remote = directory
            .path()
            .join("remote.git")
            .to_str()
            .expect("setup: the scratch path is UTF-8")
            .to_owned();
        set_up(directory.path(), &["init", "--quiet", "--bare", &remote]);
        set_up(directory.path(), &["init", "--quiet", "work"]);
        link_checkout_hooks(root, &work.join(".githooks"));
        for arguments in [
            &["config", "core.hooksPath", ".githooks"][..],
            &["config", "commit.gpgsign", "false"],
            &["remote", "add", "origin", &remote],
            &["checkout", "--quiet", "-b", "main"],
        ] {
            set_up(&work, arguments);
        }
        Scratch {
            _directory: directory,
            work,
            remote,
        }
    }

    fn commit(&self, message: &str) -> String {
        set_up(
            &self.work,
            &["commit", "--quiet", "--allow-empty", "-m", message],
        );
        set_up(&self.work, &["rev-parse", "HEAD"])
    }

    fn branch_from_a_clean_root(&self, branch: &str, message: &str) -> String {
        self.commit("docs: a clean root");
        self.start(branch);
        self.commit(message)
    }

    fn start(&self, branch: &str) {
        set_up(&self.work, &["checkout", "--quiet", "-b", branch]);
    }

    fn check_out(&self, branch: &str) {
        set_up(&self.work, &["checkout", "--quiet", branch]);
    }

    fn fast_forward_to(&self, branch: &str) {
        set_up(&self.work, &["merge", "--quiet", "--ff-only", branch]);
    }

    fn push(&self, refspecs: &[&str]) -> Output {
        git(&self.work, &[&["push", "origin"][..], refspecs].concat())
    }

    fn push_past_the_hook(&self, refspecs: &[&str]) {
        set_up(
            &self.work,
            &[&["push", "--quiet", "--no-verify", "origin"][..], refspecs].concat(),
        );
    }

    fn remote_tip(&self, branch: &str) -> Option<String> {
        let output = git(
            &self.work,
            &[
                "--git-dir",
                &self.remote,
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        );
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
}

fn assert_refused(push: &Output, offender: &str, why: &str) {
    let said = said(push);
    assert!(
        !push.status.success(),
        "the pre-push hook let through a push that {why}; it exists to refuse exactly this before it leaves the machine.\nhook and git said:\n{said}"
    );
    let named = format!("{} {OFFENCE}", &offender[..9]);
    assert!(
        said.contains(&named),
        "the pre-push hook refused a push that {why}, but its output never names the offending commit with `{named}`; a hook that refuses for some other reason, or reads some other commit, would satisfy the exit status alone.\nhook and git said:\n{said}"
    );
}

fn assert_pushed(scratch: &Scratch, push: &Output, branch: &str, tip: &str, why: &str) {
    let said = said(push);
    assert!(
        push.status.success(),
        "the pre-push hook refused a push that {why}; a hook that refuses everything passes every refusal test, and stops people pushing.\nhook and git said:\n{said}"
    );
    assert!(
        said.contains(CLEARED),
        "the push that {why} went through, but the attribution guard never said it read the pushed commits; a hook that stops calling the guard pushes this too.\nhook and git said:\n{said}"
    );
    assert_eq!(
        scratch.remote_tip(branch).as_deref(),
        Some(tip),
        "the push that {why} reported success, but the remote's {branch} is not the pushed commit"
    );
}

#[test]
fn refuses_a_new_branch_whose_commit_credits_a_model() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        let credited = scratch.branch_from_a_clean_root("model", CREDITS_A_MODEL);

        let push = scratch.push(&["model"]);

        assert_refused(
            &push,
            &credited,
            "creates a branch whose tip credits a model",
        );
        assert_eq!(
            scratch.remote_tip("model"),
            None,
            "the hook refused the push, yet the remote now holds the branch that credits a model"
        );
    });
}

#[test]
fn refuses_a_new_branch_whose_model_commit_sits_under_a_clean_tip() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        let credited = scratch.branch_from_a_clean_root("model", CREDITS_A_MODEL);
        scratch.commit(CREDITS_A_PERSON);

        let push = scratch.push(&["model"]);

        assert_refused(
            &push,
            &credited,
            "creates a branch whose commit crediting a model sits under a clean tip, which a hook reading only the pushed commit clears",
        );
        assert_eq!(
            scratch.remote_tip("model"),
            None,
            "the hook refused the push, yet the remote now holds the branch that credits a model"
        );
    });
}

#[test]
fn refuses_a_branch_that_is_not_checked_out() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        let credited = scratch.branch_from_a_clean_root("model", CREDITS_A_MODEL);
        scratch.check_out("main");

        let push = scratch.push(&["model:refs/heads/model"]);

        assert_refused(
            &push,
            &credited,
            "creates a branch crediting a model while a clean branch is checked out, which a hook reading HEAD instead of the pushed commit would clear",
        );
        assert_eq!(
            scratch.remote_tip("model"),
            None,
            "the hook refused the push, yet the remote now holds the branch that credits a model"
        );
    });
}

#[test]
fn refuses_an_update_to_a_branch_that_is_not_checked_out() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        let published = scratch.branch_from_a_clean_root("topic", CREDITS_A_PERSON);
        scratch.push_past_the_hook(&["main", "topic"]);
        let credited = scratch.commit(CREDITS_A_MODEL);
        scratch.check_out("main");

        let push = scratch.push(&["topic"]);

        assert_refused(
            &push,
            &credited,
            "adds a commit crediting a model to a branch the remote has while a clean branch is checked out, which a hook reading <remote>..HEAD clears",
        );
        assert_eq!(
            scratch.remote_tip("topic").as_deref(),
            Some(published.as_str()),
            "the hook refused the update, yet the remote's topic moved off the commit it already held"
        );
    });
}

#[test]
fn pushes_the_clean_twin() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        let twin = scratch.branch_from_a_clean_root("person", CREDITS_A_PERSON);

        let push = scratch.push(&["person"]);

        assert_pushed(
            &scratch,
            &push,
            "person",
            &twin,
            "creates a branch whose tip credits a person, the twin of one crediting a model",
        );
    });
}

#[test]
fn refuses_an_update_that_adds_a_model_trailer() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        let published = scratch.commit("docs: a clean root");
        scratch.push_past_the_hook(&["main"]);
        let credited = scratch.commit(CREDITS_A_MODEL);

        let push = scratch.push(&["main"]);

        assert_refused(
            &push,
            &credited,
            "adds a commit crediting a model to a branch the remote already has",
        );
        assert_eq!(
            scratch.remote_tip("main").as_deref(),
            Some(published.as_str()),
            "the hook refused the update, yet the remote's main moved off the commit it already held"
        );
    });
}

#[test]
fn refuses_an_update_whose_model_commit_sits_under_a_clean_tip() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        let published = scratch.commit("docs: a clean root");
        scratch.push_past_the_hook(&["main"]);
        let credited = scratch.commit(CREDITS_A_MODEL);
        scratch.commit(CREDITS_A_PERSON);

        let push = scratch.push(&["main"]);

        assert_refused(
            &push,
            &credited,
            "adds a commit crediting a model under a clean tip to a branch the remote already has, which a hook or guard reading only the newest commit clears",
        );
        assert_eq!(
            scratch.remote_tip("main").as_deref(),
            Some(published.as_str()),
            "the hook refused the update, yet the remote's main moved off the commit it already held"
        );
    });
}

#[test]
fn reads_an_update_from_the_pushed_branch_not_every_remote_ref() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        let published = scratch.commit("docs: a clean root");
        scratch.start("side");
        let credited = scratch.commit(CREDITS_A_MODEL);
        scratch.push_past_the_hook(&["main", "side"]);
        scratch.check_out("main");
        scratch.fast_forward_to("side");
        scratch.commit(CREDITS_A_PERSON);

        let push = scratch.push(&["main"]);

        assert_refused(
            &push,
            &credited,
            "moves main onto a commit crediting a model that only another remote branch holds, which a hook reading an update as <local> --not --remotes clears",
        );
        assert_eq!(
            scratch.remote_tip("main").as_deref(),
            Some(published.as_str()),
            "the hook refused the update, yet the remote's main moved off the commit it already held"
        );
    });
}

#[test]
fn reads_only_what_an_update_adds() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        scratch.commit("docs: a clean root");
        let credited = scratch.commit(CREDITS_A_MODEL);
        scratch.push_past_the_hook(&["main"]);
        assert_eq!(
            scratch.remote_tip("main").as_deref(),
            Some(credited.as_str()),
            "setup: the remote's main does not hold the commit pushed past the hook"
        );
        let added = scratch.commit(CREDITS_A_PERSON);

        let push = scratch.push(&["main"]);

        assert_pushed(
            &scratch,
            &push,
            "main",
            &added,
            "adds only a commit crediting a person on top of one the remote already holds, which a hook reading the whole history instead of <remote>..<local> refuses",
        );
    });
}

#[test]
fn reads_only_what_a_new_branch_adds() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        scratch.commit("docs: a clean root");
        let credited = scratch.commit(CREDITS_A_MODEL);
        scratch.push_past_the_hook(&["main"]);
        assert_eq!(
            scratch.remote_tip("main").as_deref(),
            Some(credited.as_str()),
            "setup: the remote's main does not hold the commit pushed past the hook"
        );
        scratch.start("topic");
        let added = scratch.commit(CREDITS_A_PERSON);

        let push = scratch.push(&["topic"]);

        assert_pushed(
            &scratch,
            &push,
            "topic",
            &added,
            "creates a branch adding only a commit crediting a person on top of one the remote already holds, which a hook reading the new branch's whole history instead of <local> --not --remotes refuses",
        );
    });
}

#[test]
fn refuses_when_any_pushed_ref_credits_a_model() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        let credited = scratch.branch_from_a_clean_root("model", CREDITS_A_MODEL);

        let push = scratch.push(&["main", "model"]);

        assert_refused(
            &push,
            &credited,
            "creates a clean branch and, on the second ref, a branch crediting a model, which a hook reading only the first ref clears",
        );
        assert_eq!(
            scratch.remote_tip("model"),
            None,
            "the hook refused the push, yet the remote now holds the branch that credits a model"
        );
    });
}

#[test]
fn refuses_a_model_ref_between_a_delete_and_a_clean_ref() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        scratch.branch_from_a_clean_root("finished", CREDITS_A_PERSON);
        scratch.push_past_the_hook(&["finished"]);
        scratch.check_out("main");
        scratch.start("model");
        let credited = scratch.commit(CREDITS_A_MODEL);

        let push = scratch.push(&[":finished", "model", "main"]);

        assert_refused(
            &push,
            &credited,
            "deletes a branch, then creates one crediting a model, then a clean one, which a hook letting the last ref's verdict win, or stopping at a delete, clears",
        );
        assert_eq!(
            scratch.remote_tip("model"),
            None,
            "the hook refused the push, yet the remote now holds the branch that credits a model"
        );
        assert!(
            scratch.remote_tip("finished").is_some(),
            "the hook refused the push, yet the remote no longer holds the branch the push would have deleted"
        );
    });
}

#[test]
fn lets_a_branch_be_deleted() {
    check_workspace(|root| {
        let scratch = Scratch::new(root);
        scratch.branch_from_a_clean_root("finished", CREDITS_A_PERSON);
        scratch.push_past_the_hook(&["main", "finished"]);
        assert!(
            scratch.remote_tip("finished").is_some(),
            "setup: the remote does not hold the branch this test deletes"
        );

        let push = scratch.push(&[":finished"]);

        assert!(
            push.status.success(),
            "the pre-push hook refused a branch delete; a delete pushes no commit, and a hook that hands the guard the zero SHA of a delete makes branches impossible to remove.\nhook and git said:\n{}",
            said(&push)
        );
        assert_eq!(
            scratch.remote_tip("finished"),
            None,
            "the delete reported success, yet the remote still holds the branch"
        );
    });
}
