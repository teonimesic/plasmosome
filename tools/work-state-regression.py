#!/usr/bin/env python3
"""Exercise real pinned Beads routing and competing claims in disposable Git worktrees."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--bd", required=True, type=Path)
    options = parser.parse_args()
    source = Path(__file__).resolve().parent
    environment = {key: value for key, value in os.environ.items() if not key.startswith(("GIT_", "BD_", "BEADS_"))}

    with tempfile.TemporaryDirectory(prefix="work-state-regression-") as temporary:
        root = Path(temporary).resolve()
        repo, linked = root / "repo", root / "linked"
        repo.mkdir()
        home = root / "home"
        (home / ".beads").mkdir(parents=True)
        (home / ".beads/config.yaml").write_text("actor: shared-config-actor\n")
        environment.update(HOME=str(home), BD_ACTOR="shared-environment-actor")

        def git(*arguments, cwd=repo):
            return subprocess.run(["git", *arguments], cwd=cwd, env=environment, check=True, text=True, capture_output=True).stdout.strip()

        def native(*arguments, cwd=repo, actor="regression", success=True):
            result = subprocess.run(
                [str(cwd / "tools/work-state"), *arguments], cwd=cwd,
                env={**environment, "BEADS_ACTOR": actor}, text=True, capture_output=True,
            )
            assert (result.returncode == 0) == success, (arguments, result.returncode, result.stdout, result.stderr)
            return result

        git("init", "-q")
        (repo / "tools").mkdir()
        for name in ("work-state", "work-state-beads-1.1.2.toml"):
            shutil.copy2(source / name, repo / "tools" / name)
        git("add", "tools")
        git("-c", "user.name=Regression", "-c", "user.email=regression@example.test", "commit", "-qm", "fixture")
        git("worktree", "add", "-q", "-b", "linked", str(linked))
        before_head = git("rev-parse", "HEAD")
        before_config = (repo / ".git/config").read_bytes()
        hook = repo / ".git/hooks/pre-commit"
        hook.write_text("#!/bin/sh\nexit 97\n")
        hook.chmod(0o700)

        foreign = repo / ".beads"
        foreign.mkdir()
        foreign_metadata = '{"backend":"dolt","dolt_mode":"embedded","dolt_database":"wrong"}'
        (foreign / "metadata.json").write_text(foreign_metadata)
        (foreign / "embeddeddolt").mkdir()
        (linked / "body.txt").write_text("Shared body from the calling worktree")
        (linked / "metadata.json").write_text('{"spec_ids":["016"],"intent_ids":["015"]}')
        (linked / "evidence.txt").write_text("Exactly one competing actor claimed the task")
        before_status = {cwd: git("status", "--porcelain=v1", "--untracked-files=all", cwd=cwd) for cwd in (repo, linked)}
        native("install", "--archive", str(options.archive.resolve()), "--bd", str(options.bd.resolve()))
        native("init", "--help")
        native("init", "--not-a-native-flag", success=False)
        native("init", "--prefix", "regression")
        native("init", "--help", cwd=linked)
        native("list", "--json", cwd=linked)
        shared_metadata = repo / ".git/plasmosome-beads/store/metadata.json"
        valid_metadata = shared_metadata.read_bytes()
        shared_metadata.write_text('[{"dolt_mode":"embedded"}]')
        refused = native("list", "--json", success=False)
        assert refused.returncode == 2 and refused.stderr.startswith("work-state:"), (refused.returncode, refused.stderr)
        assert shared_metadata.read_text() == '[{"dolt_mode":"embedded"}]'
        shared_metadata.write_bytes(valid_metadata)
        created = json.loads(native("create", "Claim race", "--body-file=body.txt", "--metadata", "@metadata.json", "--labels", "planned", "--json", cwd=linked).stdout)
        issue = created["id"]
        next_issue = json.loads(native("create", "Next priority task", "--priority", "0", "--json").stdout)["id"]
        ordinary = json.loads(native("list", "--sort", "priority", "--json").stdout)
        reverse = json.loads(native("list", "--sort", "priority", "-r", "--json").stdout)
        assert [row["id"] for row in ordinary] == [row["id"] for row in reverse][::-1]
        shown = json.loads(native("show", issue, "--json").stdout)[0]
        assert shown["description"] == "Shared body from the calling worktree"
        assert shown["metadata"] == {"spec_ids": ["016"], "intent_ids": ["015"]}
        assert issue in {item["id"] for item in json.loads(native("ready", "--label", "planned", "--json").stdout)}
        native("update", issue, "--claim", actor="", success=False)
        alias = json.loads(native("create", "Hidden native aliases", "-mhello", "--json").stdout)
        assert alias["description"] == "hello"
        native("update", alias["id"], "--description-file=body.txt", cwd=linked)
        native("update", alias["id"], "--body", "--help", "--claim", actor="", success=False)
        untouched = json.loads(native("show", alias["id"], "--json").stdout)[0]
        assert (untouched["status"], untouched["description"]) == ("open", "Shared body from the calling worktree")
        native("update", alias["id"], "--body", "--help", "--claim", actor="alias-agent")
        consumed = json.loads(native("show", alias["id"], "--json").stdout)[0]
        assert (consumed["status"], consumed["assignee"], consumed["description"]) == ("in_progress", "alias-agent", "--help")
        native("close", alias["id"], "--message", "--help", "--claim-next", actor="", success=False)
        assert json.loads(native("show", alias["id"], "--json").stdout)[0]["status"] == "in_progress"
        exported = [json.loads(line) for line in native("export", "--no-memories").stdout.splitlines()]
        assert next(row for row in exported if row.get("id") == alias["id"])["description"] == "--help"

        competitors = []
        for cwd, actor in ((repo, "agent-one"), (linked, "agent-two")):
            competitors.append(subprocess.Popen(
                [str(cwd / "tools/work-state"), "update", issue, "--claim", "--json"],
                cwd=cwd, env={**environment, "BEADS_ACTOR": actor}, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            ))
        outcomes = [(process.communicate(), process.returncode) for process in competitors]
        assert sorted(code for output, code in outcomes) == [0, 1], outcomes
        winner = "agent-one" if competitors[0].returncode == 0 else "agent-two"
        claimed = json.loads(native("show", issue, "--json", cwd=linked).stdout)[0]
        assert (claimed["status"], claimed["assignee"]) == ("in_progress", winner)
        native("close", issue, "--claim-next", actor="", success=False)
        native("close", issue, "--continue", actor="", success=False)
        assert json.loads(native("show", issue, "--json").stdout)[0]["status"] == "in_progress"

        native("comments", "add", issue, "-fevidence.txt", cwd=linked)
        native("close", issue, "--reason-file", "evidence.txt", "--claim-next", cwd=linked, actor="closer")
        closed = json.loads(native("show", issue, "--json").stdout)[0]
        assert (closed["status"], closed["close_reason"]) == ("closed", "Exactly one competing actor claimed the task")
        next_claim = json.loads(native("show", next_issue, "--json").stdout)[0]
        assert (next_claim["status"], next_claim["assignee"]) == ("in_progress", "closer")
        environment.pop("BD_ACTOR")
        config_issue = json.loads(native("create", "Config identity precedence", "--json").stdout)["id"]
        native("update", config_issue, "--claim", actor="config-resistant-agent")
        assert json.loads(native("show", config_issue, "--json").stdout)[0]["assignee"] == "config-resistant-agent"
        parent = json.loads(native("create", "Continuation", "--type", "epic", "--json").stdout)["id"]
        first = json.loads(native("create", "First step", "--parent", parent, "--json").stdout)["id"]
        second = json.loads(native("create", "Second step", "--parent", parent, "--deps", first, "--json").stdout)["id"]
        native("update", first, "--claim", actor="continuing-agent")
        refused_continue = native("close", first, "--continue", actor="continuing-agent", success=False)
        assert refused_continue.returncode == 2 and "does not assign ownership" in refused_continue.stderr
        unchanged_first = json.loads(native("show", first, "--json").stdout)[0]
        unchanged_second = json.loads(native("show", second, "--json").stdout)[0]
        assert (unchanged_first["status"], unchanged_first["assignee"]) == ("in_progress", "continuing-agent")
        assert unchanged_second["status"] == "open" and not unchanged_second.get("assignee")
        native("close", first, "--continue", "--no-auto", actor="continuing-agent")
        suggested = json.loads(native("show", second, "--json").stdout)[0]
        assert suggested["status"] == "open" and not suggested.get("assignee")
        native("update", second, "--claim", actor="continuing-agent")
        continued = json.loads(native("show", second, "--json").stdout)[0]
        assert (continued["status"], continued["assignee"]) == ("in_progress", "continuing-agent")
        for verb, location, name in (
            ("init", "../backups/snapshot", "../backups/snapshot"),
            ("add", "../backups/alias-snapshot", "../backups/alias-snapshot"),
            ("init", (root / "backups/uri-snapshot").as_uri(), "../backups/uri-snapshot"),
            ("add", "~/home-snapshot", str(home / "home-snapshot")),
        ):
            native("backup", verb, location, cwd=linked)
            native("backup", "sync")
            assert (linked / name).is_dir()
            snapshot = json.loads(native("show", issue, "--json").stdout)[0]
            generation = json.loads(native("vc", "status", "--json").stdout)["commit"]
            native("update", issue, "--notes", "Mutation after the backup")
            native("backup", "restore", "--force", name, cwd=linked)
            assert json.loads(native("show", issue, "--json").stdout)[0] == snapshot
            assert json.loads(native("vc", "status", "--json").stdout)["commit"] == generation
        for flags in (("--db", str(foreign)), ("--directory=" + str(linked),), ("-C" + str(linked),), ("--global",)):
            native("list", *flags, success=False)
        native("init", "--force", success=False)
        native("init", success=False)
        assert git("rev-parse", "HEAD") == before_head
        for cwd, status in before_status.items():
            assert git("status", "--porcelain=v1", "--untracked-files=all", cwd=cwd) == status
        assert (repo / ".git/config").read_bytes() == before_config
        assert hook.read_text() == "#!/bin/sh\nexit 97\n"
        assert (foreign / "metadata.json").read_text() == foreign_metadata
        assert list((foreign / "embeddeddolt").iterdir()) == []
        assert not (linked / ".beads").exists()
        print("PASS: safe init/help/retry, foreign-store isolation, native option types and relative files, explicit identities on competing/next/continuation claims, unchanged Git checkout/hooks")


if __name__ == "__main__":
    main()
