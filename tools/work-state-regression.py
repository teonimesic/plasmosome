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

        def git(*arguments):
            return subprocess.run(["git", *arguments], cwd=repo, env=environment, check=True, text=True, capture_output=True).stdout.strip()

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
        native("install", "--archive", str(options.archive.resolve()), "--bd", str(options.bd.resolve()))
        native("init", "--help")
        native("init", "--not-a-native-flag", success=False)
        native("init", "--prefix", "regression")
        native("init", "--help", cwd=linked)
        native("list", "--json", cwd=linked)
        (linked / "body.txt").write_text("Shared body from the calling worktree")
        (linked / "metadata.json").write_text('{"spec_ids":["016"],"intent_ids":["015"]}')
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

        (linked / "evidence.txt").write_text("Exactly one competing actor claimed the task")
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
        native("close", first, "--continue", actor="continuing-agent")
        assert json.loads(native("show", second, "--json").stdout)[0]["status"] == "in_progress"
        for flags in (("--db", str(foreign)), ("--directory=" + str(linked),), ("-C" + str(linked),), ("--global",)):
            native("list", *flags, success=False)
        native("init", "--force", success=False)
        native("init", success=False)
        assert git("rev-parse", "HEAD") == before_head
        assert (repo / ".git/config").read_bytes() == before_config
        assert hook.read_text() == "#!/bin/sh\nexit 97\n"
        assert (foreign / "metadata.json").read_text() == foreign_metadata
        assert list((foreign / "embeddeddolt").iterdir()) == []
        assert not (linked / ".beads").exists()
        print("PASS: safe init/help/retry, foreign-store isolation, native option types and relative files, explicit identities on competing/next/continuation claims, unchanged Git checkout/hooks")


if __name__ == "__main__":
    main()
