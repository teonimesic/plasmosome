use std::path::PathBuf;
use std::process::Command;

use plasmosome_guards::workspace_root;

const AGENT_SKILLS: &str = ".agents/skills";
const CLAUDE_SKILLS: &str = ".claude/skills";

fn skill_names() -> Vec<String> {
    let directory = workspace_root().join(AGENT_SKILLS);
    let entries = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("`{AGENT_SKILLS}` is readable: {error}"));
    let mut names = Vec::new();
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("an entry under `{AGENT_SKILLS}` is readable: {error}"))
            .path();
        if path.join("SKILL.md").is_file() {
            let name = path
                .file_name()
                .unwrap_or_else(|| panic!("`{}` has a file name", path.display()))
                .to_string_lossy()
                .into_owned();
            names.push(name);
        }
    }
    names.sort();
    assert!(
        !names.is_empty(),
        "no skill was found under `{AGENT_SKILLS}`; a check that inspects an empty list cannot fail, so the listing is what is broken, not the symlinks"
    );
    names
}

fn expected_target(name: &str) -> PathBuf {
    PathBuf::from("..").join("..").join(AGENT_SKILLS).join(name)
}

#[test]
fn every_skill_is_reachable_under_the_claude_code_directory() {
    for name in skill_names() {
        let link = workspace_root().join(CLAUDE_SKILLS).join(&name);
        let metadata = std::fs::symlink_metadata(&link).unwrap_or_else(|error| {
            panic!(
                "skill `{name}` has nothing at `{CLAUDE_SKILLS}/{name}`, so Claude Code does not list it: {error}"
            )
        });
        assert!(
            metadata.is_symlink(),
            "`{CLAUDE_SKILLS}/{name}` is a copy of skill `{name}` rather than a symlink to it; two copies of a skill disagree the moment one is edited"
        );
        let target = std::fs::read_link(&link).unwrap_or_else(|error| {
            panic!("`{CLAUDE_SKILLS}/{name}` is a readable symlink: {error}")
        });
        let expected = expected_target(&name);
        assert!(
            target == expected,
            "`{CLAUDE_SKILLS}/{name}` points at `{}` instead of `{}`; the target is relative to the repository so that a fresh clone anywhere on disk resolves it",
            target.display(),
            expected.display()
        );
        assert!(
            link.join("SKILL.md").is_file(),
            "`{CLAUDE_SKILLS}/{name}` does not resolve: `{CLAUDE_SKILLS}/{name}/SKILL.md` cannot be read through it"
        );
    }
}

#[test]
fn git_records_every_claude_code_skill_entry_as_a_symlink() {
    let output = Command::new("git")
        .current_dir(workspace_root())
        .args(["ls-files", "--stage", "--", CLAUDE_SKILLS])
        .output()
        .expect("git ls-files runs");
    assert!(
        output.status.success(),
        "git ls-files --stage {CLAUDE_SKILLS} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing = String::from_utf8_lossy(&output.stdout);
    for name in skill_names() {
        let path = format!("{CLAUDE_SKILLS}/{name}");
        let suffix = format!("\t{path}");
        let record = listing
            .lines()
            .find(|line| line.ends_with(&suffix))
            .unwrap_or_else(|| {
                panic!(
                    "skill `{name}` is not committed at `{path}`; an untracked symlink is absent from a fresh clone"
                )
            });
        let mode = record.split_whitespace().next().unwrap_or_else(|| {
            panic!("`git ls-files --stage` record for `{path}` starts with a mode: {record}")
        });
        assert!(
            mode == "120000",
            "git stores `{path}` with mode `{mode}`, not `120000`; a clone of this repository gets a plain file holding the link text instead of a link to skill `{name}`"
        );
    }
}
